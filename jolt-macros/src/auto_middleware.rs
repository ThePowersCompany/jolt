//! `#[derive(AutoMiddleware)]` proc-macro derive — phase10 field parsing.
//!
//! Phase10 ladder:
//! - JOLT-RS-046: parsed the struct's fields and their types into
//!   [`AutoMiddlewareInput`] + [`AutoMiddlewareField`]. The derive emits a
//!   minimal hidden marker so an integration test can verify the derive
//!   compiled and parsed without depending on later codegen.
//! - JOLT-RS-047: classify each parsed field with a [`FieldKind`]. The
//!   body-candidate rule fires when `field.ident == "body"` per the spec; the
//!   field's type is captured verbatim so the body-extraction codegen in 053
//!   can name `T` in `__req.json::<T>()`.
//! - JOLT-RS-048: mark query-extraction fields. Two rules, evaluated in this
//!   order: (a) any field whose type's last path segment is `QueryParams` is
//!   [`FieldKind::QueryParams`] regardless of name (covers `QueryParams<T>` and
//!   `crate::api::QueryParams<T>`); (b) a field NAMED `query_params` AND typed
//!   `HashMap<String, String>` (last path segment `HashMap` with two `String`
//!   generic args, covering bare `HashMap` and `std::collections::HashMap`
//!   variants) is also [`FieldKind::QueryParams`].
//! - JOLT-RS-049: mark request-injection fields. Type-based rule, regardless
//!   of name: any field whose type is `Request` or `&Request` (with or without
//!   an explicit lifetime) is [`FieldKind::Request`]. Mutable references
//!   (`&mut Request`) are NOT matched — middleware injection is the
//!   shared-reference shape per the spec, and excluding mut refs keeps the
//!   surface narrow. Path qualification on the inner type is allowed
//!   (`crate::Request`, `&::jolt_core::Request`) via last-path-segment
//!   matching. The Request rule lives between QueryParams and the body-name
//!   rule so a hypothetical `body: &Request` classifies as Request, pinning
//!   type-before-name precedence consistently with 048's QueryParams rule.
//! - JOLT-RS-050: detect the struct-level `#[cors]` attribute and stash it as
//!   [`AutoMiddlewareInput::cors`]. The derive opts the compiler into
//!   recognising `#[cors]` as a helper attribute via
//!   `#[proc_macro_derive(AutoMiddleware, attributes(cors))]` in `lib.rs`. The
//!   expansion emits a second hidden marker `__JOLT_AUTO_MIDDLEWARE_CORS: bool`
//!   so an integration test (and 051+'s layer codegen) can observe whether the
//!   CORS layer should be wired in.
//! - JOLT-RS-051 (this iteration): emit a real `::jolt_core::tower::Layer`
//!   impl on the user's struct via [`expand_layer_impl`]. The layer's
//!   `Service` is a generated wrapper struct
//!   `__JoltAutoMiddleware<Ident>Service<S>` that delegates `poll_ready` and
//!   `call` to the inner service for now — JOLT-RS-052/053 will splice the
//!   middleware-ordering chain and per-field extraction into the wrapper's
//!   `call()`. The 046 + 050 marker consts are kept alongside the new impl as
//!   parse-witnesses; they're cheap (`usize` and `bool`), already wired into
//!   the integration tests in `auto_middleware_derive.rs`, and trivially
//!   removable once 053+'s codegen has its own observable surface.
//!
//! The parse entry point is split out from `lib.rs` so it can be unit-tested
//! against a `proc_macro2::TokenStream` / parsed `syn::DeriveInput`
//! (proc-macro entry points themselves cannot be invoked outside the compiler).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Attribute, Data, DataStruct, DeriveInput, Fields, GenericArgument, Ident, PathArguments, Type,
};

/// Parsed shape of a `#[derive(AutoMiddleware)]` input.
///
/// JOLT-RS-046 captured the struct identifier and per-field metadata; 047-049
/// extended [`AutoMiddlewareField`] with kind classification (body, query,
/// req); 050 added struct-level attribute parsing for `#[cors]` via
/// [`AutoMiddlewareInput::cors`]. The struct ident is kept verbatim from the
/// source so codegen can emit `impl <ident>` blocks targeting the user's type.
#[derive(Debug)]
#[allow(dead_code)] // ident is consumed by expand_auto_middleware; the rest are read by tests this iteration. JOLT-RS-051+ wires layer codegen on top.
pub(crate) struct AutoMiddlewareInput {
    pub(crate) ident: Ident,
    pub(crate) fields: Vec<AutoMiddlewareField>,
    /// `true` iff the struct carries a bare `#[cors]` attribute. JOLT-RS-051+
    /// will use this flag to splice the CORS layer into the generated
    /// middleware call chain. The attribute is opted-in as a derive helper via
    /// `#[proc_macro_derive(AutoMiddleware, attributes(cors))]` in `lib.rs`;
    /// without that opt-in the compiler would reject `#[cors]` as an unknown
    /// macro at the user's source site before the derive ever runs.
    pub(crate) cors: bool,
}

/// One field on a `#[derive(AutoMiddleware)]` struct. Captured verbatim from
/// `syn::Field` — the ident lets later passes match on `body`, `query_params`,
/// `req` etc., and the type lets them inspect for `&Request`, `QueryParams<T>`,
/// or `HashMap<String, String>` shapes.
///
/// JOLT-RS-047 added [`FieldKind`] classification at parse time so later phase10
/// passes can iterate the parsed input once, dispatch on `kind`, and emit the
/// per-kind extraction code in 053. The `ty` stays verbatim because codegen
/// will splice it into `__req.json::<#ty>()` (Body) and similar shapes.
#[derive(Debug, Clone)]
#[allow(dead_code)] // fields are read by tests this iteration; JOLT-RS-051+ reads them in layer codegen.
pub(crate) struct AutoMiddlewareField {
    pub(crate) ident: Ident,
    pub(crate) ty: Type,
    pub(crate) kind: FieldKind,
}

/// Per-field classification used by the layer codegen in JOLT-RS-051+ to emit
/// the right extraction call per field.
///
/// Classification rules, evaluated top-to-bottom in [`classify_field`]:
/// 1. Type's last path segment is `QueryParams` → [`FieldKind::QueryParams`]
///    (regardless of field name; covers `QueryParams<T>` and path-qualified
///    forms like `crate::api::QueryParams<T>`).
/// 2. Type is `Request` or shared `&Request` (with or without lifetime) →
///    [`FieldKind::Request`] (regardless of field name; covers `Request`,
///    `&Request`, `&'a Request`, and path-qualified forms like `&crate::Request`).
///    Mutable refs (`&mut Request`) are NOT matched.
/// 3. Field NAMED `query_params` AND typed `HashMap<String, String>` (last
///    path segment is `HashMap` with two `String` generic args) →
///    [`FieldKind::QueryParams`].
/// 4. Field NAMED `body` → [`FieldKind::Body`].
/// 5. Otherwise → [`FieldKind::Other`].
///
/// Type-before-name precedence is intentional: a `body: QueryParams<T>` is
/// classified as QueryParams, and a `body: &Request` is Request — in both
/// cases the type names a framework extraction shape, so the name's framework
/// meaning is overridden. The body-name rule is otherwise the catch-all for
/// "this is a body, full stop" without needing to inspect T.
///
/// The `Other` catch-all is terminal: a field that isn't QueryParams, Request,
/// or body-named is one the user added that doesn't trigger any framework
/// extraction, and codegen treats it as `Default::default()` per-request (the
/// same default-construction contract used by `#[endpoint]` wrappers; see
/// JOLT-RS-043's progress notes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldKind {
    /// Body-candidate. The spec triggers JSON body parsing when a field named
    /// `body: T` is present; codegen in 053 will emit `__req.json::<T>()` and
    /// assign the result into the `body` field of the constructed middleware.
    Body,
    /// Query-params extraction. The spec triggers query-string parsing when
    /// the field is typed `QueryParams<T>` (any name, any path qualification)
    /// or named `query_params` and typed `HashMap<String, String>`; codegen in
    /// 053 will emit the appropriate `__req.query_params::<T>()` (for the
    /// typed shape) or a raw-map copy (for the HashMap shape).
    QueryParams,
    /// Request injection. The spec triggers per-request reference injection
    /// when a field is typed `Request` or `&Request` (any name, any path
    /// qualification, with or without lifetime); codegen in 053 will pass the
    /// active `&Request` (cloned by value if the user wrote bare `Request`)
    /// into the constructed middleware. Mutable refs (`&mut Request`) are not
    /// matched — middleware injection is the shared-reference shape.
    Request,
    /// Catch-all for fields with no framework-special meaning. After phase10
    /// closes, an `Other` field is one the user added that doesn't trigger any
    /// framework extraction — codegen treats it as `Default::default()`
    /// per-request.
    Other,
}

/// Parse a `DeriveInput` into [`AutoMiddlewareInput`].
///
/// Acceptance rules:
/// - Must be a `struct`. Enums and unions are rejected with a span pointing at
///   the offending keyword.
/// - Named-fields struct → captured field-by-field.
/// - Unit struct → accepted with an empty field list (a unit middleware is a
///   no-op extraction passthrough — the layer codegen in 051+ still emits the
///   tower::Layer impl, just with no body/query/req injection).
/// - Tuple struct → rejected. Field-kind detection in 047-049 keys on field
///   names (`body`, `query_params`, `req`); positional fields can't carry that
///   meaning, so accepting them would force a separate kind-detection rule
///   that doesn't compose with named-field structs.
pub(crate) fn parse_auto_middleware_input(
    input: DeriveInput,
) -> syn::Result<AutoMiddlewareInput> {
    let ident = input.ident.clone();
    let cors = parse_struct_attrs(&input.attrs);
    match input.data {
        Data::Struct(s) => {
            let fields = parse_struct_fields(&s, &ident)?;
            Ok(AutoMiddlewareInput {
                ident,
                fields,
                cors,
            })
        }
        Data::Enum(e) => Err(syn::Error::new_spanned(
            e.enum_token,
            "#[derive(AutoMiddleware)] can only be applied to structs, not enums",
        )),
        Data::Union(u) => Err(syn::Error::new_spanned(
            u.union_token,
            "#[derive(AutoMiddleware)] can only be applied to structs, not unions",
        )),
    }
}

/// Inspect the struct-level attributes for the `#[cors]` opt-in.
///
/// Matches a bare `#[cors]` (path attribute with no arguments). 050 doesn't
/// parse any sub-arguments — the CORS configuration shape lands later in
/// JOLT-RS-055+ (`CorsConfig { allow_origins, allow_methods, ... }`). For now
/// a presence-or-absence flag is enough for 051+'s layer codegen to decide
/// whether to wire in the CORS layer.
///
/// Other unrelated attributes (`#[derive(...)]` itself, `#[doc = "..."]`,
/// custom user attributes) are ignored. If a user writes `#[cors]` multiple
/// times the function still returns `true` — duplicates are a no-op rather
/// than an error, mirroring how rustc treats repeated zero-arg attributes.
fn parse_struct_attrs(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("cors"))
}

fn parse_struct_fields(
    data: &DataStruct,
    owner: &Ident,
) -> syn::Result<Vec<AutoMiddlewareField>> {
    match &data.fields {
        Fields::Named(named) => {
            let mut out = Vec::with_capacity(named.named.len());
            for field in &named.named {
                let field_ident = field
                    .ident
                    .clone()
                    .expect("Fields::Named guarantees every field has an ident");
                let kind = classify_field(&field_ident, &field.ty);
                out.push(AutoMiddlewareField {
                    ident: field_ident,
                    ty: field.ty.clone(),
                    kind,
                });
            }
            Ok(out)
        }
        Fields::Unit => Ok(Vec::new()),
        Fields::Unnamed(_) => Err(syn::Error::new_spanned(
            owner,
            "#[derive(AutoMiddleware)] requires named fields (tuple structs aren't supported; \
             field-kind detection in JOLT-RS-047+ keys on field names like `body`, `query_params`, `req`)",
        )),
    }
}

fn classify_field(ident: &Ident, ty: &Type) -> FieldKind {
    if type_path_ends_with(ty, "QueryParams") {
        return FieldKind::QueryParams;
    }
    if is_request_type(ty) {
        return FieldKind::Request;
    }
    if ident == "query_params" && is_hashmap_string_string(ty) {
        return FieldKind::QueryParams;
    }
    if ident == "body" {
        return FieldKind::Body;
    }
    FieldKind::Other
}

/// True iff `ty` is a path type whose last segment's ident equals `name`.
///
/// Used by 048 to spot `QueryParams<T>` (and `crate::api::QueryParams<T>`,
/// `::framework::QueryParams<T>`, etc.) without coupling to a specific module
/// path. 049 reuses this for the inner-type check inside [`is_request_type`].
fn type_path_ends_with(ty: &Type, name: &str) -> bool {
    let Type::Path(tp) = ty else {
        return false;
    };
    tp.path
        .segments
        .last()
        .is_some_and(|seg| seg.ident == name)
}

/// True iff `ty` is `Request` or a shared reference to `Request`.
///
/// Accepted shapes: bare `Request`, path-qualified `crate::Request`,
/// `&Request`, `&'a Request`, `&::jolt_core::Request`. Rejected shapes:
/// `&mut Request` (mutability disqualifies — middleware injection is the
/// shared-reference shape per the spec), `Option<Request>` (last path segment
/// is `Option`), `Vec<Request>`, etc.
///
/// Path qualification is matched on the LAST path segment via
/// [`type_path_ends_with`], so a user who imports `Request` under a different
/// crate path or a re-export still gets the Request kind without coupling to
/// jolt_core's specific module layout.
fn is_request_type(ty: &Type) -> bool {
    let inner = match ty {
        Type::Path(_) => ty,
        Type::Reference(r) if r.mutability.is_none() => r.elem.as_ref(),
        _ => return false,
    };
    type_path_ends_with(inner, "Request")
}

/// True iff `ty` is `HashMap<String, String>` — match on the last path segment
/// being `HashMap` with exactly two generic type arguments, both of which are
/// path types ending in `String`. Covers bare `HashMap` and `std::collections::HashMap`
/// (and any other path qualification). Doesn't match if the value type isn't
/// `String` (e.g. `HashMap<String, u32>` falls through to `Other`).
fn is_hashmap_string_string(ty: &Type) -> bool {
    let Type::Path(tp) = ty else {
        return false;
    };
    let Some(last) = tp.path.segments.last() else {
        return false;
    };
    if last.ident != "HashMap" {
        return false;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return false;
    };
    if args.args.len() != 2 {
        return false;
    }
    args.args.iter().all(|arg| match arg {
        GenericArgument::Type(t) => type_path_ends_with(t, "String"),
        _ => false,
    })
}

/// Top-level driver for `#[derive(AutoMiddleware)]`.
///
/// Parses via [`parse_auto_middleware_input`] and emits, in order:
///
/// 1. A hidden marker impl carrying `__JOLT_AUTO_MIDDLEWARE_FIELD_COUNT: usize`
///    (046) and `__JOLT_AUTO_MIDDLEWARE_CORS: bool` (050) so the integration
///    tests in `jolt-core/tests/auto_middleware_derive.rs` can witness that
///    parsing observed the right field count and cors flag.
/// 2. A `#[doc(hidden)]` wrapper service struct
///    `__JoltAutoMiddleware<Ident>Service<S>` (051) holding the inner service.
/// 3. An `impl<S> ::jolt_core::tower::Layer<S> for <Ident>` (051) that pulls
///    the wrapper service over the inner.
/// 4. An `impl<S, Req> ::jolt_core::tower::Service<Req> for <wrapper><S>` (051)
///    that delegates `poll_ready` and `call` to the inner service for now.
///    JOLT-RS-052/053 will splice the middleware-ordering chain (auth, cors,
///    parse-query, parse-body, ...) and the per-field extraction code into the
///    delegating `call()`.
///
/// On parse failure the emission is a single `compile_error!` token (with the
/// span the parser attached) — no marker impl, no partial codegen. This keeps
/// the diagnostic clean: the user sees one underlined error rather than a
/// cascade from later code that names the user's struct.
pub(crate) fn expand_auto_middleware(input: DeriveInput) -> TokenStream {
    let parsed = match parse_auto_middleware_input(input) {
        Ok(p) => p,
        Err(err) => return err.to_compile_error(),
    };
    let ident = &parsed.ident;
    let field_count = parsed.fields.len();
    let cors = parsed.cors;
    let layer = expand_layer_impl(&parsed);
    quote! {
        #[automatically_derived]
        impl #ident {
            #[doc(hidden)]
            pub const __JOLT_AUTO_MIDDLEWARE_FIELD_COUNT: usize = #field_count;
            #[doc(hidden)]
            pub const __JOLT_AUTO_MIDDLEWARE_CORS: bool = #cors;
        }

        #layer
    }
}

/// Build the helper-service ident for a given middleware struct.
///
/// Naming is `__JoltAutoMiddleware<UserIdent>Service` so that two derives in
/// the same scope can't collide (the user's ident is part of the wrapper's
/// name). The double-underscore prefix marks it as macro-internal — users
/// shouldn't reference it directly. The wrapper is `#[doc(hidden)]` for the
/// same reason.
fn service_ident_for(ident: &Ident) -> Ident {
    format_ident!("__JoltAutoMiddleware{}Service", ident)
}

/// Emit the `tower::Layer` + wrapper-service portion of the derive expansion
/// (JOLT-RS-051).
///
/// Shape of the emission:
///
/// ```ignore
/// #[doc(hidden)]
/// pub struct __JoltAutoMiddleware<Ident>Service<S> {
///     inner: S,
/// }
///
/// impl<S: ::core::clone::Clone> ::core::clone::Clone for ... { ... }
///
/// #[automatically_derived]
/// impl<__S> ::jolt_core::tower::Layer<__S> for <Ident> {
///     type Service = __JoltAutoMiddleware<Ident>Service<__S>;
///     fn layer(&self, inner: __S) -> Self::Service {
///         __JoltAutoMiddleware<Ident>Service { inner }
///     }
/// }
///
/// #[automatically_derived]
/// impl<__S, __Req> ::jolt_core::tower::Service<__Req>
///     for __JoltAutoMiddleware<Ident>Service<__S>
/// where
///     __S: ::jolt_core::tower::Service<__Req>,
/// {
///     type Response = <__S as ...::Service<__Req>>::Response;
///     type Error    = <__S as ...::Service<__Req>>::Error;
///     type Future   = <__S as ...::Service<__Req>>::Future;
///     fn poll_ready(&mut self, cx) -> Poll<Result<(), Self::Error>> { ... }
///     fn call(&mut self, req) -> Self::Future { ... }
/// }
/// ```
///
/// Decisions pinned at 051 (split out so 052/053's iterations can find them):
///
/// 1. **Wrapper service is a SIBLING free-standing struct**, not an inner
///    module or associated type. Free-standing items can carry their own
///    `impl` blocks with `where` clauses; an inner `mod` would force the
///    `Service` impl into the same module and add a path qualifier at every
///    call site. Naming via [`service_ident_for`] (`__JoltAutoMiddleware<Ident>Service`)
///    embeds the user's ident so two derives in the same scope can't collide.
/// 2. **Wrapper holds only `inner: __S`.** The user's middleware data
///    (extracted body / query / req) is per-request and constructed inside
///    `call()` via `Default::default()` in 053 — it does NOT live on the
///    wrapper. Keeping the wrapper minimal at 051 means 052/053 only need to
///    extend `call()`, not re-shape the wrapper's storage.
/// 3. **Generic over `__S` (the inner service) and `__Req` (the request
///    type).** The spec leans on `tower::Layer`'s usual signature where the
///    inner is a generic `S: Service<Request>`. Generic-over-`__Req` lets the
///    layer slot into either a `Service<axum::extract::Request>` stack
///    (production) or a `Service<()>` stack (tests / type-level assertions),
///    without forcing a specific request type at the proc-macro layer.
/// 4. **`call` and `poll_ready` delegate to the inner service.** No
///    extraction logic is emitted at 051 — the `call()` body is
///    `<__S as Service<__Req>>::call(&mut self.inner, __req)`. JOLT-RS-052
///    will wrap this in the auth/cors/parse chain; JOLT-RS-053 will splice
///    per-field `__req.json::<T>()` / `__req.query_params::<T>()` /
///    `&__req` extraction calls before the `inner.call(__req)`.
/// 5. **Wrapper derives `Clone` via a hand-written impl when `__S: Clone`.**
///    Tower stacks routinely clone services per-connection; a wrapper that
///    can't clone breaks composition. The hand-written impl avoids the
///    `#[derive(Clone)]`-needs-PhantomData-or-bounds-on-the-struct dance for
///    a generic with no field of that type. (Here `__S` IS a field type, so
///    `#[derive(Clone)]` would in theory work — but the hand-written impl is
///    explicit about the bound, lets us add `Sync`/`Send` bounds later
///    without re-deriving, and matches the rest of the proc-macro emission
///    style.)
/// 6. **The cors flag and per-field kinds are NOT consumed at 051.** They're
///    parsed and stored, but the `call()` body doesn't branch on them yet.
///    JOLT-RS-052 will read `parsed.cors` to decide whether to splice the
///    cors layer; JOLT-RS-053 will iterate `parsed.fields` to emit
///    extraction. The unused-field allow on `AutoMiddlewareInput` /
///    `AutoMiddlewareField` continues to cover this.
fn expand_layer_impl(parsed: &AutoMiddlewareInput) -> TokenStream {
    let ident = &parsed.ident;
    let service_ident = service_ident_for(ident);
    quote! {
        #[doc(hidden)]
        pub struct #service_ident<__S> {
            inner: __S,
        }

        #[automatically_derived]
        impl<__S: ::core::clone::Clone> ::core::clone::Clone for #service_ident<__S> {
            fn clone(&self) -> Self {
                Self {
                    inner: ::core::clone::Clone::clone(&self.inner),
                }
            }
        }

        #[automatically_derived]
        impl<__S> ::jolt_core::tower::Layer<__S> for #ident {
            type Service = #service_ident<__S>;

            fn layer(&self, inner: __S) -> Self::Service {
                #service_ident { inner }
            }
        }

        #[automatically_derived]
        impl<__S, __Req> ::jolt_core::tower::Service<__Req> for #service_ident<__S>
        where
            __S: ::jolt_core::tower::Service<__Req>,
        {
            type Response = <__S as ::jolt_core::tower::Service<__Req>>::Response;
            type Error = <__S as ::jolt_core::tower::Service<__Req>>::Error;
            type Future = <__S as ::jolt_core::tower::Service<__Req>>::Future;

            fn poll_ready(
                &mut self,
                __cx: &mut ::core::task::Context<'_>,
            ) -> ::core::task::Poll<::core::result::Result<(), Self::Error>> {
                <__S as ::jolt_core::tower::Service<__Req>>::poll_ready(&mut self.inner, __cx)
            }

            fn call(&mut self, __req: __Req) -> Self::Future {
                // JOLT-RS-052 will wrap this dispatch with the middleware
                // ordering chain (auth → cors → parse-query → parse-body →
                // user → handler). JOLT-RS-053 will splice per-field
                // extraction (`__req.json::<T>()`, `__req.query_params::<T>()`,
                // `&__req`) before the delegating call.
                <__S as ::jolt_core::tower::Service<__Req>>::call(&mut self.inner, __req)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use syn::parse2;

    fn parse_derive(src: &str) -> DeriveInput {
        let tokens = TokenStream::from_str(src).expect("test input parses as TokenStream");
        parse2::<DeriveInput>(tokens).expect("test input parses as DeriveInput")
    }

    #[test]
    fn parses_unit_struct_as_zero_fields() {
        // Unit struct → accepted with empty field list. The layer codegen in
        // 051+ will still emit the tower::Layer impl; with zero fields it just
        // performs no extraction.
        let input = parse_derive("struct Marker;");
        let parsed = parse_auto_middleware_input(input).expect("unit struct parses");
        assert_eq!(parsed.ident, "Marker");
        assert!(parsed.fields.is_empty(), "unit struct has zero fields");
    }

    #[test]
    fn parses_struct_with_single_named_field() {
        let input = parse_derive(
            r#"
            struct Auth {
                user_id: String,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("named struct parses");
        assert_eq!(parsed.ident, "Auth");
        assert_eq!(parsed.fields.len(), 1);
        assert_eq!(parsed.fields[0].ident, "user_id");
    }

    #[test]
    fn parses_struct_with_various_field_types() {
        // PRD-mandated shape: a struct with various field types must parse
        // through. Mixes body-candidate (CreateUserRequest), query-candidate
        // (QueryParams<Filters>), HashMap, request-ref (`&'a Request`), and
        // primitive — all five field types must land in the parsed list with
        // their idents and types preserved.
        let input = parse_derive(
            r#"
            struct Mixed<'a> {
                body: CreateUserRequest,
                query_params: QueryParams<Filters>,
                headers: std::collections::HashMap<String, String>,
                req: &'a Request,
                count: usize,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("various fields parse");
        assert_eq!(parsed.ident, "Mixed");
        assert_eq!(parsed.fields.len(), 5);
        let names: Vec<String> = parsed
            .fields
            .iter()
            .map(|f| f.ident.to_string())
            .collect();
        assert_eq!(
            names,
            vec!["body", "query_params", "headers", "req", "count"]
        );
    }

    #[test]
    fn rejects_enum() {
        let input = parse_derive("enum Bad { A, B }");
        let err = parse_auto_middleware_input(input)
            .expect_err("enum must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("structs"),
            "diagnostic must mention structs, got: {msg}"
        );
        assert!(
            msg.contains("enum"),
            "diagnostic must mention enum, got: {msg}"
        );
    }

    #[test]
    fn rejects_union() {
        let input = parse_derive(
            r#"
            union Bad {
                a: u32,
                b: f32,
            }
            "#,
        );
        let err = parse_auto_middleware_input(input)
            .expect_err("union must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("structs"),
            "diagnostic must mention structs, got: {msg}"
        );
        assert!(
            msg.contains("union"),
            "diagnostic must mention union, got: {msg}"
        );
    }

    #[test]
    fn rejects_tuple_struct() {
        // Tuple structs have positional fields; field-kind detection in 047-049
        // keys on names. Reject with a diagnostic that points at the struct ident
        // and explains the constraint.
        let input = parse_derive("struct TupleMw(String, u32);");
        let err = parse_auto_middleware_input(input)
            .expect_err("tuple struct must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("named fields"),
            "diagnostic must mention named fields, got: {msg}"
        );
    }

    #[test]
    fn expand_emits_const_with_field_count() {
        // End-to-end shape: the derive emission must contain the per-struct
        // hidden const reporting the parsed field count. The integration test
        // in jolt-core/tests/auto_middleware_derive.rs exercises the same
        // shape through cargo's compile pipeline.
        let input = parse_derive(
            r#"
            struct Three {
                a: String,
                b: u32,
                c: bool,
            }
            "#,
        );
        let rendered = expand_auto_middleware(input).to_string();
        assert!(
            rendered.contains("__JOLT_AUTO_MIDDLEWARE_FIELD_COUNT"),
            "expected hidden field-count const, rendered: {rendered}"
        );
        assert!(
            rendered.contains("impl Three"),
            "expected impl block targeting user's struct, rendered: {rendered}"
        );
        assert!(
            rendered.contains(": usize = 3"),
            "expected field count = 3, rendered: {rendered}"
        );
        assert!(
            rendered.contains("# [automatically_derived]"),
            "marker impl must be tagged #[automatically_derived], rendered: {rendered}"
        );
        assert!(
            !rendered.contains("compile_error"),
            "valid input must not surface a compile_error, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_emits_zero_count_for_unit_struct() {
        let input = parse_derive("struct Empty;");
        let rendered = expand_auto_middleware(input).to_string();
        assert!(
            rendered.contains(": usize = 0"),
            "unit struct field count must be 0, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_surfaces_compile_error_on_enum() {
        // An enum must produce a compile_error! token with no impl emission —
        // the user sees one targeted diagnostic rather than a cascade from
        // missing __JOLT_AUTO_MIDDLEWARE_FIELD_COUNT references.
        let input = parse_derive("enum Bad { A }");
        let rendered = expand_auto_middleware(input).to_string();
        assert!(
            rendered.contains("compile_error"),
            "enum must surface compile_error, rendered: {rendered}"
        );
        assert!(
            !rendered.contains("__JOLT_AUTO_MIDDLEWARE_FIELD_COUNT"),
            "must NOT emit marker impl when parse fails, rendered: {rendered}"
        );
    }

    #[test]
    fn parses_body_field_with_custom_type_as_body_kind() {
        // PRD-mandated verification: "field body: CreateUserRequest → detected
        // as body field." The detection rule is name-based (per the spec:
        // "auto-applies body parsing if `body: T` field exists"). The captured
        // type is preserved verbatim so the body-extraction codegen in 053 can
        // splice it into `__req.json::<T>()`.
        let input = parse_derive(
            r#"
            struct WithBody {
                body: CreateUserRequest,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("body-bearing struct parses");
        assert_eq!(parsed.fields.len(), 1);
        assert_eq!(parsed.fields[0].ident, "body");
        assert_eq!(parsed.fields[0].kind, FieldKind::Body);
        let ty = &parsed.fields[0].ty;
        let ty_tokens = quote::quote!(#ty).to_string();
        assert!(
            ty_tokens.contains("CreateUserRequest"),
            "body field type must round-trip for codegen in 053, got: {ty_tokens}"
        );
    }

    #[test]
    fn parses_body_field_with_primitive_type_as_body_kind() {
        // A `body: String` field is still a body-candidate. Detection is
        // name-based, not type-based — the spec says ANY `body: T` field
        // triggers body parsing. The codegen in 053 will emit
        // `__req.json::<String>()`, which is valid (serde_json deserializes
        // into String for a JSON string body).
        let input = parse_derive(
            r#"
            struct WithStringBody {
                body: String,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("body-bearing struct parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::Body);
    }

    #[test]
    fn parses_non_body_named_field_as_other_kind() {
        // Fields not named `body` fall through to `Other` for 047. 048/049 will
        // narrow `query_params` and `req` further, but until then a `count`
        // field stays `Other` — the layer codegen treats `Other` as
        // `Default::default()` per-request.
        let input = parse_derive(
            r#"
            struct WithCount {
                count: usize,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("count-bearing struct parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::Other);
    }

    #[test]
    fn classifies_body_field_only_in_mixed_struct() {
        // In a multi-field struct, only the `body` field is marked Body; every
        // other field is Other (until 048/049 narrow further). This pins the
        // ordering-independence of the classification — the body field can
        // appear anywhere in the field list and still be the only Body.
        let input = parse_derive(
            r#"
            struct Mixed {
                count: usize,
                body: CreateUserRequest,
                flag: bool,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("mixed struct parses");
        let kinds: Vec<FieldKind> = parsed.fields.iter().map(|f| f.kind).collect();
        assert_eq!(kinds, vec![FieldKind::Other, FieldKind::Body, FieldKind::Other]);
    }

    #[test]
    fn classifies_body_field_at_arbitrary_position() {
        // The `body` field can be the first OR last field; classification
        // depends only on the ident, not on the position.
        let first = parse_derive(
            r#"
            struct First {
                body: T,
                trailing: bool,
            }
            "#,
        );
        let last = parse_derive(
            r#"
            struct Last {
                leading: bool,
                body: T,
            }
            "#,
        );
        let first_parsed = parse_auto_middleware_input(first).expect("first-position parses");
        let last_parsed = parse_auto_middleware_input(last).expect("last-position parses");
        assert_eq!(
            first_parsed.fields[0].kind,
            FieldKind::Body,
            "body at position 0 is Body"
        );
        assert_eq!(
            first_parsed.fields[1].kind,
            FieldKind::Other,
            "trailing field is Other"
        );
        assert_eq!(
            last_parsed.fields[0].kind,
            FieldKind::Other,
            "leading field is Other"
        );
        assert_eq!(
            last_parsed.fields[1].kind,
            FieldKind::Body,
            "body at last position is Body"
        );
    }

    #[test]
    fn unit_struct_has_no_body_field() {
        // Unit structs have zero fields → there's nothing to classify, and the
        // body-candidate iterator should yield nothing. Pinned here because
        // 053's codegen will iterate `parsed.fields.iter().filter(|f| f.kind
        // == Body)` and a unit struct must produce zero extraction calls.
        let input = parse_derive("struct Marker;");
        let parsed = parse_auto_middleware_input(input).expect("unit struct parses");
        let body_count = parsed
            .fields
            .iter()
            .filter(|f| f.kind == FieldKind::Body)
            .count();
        assert_eq!(body_count, 0);
    }

    #[test]
    fn request_type_takes_precedence_over_body_name() {
        // Precedence pin: a `body: Request` is Request, NOT Body. The
        // type-based Request rule runs before the name-based body rule in
        // `classify_field`. Parallels 048's `query_params_type_takes_precedence_over_body_name`.
        // Replaces the earlier `body_field_classification_does_not_depend_on_type`
        // test (047 era), where the same input classified as Body before 049
        // added the type-based Request rule. The reversal is intentional:
        // calling a `Request`-typed field `body` is almost certainly a typo,
        // and the type-first rule produces correct injection rather than a
        // body-extraction call that would fail to compile downstream.
        let input = parse_derive(
            r#"
            struct WithRequestNamedBody {
                body: Request,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::Request);
    }

    #[test]
    fn expand_surfaces_compile_error_on_tuple_struct() {
        let input = parse_derive("struct TupleMw(String, u32);");
        let rendered = expand_auto_middleware(input).to_string();
        assert!(
            rendered.contains("compile_error"),
            "tuple struct must surface compile_error, rendered: {rendered}"
        );
        assert!(
            !rendered.contains("__JOLT_AUTO_MIDDLEWARE_FIELD_COUNT"),
            "must NOT emit marker impl when parse fails, rendered: {rendered}"
        );
    }

    #[test]
    fn parses_query_params_typed_field_as_query_params_kind() {
        // PRD-mandated verification: "field query: QueryParams<Filters> →
        // detected as query field." The detection rule is type-based for the
        // `QueryParams<T>` shape — the field's name is irrelevant when the
        // type matches, so a user writing `query: QueryParams<Filters>`
        // (instead of the conventional `query_params: ...`) still gets the
        // QueryParams kind.
        let input = parse_derive(
            r#"
            struct WithQuery {
                query: QueryParams<Filters>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields.len(), 1);
        assert_eq!(parsed.fields[0].kind, FieldKind::QueryParams);
        let ty = &parsed.fields[0].ty;
        let ty_tokens = quote::quote!(#ty).to_string();
        assert!(
            ty_tokens.contains("QueryParams") && ty_tokens.contains("Filters"),
            "type must round-trip verbatim for codegen in 053, got: {ty_tokens}"
        );
    }

    #[test]
    fn parses_path_qualified_query_params_as_query_params_kind() {
        // Type detection keys on the LAST path segment, not the full path —
        // so `crate::api::QueryParams<T>`, `::framework::QueryParams<T>`,
        // and bare `QueryParams<T>` all classify as QueryParams. Pinning this
        // because users in larger codebases will namespace the import.
        let input = parse_derive(
            r#"
            struct Mw {
                q: crate::api::QueryParams<Filters>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::QueryParams);
    }

    #[test]
    fn parses_named_query_params_with_hashmap_string_string_as_query_params_kind() {
        // Second detection rule: a field NAMED `query_params` AND typed
        // `HashMap<String, String>` is QueryParams. This is the raw-map shape
        // for users who want all query string entries without a typed schema.
        let input = parse_derive(
            r#"
            struct Mw {
                query_params: HashMap<String, String>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::QueryParams);
    }

    #[test]
    fn parses_named_query_params_with_path_qualified_hashmap_as_query_params_kind() {
        // The HashMap rule matches on the LAST path segment too —
        // `std::collections::HashMap<String, String>` and bare
        // `HashMap<String, String>` both classify as QueryParams when the
        // field is named `query_params`.
        let input = parse_derive(
            r#"
            struct Mw {
                query_params: std::collections::HashMap<String, String>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::QueryParams);
    }

    #[test]
    fn hashmap_string_string_with_non_query_params_name_classifies_as_other() {
        // The HashMap rule requires BOTH the right name AND the right type.
        // A `headers: HashMap<String, String>` field is NOT QueryParams — the
        // user clearly meant headers, not query params. Falls through to Other
        // (049 may narrow further if it adds a Headers variant; until then,
        // codegen treats it as `Default::default()`).
        let input = parse_derive(
            r#"
            struct Mw {
                headers: HashMap<String, String>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::Other);
    }

    #[test]
    fn named_query_params_with_non_string_value_hashmap_classifies_as_other() {
        // The HashMap rule requires the value type to be `String` too — a
        // `query_params: HashMap<String, u32>` doesn't match. The user
        // probably has a typed-deserialization use in mind and would be
        // better served by `QueryParams<T>`; falling through to Other
        // forces them to either rename the field or use the typed shape.
        let input = parse_derive(
            r#"
            struct Mw {
                query_params: HashMap<String, u32>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::Other);
    }

    #[test]
    fn query_params_type_takes_precedence_over_body_name() {
        // Precedence pin: a `body: QueryParams<T>` is QueryParams, NOT Body.
        // The type-based QueryParams rule runs before the name-based body
        // rule in `classify_field`. This is the right call because
        // `QueryParams<T>` is a framework type with extraction semantics —
        // calling it `body` doesn't change what extraction it needs.
        let input = parse_derive(
            r#"
            struct Mw {
                body: QueryParams<Filters>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::QueryParams);
    }

    #[test]
    fn named_query_params_with_non_hashmap_type_classifies_as_other() {
        // Edge case: a field named `query_params` but typed as something
        // OTHER than `HashMap<String, String>` or `QueryParams<T>` falls
        // through to Other. The framework's framework-meaning for
        // `query_params` is the raw-map shape; if the user wants a typed
        // shape they must use `QueryParams<T>` (rule 1 fires on the type).
        let input = parse_derive(
            r#"
            struct Mw {
                query_params: Vec<String>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::Other);
    }

    #[test]
    fn unit_struct_has_no_query_params_field() {
        // Parallel to `unit_struct_has_no_body_field`: the QueryParams-filter
        // iterator on a unit struct yields zero fields, so 053's codegen will
        // emit zero query-extraction calls. Pinned because the layer codegen
        // will use `parsed.fields.iter().filter(|f| f.kind == QueryParams)`.
        let input = parse_derive("struct Marker;");
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let q_count = parsed
            .fields
            .iter()
            .filter(|f| f.kind == FieldKind::QueryParams)
            .count();
        assert_eq!(q_count, 0);
    }

    #[test]
    fn classifies_all_four_kinds_in_mixed_struct() {
        // End-to-end shape: a struct that mixes body, query_params (typed),
        // headers (HashMap but wrong name), req (Request type), and a
        // primitive must produce the right kinds in field order. Updated
        // from 048's `classifies_all_three_kinds_in_mixed_struct` now that
        // 049's Request rule narrows the `req: Request` field.
        let input = parse_derive(
            r#"
            struct Mixed {
                body: CreateUserRequest,
                query_params: QueryParams<Filters>,
                headers: HashMap<String, String>,
                req: Request,
                count: usize,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let kinds: Vec<FieldKind> = parsed.fields.iter().map(|f| f.kind).collect();
        assert_eq!(
            kinds,
            vec![
                FieldKind::Body,
                FieldKind::QueryParams,
                FieldKind::Other, // headers: HashMap rule requires query_params name
                FieldKind::Request,
                FieldKind::Other,
            ]
        );
    }

    #[test]
    fn parses_request_reference_field_as_request_kind() {
        // PRD-mandated verification: "field req: &Request → detected as
        // request injection." The detection rule is type-based — the field's
        // name is irrelevant when the type matches, so `req: &Request` and
        // `whatever: &Request` both classify as Request. The captured type is
        // preserved verbatim so the layer codegen in 053 can splice it back
        // into the per-request injection call.
        let input = parse_derive(
            r#"
            struct WithReq {
                req: &Request,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields.len(), 1);
        assert_eq!(parsed.fields[0].ident, "req");
        assert_eq!(parsed.fields[0].kind, FieldKind::Request);
        let ty = &parsed.fields[0].ty;
        let ty_tokens = quote::quote!(#ty).to_string();
        assert!(
            ty_tokens.contains("Request"),
            "request field type must round-trip for codegen in 053, got: {ty_tokens}"
        );
    }

    #[test]
    fn parses_bare_request_type_as_request_kind() {
        // The PRD lists both `&Request` and `Request` as request-injection
        // shapes. Bare `Request` (by-value) classifies the same way; codegen
        // in 053 will handle the by-value vs by-reference shape distinction.
        let input = parse_derive(
            r#"
            struct WithReq {
                req: Request,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::Request);
    }

    #[test]
    fn parses_request_reference_with_lifetime_as_request_kind() {
        // `&'a Request` with an explicit lifetime is the most common shape in
        // user-facing middleware (the lifetime ties the borrow to the
        // request's lifetime). Still classifies as Request — the lifetime
        // annotation lives on the reference node, not the inner type, so
        // last-path-segment matching on the elem still hits `Request`.
        let input = parse_derive(
            r#"
            struct WithReq<'a> {
                req: &'a Request,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::Request);
    }

    #[test]
    fn parses_path_qualified_request_as_request_kind() {
        // Path qualification on the inner type — `&::jolt_core::Request`,
        // `crate::Request`, `&framework::request::Request`. The
        // last-path-segment rule via `type_path_ends_with` covers these
        // uniformly. Pinned because users in larger codebases will namespace
        // the import.
        let bare = parse_derive(
            r#"
            struct Mw {
                req: ::jolt_core::Request,
            }
            "#,
        );
        let by_ref = parse_derive(
            r#"
            struct Mw {
                req: &crate::Request,
            }
            "#,
        );
        let bare_parsed = parse_auto_middleware_input(bare).expect("parses");
        let by_ref_parsed = parse_auto_middleware_input(by_ref).expect("parses");
        assert_eq!(bare_parsed.fields[0].kind, FieldKind::Request);
        assert_eq!(by_ref_parsed.fields[0].kind, FieldKind::Request);
    }

    #[test]
    fn request_classification_does_not_depend_on_field_name() {
        // Field-name irrelevant when the type matches — parallel to the
        // QueryParams behavior pinned by 048. A field NAMED `whatever` typed
        // `&Request` still classifies as Request, ensuring the codegen in 053
        // picks up request injection regardless of the user's chosen name.
        let input = parse_derive(
            r#"
            struct Mw<'a> {
                whatever: &'a Request,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::Request);
    }

    #[test]
    fn mut_request_reference_classifies_as_other() {
        // `&mut Request` is intentionally NOT matched. Middleware injection
        // per the spec is shared-reference (`&Request`), and excluding mut
        // refs keeps the surface narrow. A user who writes `&mut Request`
        // falls through to Other; if they meant request injection, the codegen
        // in 053 would be unable to construct a mut ref out of the shared-ref
        // contract anyway. Pinning this so a future relaxation is explicit.
        let input = parse_derive(
            r#"
            struct Mw<'a> {
                req: &'a mut Request,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::Other);
    }

    #[test]
    fn option_request_classifies_as_other() {
        // `Option<Request>` has last path segment `Option`, not `Request` —
        // the wrapping changes the framework-meaning. The user wants
        // `None`-when-absent semantics, which doesn't map onto the
        // inject-the-active-request contract. Falls through to Other; codegen
        // treats it as `Default::default()`. Pinned because the existing
        // integration test in `auto_middleware_derive.rs` uses this shape.
        let input = parse_derive(
            r#"
            struct Mw {
                req: Option<Request>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::Other);
    }

    #[test]
    fn unit_struct_has_no_request_field() {
        // Parallel to `unit_struct_has_no_body_field` and
        // `unit_struct_has_no_query_params_field`: the Request-filter
        // iterator on a unit struct yields zero fields, so 053's codegen
        // emits zero injection calls. Pinned because the layer codegen will
        // use `parsed.fields.iter().filter(|f| f.kind == Request)`.
        let input = parse_derive("struct Marker;");
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let req_count = parsed
            .fields
            .iter()
            .filter(|f| f.kind == FieldKind::Request)
            .count();
        assert_eq!(req_count, 0);
    }

    #[test]
    fn request_reference_takes_precedence_over_body_name() {
        // Precedence pin: a `body: &Request` is Request, NOT Body. The
        // type-based Request rule runs before the name-based body rule in
        // `classify_field`. Pairs with `request_type_takes_precedence_over_body_name`
        // for the by-reference shape.
        let input = parse_derive(
            r#"
            struct Mw<'a> {
                body: &'a Request,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::Request);
    }

    #[test]
    fn parses_struct_with_cors_attribute_sets_cors_true() {
        // PRD-mandated verification: "#[cors] on struct → CORS flag set to
        // true in generated code." Detection is a bare path-attribute check
        // on the struct's attribute list; arguments (if any) are ignored for
        // 050 — JOLT-RS-055 introduces the CorsConfig shape.
        let input = parse_derive(
            r#"
            #[cors]
            struct CorsMw {
                body: CreateUserRequest,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert!(parsed.cors, "#[cors] struct attr must flip cors to true");
        assert_eq!(parsed.fields[0].kind, FieldKind::Body);
    }

    #[test]
    fn parses_struct_without_cors_attribute_leaves_cors_false() {
        // Bare struct (no struct-level attrs) → cors is false. Field-level
        // attributes don't count; the spec is explicit that `#[cors]` is a
        // struct-level opt-in.
        let input = parse_derive(
            r#"
            struct PlainMw {
                body: CreateUserRequest,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert!(!parsed.cors, "no #[cors] attr leaves cors false");
    }

    #[test]
    fn unrelated_struct_attributes_do_not_set_cors() {
        // The cors check matches only on `path().is_ident("cors")`. Other
        // attributes — `#[doc = "..."]`, `#[allow(...)]`, user-defined
        // attributes that happen to be on the struct — must not flip the flag.
        let input = parse_derive(
            r#"
            #[doc = "a middleware"]
            #[allow(dead_code)]
            struct Mw {
                body: T,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert!(!parsed.cors, "unrelated attrs must not flip cors");
    }

    #[test]
    fn duplicate_cors_attributes_are_a_no_op() {
        // Multiple `#[cors]` attrs on the same struct still mean cors=true.
        // 050 doesn't error on duplicates — mirrors how rustc treats repeated
        // zero-arg helper attributes. If a future PRD wants to reject
        // duplicates (e.g. once `#[cors(...)]` takes config), this test will
        // need to be updated alongside the new rule.
        let input = parse_derive(
            r#"
            #[cors]
            #[cors]
            struct Mw;
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert!(parsed.cors, "duplicate #[cors] still means cors=true");
    }

    #[test]
    fn cors_attribute_on_unit_struct() {
        // Unit struct + #[cors] is a valid combination: the user wants the
        // CORS layer wired in but has no body/query/req extraction. Pinned so
        // 051+'s layer codegen can treat this as a "CORS-only middleware"
        // shape.
        let input = parse_derive("#[cors] struct Marker;");
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert!(parsed.cors);
        assert!(parsed.fields.is_empty());
    }

    #[test]
    fn expand_emits_cors_const_true_when_attribute_present() {
        // End-to-end shape: the marker impl block carries
        // `__JOLT_AUTO_MIDDLEWARE_CORS: bool = true` when `#[cors]` is on the
        // struct. The integration test in `auto_middleware_derive.rs` will
        // assert this directly.
        let input = parse_derive(
            r#"
            #[cors]
            struct CorsMw {
                body: T,
            }
            "#,
        );
        let rendered = expand_auto_middleware(input).to_string();
        assert!(
            rendered.contains("__JOLT_AUTO_MIDDLEWARE_CORS"),
            "expected hidden cors-flag const, rendered: {rendered}"
        );
        assert!(
            rendered.contains(": bool = true"),
            "expected cors = true literal, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_emits_cors_const_false_when_attribute_absent() {
        // Parallel to the true case: bare struct → cors = false in the marker
        // impl. Both consts are always emitted so consumers don't need to
        // probe for the attribute's presence via cfg.
        let input = parse_derive(
            r#"
            struct PlainMw {
                body: T,
            }
            "#,
        );
        let rendered = expand_auto_middleware(input).to_string();
        assert!(
            rendered.contains("__JOLT_AUTO_MIDDLEWARE_CORS"),
            "expected hidden cors-flag const even when false, rendered: {rendered}"
        );
        assert!(
            rendered.contains(": bool = false"),
            "expected cors = false literal, rendered: {rendered}"
        );
    }

    // ----- JOLT-RS-051: expand_layer_impl -----
    //
    // The unit tests below cover the rendered token shape of
    // [`expand_layer_impl`] directly so we can pin individual emission decisions
    // (wrapper naming, `tower::Layer` impl shape, `tower::Service` delegation)
    // without going through the slower compile-and-run path. The integration
    // test in `jolt-core/tests/auto_middleware_derive.rs` covers the
    // compile-time witness that the trait actually IS implemented (a
    // `where T: tower::Layer<S>` bound that only resolves if the derive
    // produced a real impl).

    #[test]
    fn service_ident_for_embeds_user_struct_name() {
        // Wrapper-service naming embeds the user's ident so two derives in the
        // same scope can't collide. Pinned because 052/053 will reach into the
        // wrapper from the user's impl block; a renamed wrapper would silently
        // break the splice. The double-underscore prefix marks it macro-internal.
        let id: Ident = syn::parse_str("MyMw").expect("parses");
        let svc = service_ident_for(&id);
        assert_eq!(svc.to_string(), "__JoltAutoMiddlewareMyMwService");
    }

    #[test]
    fn expand_layer_impl_emits_wrapper_service_struct() {
        // The wrapper is a free-standing struct generic over the inner service
        // type `__S`. JOLT-RS-053 will not change the storage shape — only the
        // `call()` body — so pinning the storage here means a regression that
        // splices an extracted-fields struct here would surface immediately.
        let input = parse_derive("struct AuthMw;");
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        assert!(
            rendered.contains("# [doc (hidden)]"),
            "wrapper must be #[doc(hidden)], rendered: {rendered}"
        );
        assert!(
            rendered.contains("pub struct __JoltAutoMiddlewareAuthMwService < __S >"),
            "wrapper struct ident + generic param must match, rendered: {rendered}"
        );
        assert!(
            rendered.contains("inner : __S"),
            "wrapper must hold `inner: __S`, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_layer_impl_emits_layer_impl_for_user_struct() {
        // PRD verification: "Generated code compiles and implements
        // tower::Layer." Token-shape pin: the impl block must be
        // `impl<__S> ::jolt_core::tower::Layer<__S> for <UserIdent>` with a
        // `type Service = <wrapper>` and a `fn layer(&self, inner: __S)`. The
        // path is `::jolt_core::tower` (not bare `::tower`) because jolt-core
        // re-exports tower so user crates don't have to depend on it.
        let input = parse_derive("struct AuthMw;");
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        assert!(
            rendered.contains("impl < __S > :: jolt_core :: tower :: Layer < __S > for AuthMw"),
            "Layer impl header must match, rendered: {rendered}"
        );
        assert!(
            rendered.contains("type Service = __JoltAutoMiddlewareAuthMwService < __S >"),
            "Layer::Service must point at the generated wrapper, rendered: {rendered}"
        );
        assert!(
            rendered.contains("fn layer (& self , inner : __S) -> Self :: Service"),
            "Layer::layer signature must match, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_layer_impl_emits_service_impl_delegating_to_inner() {
        // The wrapper IS a tower::Service for any `Req` whose handling the
        // inner service implements. `call` and `poll_ready` delegate to
        // `self.inner` — JOLT-RS-052 will wrap the delegation in the
        // middleware-ordering chain; JOLT-RS-053 will splice per-field
        // extraction calls before it. Both still call through to inner, so
        // the `Service<__Req> for <wrapper>` impl shape stays load-bearing.
        let input = parse_derive("struct AuthMw;");
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        assert!(
            rendered.contains(
                "impl < __S , __Req > :: jolt_core :: tower :: Service < __Req > for __JoltAutoMiddlewareAuthMwService < __S >"
            ),
            "Service impl header must match, rendered: {rendered}"
        );
        assert!(
            rendered.contains("__S : :: jolt_core :: tower :: Service < __Req >"),
            "where-clause bound must match, rendered: {rendered}"
        );
        assert!(
            rendered.contains("fn call (& mut self , __req : __Req) -> Self :: Future"),
            "Service::call signature must match, rendered: {rendered}"
        );
        // `quote!`'s tokens print double-`>` without a space when one closes a
        // nested generic (`Service<__Req>>::call`). Match that exact shape.
        assert!(
            rendered.contains(":: jolt_core :: tower :: Service < __Req >> :: call (& mut self . inner , __req)"),
            "call() must delegate to inner.call, rendered: {rendered}"
        );
        assert!(
            rendered.contains(":: jolt_core :: tower :: Service < __Req >> :: poll_ready (& mut self . inner , __cx)"),
            "poll_ready must delegate to inner.poll_ready, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_layer_impl_emits_clone_when_inner_clone() {
        // Tower stacks routinely clone services per-connection. The hand-written
        // Clone impl bounds Clone on `__S: Clone` so a wrapper around a
        // non-Clone inner correctly fails to be Clone (matches axum's behavior
        // — services-without-Clone don't compose into multi-connection
        // servers). Pinned because removing this would silently break tower
        // composition for any caller that clones the layered stack.
        let input = parse_derive("struct AuthMw;");
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        assert!(
            rendered.contains(
                "impl < __S : :: core :: clone :: Clone > :: core :: clone :: Clone for __JoltAutoMiddlewareAuthMwService < __S >"
            ),
            "Clone impl header must match, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_auto_middleware_includes_both_marker_consts_and_layer() {
        // End-to-end: `expand_auto_middleware` splices the marker impl AND the
        // layer expansion into a single TokenStream. Both witnesses (consts
        // for parse-trace, layer for runtime behavior) coexist after 051 —
        // the consts can be removed after 053 has its own observable surface,
        // but until then the integration tests in
        // `auto_middleware_derive.rs` rely on both.
        let input = parse_derive(
            r#"
            #[cors]
            struct ChainedMw {
                body: T,
            }
            "#,
        );
        let rendered = expand_auto_middleware(input).to_string();
        assert!(
            rendered.contains("__JOLT_AUTO_MIDDLEWARE_FIELD_COUNT"),
            "field-count const must remain, rendered: {rendered}"
        );
        assert!(
            rendered.contains("__JOLT_AUTO_MIDDLEWARE_CORS"),
            "cors const must remain, rendered: {rendered}"
        );
        assert!(
            rendered.contains(":: jolt_core :: tower :: Layer < __S > for ChainedMw"),
            "Layer impl must be emitted alongside marker consts, rendered: {rendered}"
        );
        assert!(
            rendered.contains("__JoltAutoMiddlewareChainedMwService"),
            "wrapper service struct must be emitted, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_layer_impl_does_not_reference_user_struct_fields() {
        // Critical for 053's eventual codegen: the wrapper service holds ONLY
        // `inner`, never any of the user's struct fields. Per-request
        // middleware data lives on the `Default::default()`-constructed
        // user struct inside `call()`, not on the wrapper. A regression that
        // tried to thread fields through the wrapper would force every user
        // field type to be `Send + 'static`, defeating the by-reference
        // `&Request` field shape that 049 supports.
        let input = parse_derive(
            r#"
            struct WithBody {
                body: CreateUserRequest,
                count: usize,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        assert!(
            !rendered.contains("CreateUserRequest"),
            "wrapper must not reference user-field types, rendered: {rendered}"
        );
        assert!(
            !rendered.contains("body :"),
            "wrapper must not include user field idents, rendered: {rendered}"
        );
        assert!(
            !rendered.contains("count :"),
            "wrapper must not include user field idents, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_layer_impl_handles_unit_struct() {
        // Unit struct → wrapper still emitted, Layer + Service impls still
        // emitted, no fields to extract. JOLT-RS-053 will iterate
        // `parsed.fields` to splice extraction; on a unit struct that
        // iteration yields nothing, so the call() body stays a pure
        // delegation — no special-case handling required.
        let input = parse_derive("struct Marker;");
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        assert!(
            rendered.contains("__JoltAutoMiddlewareMarkerService"),
            "wrapper struct must be emitted for unit struct, rendered: {rendered}"
        );
        assert!(
            rendered.contains(":: jolt_core :: tower :: Layer < __S > for Marker"),
            "Layer impl must be emitted for unit struct, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_emits_layer_impl_even_when_cors_attribute_present() {
        // The cors flag does NOT change the layer-impl shape at 051 — the
        // wrapper service is identical for `#[cors]`-bearing and bare structs.
        // JOLT-RS-052 will read `parsed.cors` to splice the cors layer into
        // the call chain, but the outer Layer impl that 051 emits is
        // unconditional.
        let input = parse_derive(
            r#"
            #[cors]
            struct WithCors;
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        assert!(
            rendered.contains(":: jolt_core :: tower :: Layer < __S > for WithCors"),
            "Layer impl must be emitted for cors-attr struct too, rendered: {rendered}"
        );
    }
}
