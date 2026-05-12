//! `#[derive(AutoMiddleware)]` proc-macro derive — phase10 field parsing.
//!
//! Phase10 ladder:
//! - JOLTR-RS-046: parsed the struct's fields and their types into
//!   [`AutoMiddlewareInput`] + [`AutoMiddlewareField`]. The derive emits a
//!   minimal hidden marker so an integration test can verify the derive
//!   compiled and parsed without depending on later codegen.
//! - JOLTR-RS-047: classify each parsed field with a [`FieldKind`]. The
//!   body-candidate rule fires when `field.ident == "body"` per the spec; the
//!   field's type is captured verbatim so the body-extraction codegen in 053
//!   can name `T` in `__req.json::<T>()`.
//! - JOLTR-RS-048: mark query-extraction fields. Two rules, evaluated in this
//!   order: (a) any field whose type's last path segment is `QueryParams` is
//!   [`FieldKind::QueryParams`] regardless of name (covers `QueryParams<T>` and
//!   `crate::api::QueryParams<T>`); (b) a field NAMED `query_params` AND typed
//!   `HashMap<String, String>` (last path segment `HashMap` with two `String`
//!   generic args, covering bare `HashMap` and `std::collections::HashMap`
//!   variants) is also [`FieldKind::QueryParams`].
//! - JOLTR-RS-049: mark request-injection fields. Type-based rule, regardless
//!   of name: any field whose type is `Request` or `&Request` (with or without
//!   an explicit lifetime) is [`FieldKind::Request`]. Mutable references
//!   (`&mut Request`) are NOT matched — middleware injection is the
//!   shared-reference shape per the spec, and excluding mut refs keeps the
//!   surface narrow. Path qualification on the inner type is allowed
//!   (`crate::Request`, `&::joltr_core::Request`) via last-path-segment
//!   matching. The Request rule lives between QueryParams and the body-name
//!   rule so a hypothetical `body: &Request` classifies as Request, pinning
//!   type-before-name precedence consistently with 048's QueryParams rule.
//! - JOLTR-RS-050: detect the struct-level `#[cors]` attribute and stash it as
//!   [`AutoMiddlewareInput::cors`]. The derive opts the compiler into
//!   recognising `#[cors]` as a helper attribute via
//!   `#[proc_macro_derive(AutoMiddleware, attributes(cors))]` in `lib.rs`. The
//!   expansion emits a second hidden marker `__JOLTR_AUTO_MIDDLEWARE_CORS: bool`
//!   so an integration test (and 051+'s layer codegen) can observe whether the
//!   CORS layer should be wired in.
//! - JOLTR-RS-051: emit a real `::joltr_core::tower::Layer` impl on the user's
//!   struct via [`expand_layer_impl`]. The layer's `Service` is a generated
//!   wrapper struct `__JoltRAutoMiddleware<Ident>Service<S>` that delegates
//!   `poll_ready` and `call` to the inner service. The 046 + 050 marker
//!   consts are kept alongside the new impl as parse-witnesses; they're cheap
//!   (`usize` and `bool`), already wired into the integration tests in
//!   `auto_middleware_derive.rs`, and trivially removable once 053+'s codegen
//!   has its own observable surface.
//! - JOLTR-RS-052: emit the canonical middleware step ordering inside the
//!   wrapper's `call()` body via [`middleware_chain`] + [`MiddlewareStep`]. The
//!   chain's canonical order is
//!   `auth → cors → log → parse-query → parse-body → user → handler`; 052
//!   handles `cors` (per `parsed.cors`), `parse-query` (per any
//!   [`FieldKind::QueryParams`] field), and `parse-body` (per any
//!   [`FieldKind::Body`] field). Each present step renders as a stable
//!   string-literal marker statement (`let _: &str = "joltr::middleware::step::<name>";`)
//!   inside `call()`, in canonical order, BEFORE the existing delegating
//!   `<__S as Service<__Req>>::call(&mut self.inner, __req)`. The marker
//!   statements survive tokenisation (unlike `//` comments) so 053 has a
//!   stable splice point per step, and unit tests can pin the ordering by
//!   substring position. Auth, log, and user-defined steps are NOT emitted at
//!   052 — auth/log require attribute parsing landing in JOLTR-RS-056+, and
//!   user-defined middleware composition lands later still.
//! - JOLTR-RS-053 (this iteration): emit a per-derive
//!   `__jolt_extract_from(req: &::joltr_core::Request) -> Self` method on the
//!   user's middleware struct via [`expand_extraction`]. The method constructs
//!   `Self { ... }` with each field initialised by an expression matched to its
//!   [`FieldKind`]: Body via `req.json::<T>()`, HashMap-shaped QueryParams via
//!   `req.query_params.clone()`, by-value Request via `<Request as Clone>::clone(req)`,
//!   and Other via `<T as Default>::default()`. Typed `QueryParams<T>` originally
//!   emitted an `unimplemented!(...)` placeholder until typed parsing landed;
//!   by-ref `&Request` borrows the active request once the user's lifetime
//!   generics are threaded through the helper impl. The 052 chain markers in `call()` stay as marker
//!   statements — replacing them with calls into `__jolt_extract_from` would
//!   either break 051's generic-over-`__Req` design (the wrapper's `call` is
//!   generic over `__Req`, but extraction needs `&::joltr_core::Request`
//!   specifically) or require lifetime threading on Self construction. The
//!   extraction helper is the standalone observable surface JOLTR-RS-054 will
//!   exercise at runtime to verify per-field population.
//!
//! The parse entry point is split out from `lib.rs` so it can be unit-tested
//! against a `proc_macro2::TokenStream` / parsed `syn::DeriveInput`
//! (proc-macro entry points themselves cannot be invoked outside the compiler).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_quote, Attribute, Data, DataStruct, DeriveInput, Fields, GenericArgument, Generics,
    Ident, Lifetime, PathArguments, Type,
};

/// Parsed shape of a `#[derive(AutoMiddleware)]` input.
///
/// JOLTR-RS-046 captured the struct identifier and per-field metadata; 047-049
/// extended [`AutoMiddlewareField`] with kind classification (body, query,
/// req); 050 added struct-level attribute parsing for `#[cors]` via
/// [`AutoMiddlewareInput::cors`]; 052 reads both `cors` and `fields[].kind`
/// inside [`middleware_chain`] to decide which middleware-ordering steps to
/// splice into the generated `call()` body. The struct ident is kept verbatim
/// from the source so codegen can emit `impl <ident>` blocks targeting the
/// user's type.
#[derive(Debug)]
pub(crate) struct AutoMiddlewareInput {
    pub(crate) ident: Ident,
    pub(crate) generics: Generics,
    pub(crate) fields: Vec<AutoMiddlewareField>,
    /// `true` iff the struct carries a bare `#[cors]` attribute. 052 reads
    /// this in [`middleware_chain`] to splice the cors step into the
    /// generated call chain. The attribute is opted-in as a derive helper via
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
/// JOLTR-RS-047 added [`FieldKind`] classification at parse time so later phase10
/// passes can iterate the parsed input once, dispatch on `kind`, and emit the
/// per-kind extraction code in 053. The `ty` stays verbatim because codegen
/// will splice it into `__req.json::<#ty>()` (Body) and similar shapes.
///
/// As of JOLTR-RS-053, all three fields are consumed by codegen: [`kind`] is
/// read by [`middleware_chain`] (052) and [`expand_extraction`] (053) to
/// dispatch on the field's framework meaning; [`ident`] is spliced into the
/// `Self { #ident: ... }` literal that the extraction helper builds; [`ty`] is
/// spliced into `__req.json::<#ty>()` (Body) and `<#ty as Default>::default()`
/// (Other) so per-kind codegen names the right type.
#[derive(Debug, Clone)]
pub(crate) struct AutoMiddlewareField {
    pub(crate) ident: Ident,
    pub(crate) ty: Type,
    pub(crate) kind: FieldKind,
}

/// Per-field classification used by the layer codegen in JOLTR-RS-051+ to emit
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
/// JOLTR-RS-043's progress notes).
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
pub(crate) fn parse_auto_middleware_input(input: DeriveInput) -> syn::Result<AutoMiddlewareInput> {
    let ident = input.ident.clone();
    let generics = input.generics.clone();
    let cors = parse_struct_attrs(&input.attrs);
    match input.data {
        Data::Struct(s) => {
            let fields = parse_struct_fields(&s, &ident)?;
            Ok(AutoMiddlewareInput {
                ident,
                generics,
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
/// JOLTR-RS-055+ (`CorsConfig { allow_origins, allow_methods, ... }`). For now
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

fn parse_struct_fields(data: &DataStruct, owner: &Ident) -> syn::Result<Vec<AutoMiddlewareField>> {
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
             field-kind detection in JOLTR-RS-047+ keys on field names like `body`, `query_params`, `req`)",
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
    tp.path.segments.last().is_some_and(|seg| seg.ident == name)
}

/// True iff `ty` is `Request` or a shared reference to `Request`.
///
/// Accepted shapes: bare `Request`, path-qualified `crate::Request`,
/// `&Request`, `&'a Request`, `&::joltr_core::Request`. Rejected shapes:
/// `&mut Request` (mutability disqualifies — middleware injection is the
/// shared-reference shape per the spec), `Option<Request>` (last path segment
/// is `Option`), `Vec<Request>`, etc.
///
/// Path qualification is matched on the LAST path segment via
/// [`type_path_ends_with`], so a user who imports `Request` under a different
/// crate path or a re-export still gets the Request kind without coupling to
/// joltr_core's specific module layout.
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

fn query_params_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(tp) = ty else {
        return None;
    };
    let last = tp.path.segments.last()?;
    if last.ident != "QueryParams" {
        return None;
    }
    single_generic_inner(last)
}

fn single_generic_inner(segment: &syn::PathSegment) -> Option<&Type> {
    if let PathArguments::AngleBracketed(args) = &segment.arguments {
        if args.args.len() == 1 {
            if let GenericArgument::Type(inner) = &args.args[0] {
                return Some(inner);
            }
        }
    }
    None
}

/// One step in the canonical middleware call chain emitted into the wrapper
/// service's `call()` body (JOLTR-RS-052).
///
/// The PRD's chain order is `auth → cors → log → parse-query → parse-body →
/// user-defined middlewares → handler`. 052 emits only the steps that are
/// (a) present on the parsed input AND (b) implementable today; auth/log/user
/// require future PRD items (auth + log are likely 056+ attributes; user-defined
/// middleware composition is later still). The handler step is the existing
/// `<__S as Service<__Req>>::call(&mut self.inner, __req)` delegation, which
/// stays as the terminal expression of `call()` — 052's emission is purely
/// additive in front of it.
///
/// Variants are listed in canonical order; [`middleware_chain`] preserves
/// that order when assembling the per-derive chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MiddlewareStep {
    /// Fires iff `parsed.cors` (struct-level `#[cors]` attribute, 050). 053
    /// will replace the marker statement with the cors short-circuit + header
    /// injection logic that 056-058 will define on `CorsConfig`.
    Cors,
    /// Fires iff any field is [`FieldKind::QueryParams`] (048). 053 will
    /// replace the marker statement with the query-extraction calls — one
    /// `__req.query_params::<T>()` per `QueryParams<T>`-typed field, plus a
    /// raw-map copy for any `query_params: HashMap<String, String>`-typed
    /// field. Multiple QueryParams fields collapse into a single chain entry;
    /// 053 emits one extraction expression per matching field inside this
    /// single step.
    ParseQuery,
    /// Fires iff any field is [`FieldKind::Body`] (047). 053 will replace the
    /// marker statement with `__req.json::<T>()` for the (single) body field
    /// — the spec defines `body: T` as the body-extraction shape. Multiple
    /// `body`-named fields would already be a parse error from `Fields::Named`
    /// (Rust forbids duplicate field names), so a single chain entry is
    /// sufficient.
    ParseBody,
}

impl MiddlewareStep {
    /// Stable token-tag for the step. Embedded as a string literal in the
    /// generated `call()` body so unit tests can witness the per-derive chain
    /// shape and ordering by substring position. JOLTR-RS-053 will replace
    /// each marker statement with the real extraction body; until then the
    /// marker IS the observable surface of the chain.
    ///
    /// The `joltr::middleware::step::` prefix namespaces the marker so a
    /// substring search can't collide with unrelated string literals a user
    /// might happen to embed in a body that flows through this macro
    /// (extremely unlikely but cheap to defend).
    fn token_tag(self) -> &'static str {
        match self {
            Self::Cors => "joltr::middleware::step::cors",
            Self::ParseQuery => "joltr::middleware::step::parse_query",
            Self::ParseBody => "joltr::middleware::step::parse_body",
        }
    }
}

/// Build the canonical middleware step chain for `parsed`.
///
/// Steps are emitted in the canonical PRD order
/// (`auth → cors → log → parse-query → parse-body → user → handler`); 052
/// implements the cors / parse-query / parse-body subset, in that order.
///
/// Activation rules:
/// - [`MiddlewareStep::Cors`]: `parsed.cors == true`.
/// - [`MiddlewareStep::ParseQuery`]: any `parsed.fields[i].kind == QueryParams`.
/// - [`MiddlewareStep::ParseBody`]: any `parsed.fields[i].kind == Body`.
///
/// Each step appears at most once in the returned vector — multiple fields of
/// the same kind coalesce to a single chain entry, and 053's per-field
/// extraction codegen will fan out the field-level work inside the single
/// step. The chain order is independent of the field declaration order on
/// the source struct: a struct that lists `body` before `query_params` still
/// gets `[ParseQuery, ParseBody]` (canonical), not `[ParseBody, ParseQuery]`.
///
/// An empty chain (no `#[cors]` and no Body/QueryParams fields) returns an
/// empty `Vec` — the wrapper's `call()` body is then a pure delegation to
/// the inner service, identical to 051's pre-chain shape.
pub(crate) fn middleware_chain(parsed: &AutoMiddlewareInput) -> Vec<MiddlewareStep> {
    let mut chain = Vec::new();
    if parsed.cors {
        chain.push(MiddlewareStep::Cors);
    }
    if parsed
        .fields
        .iter()
        .any(|f| f.kind == FieldKind::QueryParams)
    {
        chain.push(MiddlewareStep::ParseQuery);
    }
    if parsed.fields.iter().any(|f| f.kind == FieldKind::Body) {
        chain.push(MiddlewareStep::ParseBody);
    }
    chain
}

/// Top-level driver for `#[derive(AutoMiddleware)]`.
///
/// Parses via [`parse_auto_middleware_input`] and emits, in order:
///
/// 1. A hidden marker impl carrying `__JOLTR_AUTO_MIDDLEWARE_FIELD_COUNT: usize`
///    (046) and `__JOLTR_AUTO_MIDDLEWARE_CORS: bool` (050) so the integration
///    tests in `joltr-core/tests/auto_middleware_derive.rs` can witness that
///    parsing observed the right field count and cors flag.
/// 2. A `#[doc(hidden)]` wrapper service struct
///    `__JoltRAutoMiddleware<Ident>Service<S>` (051) holding the inner service.
/// 3. An `impl<S> ::joltr_core::tower::Layer<S> for <Ident>` (051) that pulls
///    the wrapper service over the inner.
/// 4. An `impl<S, Req> ::joltr_core::tower::Service<Req> for <wrapper><S>` (051)
///    that delegates `poll_ready` and `call` to the inner service. As of 052,
///    `call()` splices in canonical-order step markers (cors, parse-query,
///    parse-body) for steps that fire on the parsed input. The terminal
///    `inner.call(__req)` delegation stays as the handler step.
/// 5. A per-derive `__jolt_extract_from(&::joltr_core::Request) -> Self` method
///    (053) on the user's struct, using the user's request lifetime for
///    by-ref `&Request` fields, that constructs `Self { ... }` with each
///    field initialised by an expression matched to its [`FieldKind`]. This is
///    the standalone observable surface JOLTR-RS-054 will exercise to verify
///    per-field extraction; later PRD items will splice the call into the
///    wrapper service's `call()` body once the wrapper-vs-Request-type design
///    tension is resolved (see [`expand_extraction`] for details).
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
    let (impl_generics, ty_generics, where_clause) = parsed.generics.split_for_impl();
    let field_count = parsed.fields.len();
    let cors = parsed.cors;
    let layer = expand_layer_impl(&parsed);
    let extraction = expand_extraction(&parsed);
    quote! {
        #[automatically_derived]
        impl #impl_generics #ident #ty_generics #where_clause {
            #[doc(hidden)]
            pub const __JOLTR_AUTO_MIDDLEWARE_FIELD_COUNT: usize = #field_count;
            #[doc(hidden)]
            pub const __JOLTR_AUTO_MIDDLEWARE_CORS: bool = #cors;
        }

        #layer

        #extraction
    }
}

/// Build the helper-service ident for a given middleware struct.
///
/// Naming is `__JoltRAutoMiddleware<UserIdent>Service` so that two derives in
/// the same scope can't collide (the user's ident is part of the wrapper's
/// name). The double-underscore prefix marks it as macro-internal — users
/// shouldn't reference it directly. The wrapper is `#[doc(hidden)]` for the
/// same reason.
fn service_ident_for(ident: &Ident) -> Ident {
    format_ident!("__JoltRAutoMiddleware{}Service", ident)
}

/// Emit the `tower::Layer` + wrapper-service portion of the derive expansion
/// (JOLTR-RS-051 + JOLTR-RS-052).
///
/// Shape of the emission:
///
/// ```ignore
/// #[doc(hidden)]
/// pub struct __JoltRAutoMiddleware<Ident>Service<S> {
///     inner: S,
/// }
///
/// impl<S: ::core::clone::Clone> ::core::clone::Clone for ... { ... }
///
/// #[automatically_derived]
/// impl<__S> ::joltr_core::tower::Layer<__S> for <Ident> {
///     type Service = __JoltRAutoMiddleware<Ident>Service<__S>;
///     fn layer(&self, inner: __S) -> Self::Service {
///         __JoltRAutoMiddleware<Ident>Service { inner }
///     }
/// }
///
/// #[automatically_derived]
/// impl<__S, __Req> ::joltr_core::tower::Service<__Req>
///     for __JoltRAutoMiddleware<Ident>Service<__S>
/// where
///     __S: ::joltr_core::tower::Service<__Req>,
/// {
///     type Response = <__S as ...::Service<__Req>>::Response;
///     type Error    = <__S as ...::Service<__Req>>::Error;
///     type Future   = <__S as ...::Service<__Req>>::Future;
///     fn poll_ready(&mut self, cx) -> Poll<Result<(), Self::Error>> { ... }
///     fn call(&mut self, req) -> Self::Future {
///         // 052: per-derive chain steps in canonical order, one stmt per active step.
///         let _: &::core::primitive::str = "joltr::middleware::step::cors";
///         let _: &::core::primitive::str = "joltr::middleware::step::parse_query";
///         let _: &::core::primitive::str = "joltr::middleware::step::parse_body";
///         <__S as ...::Service<__Req>>::call(&mut self.inner, __req)
///     }
/// }
/// ```
///
/// Decisions pinned at 051 (split out so 052/053's iterations can find them):
///
/// 1. **Wrapper service is a SIBLING free-standing struct**, not an inner
///    module or associated type. Free-standing items can carry their own
///    `impl` blocks with `where` clauses; an inner `mod` would force the
///    `Service` impl into the same module and add a path qualifier at every
///    call site. Naming via [`service_ident_for`] (`__JoltRAutoMiddleware<Ident>Service`)
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
/// 4. **`call` and `poll_ready` delegate to the inner service.** The
///    terminal expression of `call()` is
///    `<__S as Service<__Req>>::call(&mut self.inner, __req)`. JOLTR-RS-052's
///    chain markers are statement-level splices BEFORE that terminal
///    delegation; JOLTR-RS-053 will replace each marker with per-field
///    extraction (`__req.json::<T>()` / `__req.query_params::<T>()` /
///    `&__req`).
/// 5. **Wrapper derives `Clone` via a hand-written impl when `__S: Clone`.**
///    Tower stacks routinely clone services per-connection; a wrapper that
///    can't clone breaks composition. The hand-written impl avoids the
///    `#[derive(Clone)]`-needs-PhantomData-or-bounds-on-the-struct dance for
///    a generic with no field of that type. (Here `__S` IS a field type, so
///    `#[derive(Clone)]` would in theory work — but the hand-written impl is
///    explicit about the bound, lets us add `Sync`/`Send` bounds later
///    without re-deriving, and matches the rest of the proc-macro emission
///    style.)
///
/// Decisions pinned at 052:
///
/// 6. **Chain steps are statement-level string-literal markers, not function
///    calls.** Each active step renders as
///    `let _: &::core::primitive::str = "joltr::middleware::step::<name>";`.
///    The marker:
///      - is valid Rust regardless of `__Req`'s shape (no method calls on the
///        request, no trait bounds beyond `Service<__Req>`),
///      - survives tokenisation (unlike `//` comments, which `quote!` strips),
///      - doesn't trigger `unused_variables` (wildcard `_` pattern) or
///        `clippy::let_unit_value` (bound type is `&str`, not `()`),
///      - leaves the wrapper's `Self::Future` type unchanged (still the
///        inner service's `Future`; no `Box<dyn Future>` rewrap).
///
///    The marker is the splice point 053 targets — replacing the entire
///    `let _: ... = "...";` statement with the real step body keeps the
///    surrounding `call()` shape stable.
/// 7. **Multiple matching fields collapse to a single chain step.** The
///    `middleware_chain` builder pushes each step variant at most once even
///    if many fields match (e.g. two `QueryParams<T>` fields). 053 will
///    iterate `parsed.fields.iter().filter(|f| f.kind == ...)` inside the
///    single step to emit per-field extraction. Keeping coalescing here
///    means the chain ordering surface stays per-step (not per-field), which
///    matches the PRD's chain shape exactly.
/// 8. **Empty chain renders as the 051 pre-chain shape.** A derive with no
///    `#[cors]` and no Body/QueryParams fields produces an empty
///    `middleware_chain`, so `call()` is just the bare delegation — bit-for-bit
///    identical to 051's emission. That preserves the existing
///    integration-test contract for `UnitMiddleware`.
fn expand_layer_impl(parsed: &AutoMiddlewareInput) -> TokenStream {
    let ident = &parsed.ident;
    let (_, ty_generics, _) = parsed.generics.split_for_impl();
    let mut layer_generics = parsed.generics.clone();
    layer_generics.params.push(parse_quote!(__S));
    let (layer_impl_generics, _, layer_where_clause) = layer_generics.split_for_impl();
    let service_ident = service_ident_for(ident);
    let chain = middleware_chain(parsed);
    let chain_stmts: Vec<_> = chain
        .iter()
        .map(|step| {
            let tag = step.token_tag();
            quote! {
                let _: &::core::primitive::str = #tag;
            }
        })
        .collect();
    let service_impl = if parsed.fields.is_empty() {
        quote! {
            #[automatically_derived]
            impl<__S, __Req> ::joltr_core::tower::Service<__Req> for #service_ident<__S>
            where
                __S: ::joltr_core::tower::Service<__Req>,
            {
                type Response = <__S as ::joltr_core::tower::Service<__Req>>::Response;
                type Error = <__S as ::joltr_core::tower::Service<__Req>>::Error;
                type Future = <__S as ::joltr_core::tower::Service<__Req>>::Future;

                fn poll_ready(
                    &mut self,
                    __cx: &mut ::core::task::Context<'_>,
                ) -> ::core::task::Poll<::core::result::Result<(), Self::Error>> {
                    <__S as ::joltr_core::tower::Service<__Req>>::poll_ready(&mut self.inner, __cx)
                }

                fn call(&mut self, __req: __Req) -> Self::Future {
                    #(#chain_stmts)*
                    <__S as ::joltr_core::tower::Service<__Req>>::call(&mut self.inner, __req)
                }
            }
        }
    } else {
        quote! {
            #[automatically_derived]
            impl<__S, __Req> ::joltr_core::tower::Service<__Req> for #service_ident<__S>
            where
                __S: ::joltr_core::tower::Service<__Req>,
                __Req: ::core::any::Any,
                <__S as ::joltr_core::tower::Service<__Req>>::Future: 'static,
                <__S as ::joltr_core::tower::Service<__Req>>::Response:
                    ::core::convert::From<::joltr_core::parse_query::QueryErrorResponse> + 'static,
                <__S as ::joltr_core::tower::Service<__Req>>::Error: 'static,
            {
                type Response = <__S as ::joltr_core::tower::Service<__Req>>::Response;
                type Error = <__S as ::joltr_core::tower::Service<__Req>>::Error;
                type Future = ::core::pin::Pin<
                    ::std::boxed::Box<
                        dyn ::core::future::Future<
                            Output = ::core::result::Result<Self::Response, Self::Error>
                        >
                    >
                >;

                fn poll_ready(
                    &mut self,
                    __cx: &mut ::core::task::Context<'_>,
                ) -> ::core::task::Poll<::core::result::Result<(), Self::Error>> {
                    <__S as ::joltr_core::tower::Service<__Req>>::poll_ready(&mut self.inner, __cx)
                }

                fn call(&mut self, __req: __Req) -> Self::Future {
                    if let ::core::option::Option::Some(__jolt_response) =
                        ::joltr_core::request_ext::take_finished_response_for(&__req)
                    {
                        let __jolt_response = <Self::Response as ::core::convert::From<_>>::from(__jolt_response);
                        return ::std::boxed::Box::pin(async move {
                            ::core::result::Result::Ok(__jolt_response)
                        });
                    }

                    #(#chain_stmts)*
                    if let ::core::option::Option::Some(__jolt_req) =
                        (&__req as &dyn ::core::any::Any).downcast_ref::<::joltr_core::Request>()
                    {
                        if let ::core::result::Result::Err(__jolt_response) =
                            #ident::__jolt_try_extract_from(__jolt_req)
                        {
                            let __jolt_response = <Self::Response as ::core::convert::From<_>>::from(__jolt_response);
                            return ::std::boxed::Box::pin(async move {
                                ::core::result::Result::Ok(__jolt_response)
                            });
                        }
                    }
                    let __jolt_future = <__S as ::joltr_core::tower::Service<__Req>>::call(&mut self.inner, __req);
                    ::std::boxed::Box::pin(async move { __jolt_future.await })
                }
            }
        }
    };
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
        impl #layer_impl_generics ::joltr_core::tower::Layer<__S> for #ident #ty_generics #layer_where_clause {
            type Service = #service_ident<__S>;

            fn layer(&self, inner: __S) -> Self::Service {
                #service_ident { inner }
            }
        }

        #service_impl
    }
}

/// Emit the per-derive extraction helper (JOLTR-RS-053).
///
/// Renders, for the user's middleware struct `<Ident>`, an impl preserving the
/// user's generics with `pub fn __jolt_extract_from(__req: &Request) -> Self`.
/// When a by-ref request field is present, the helper parameter uses that field's
/// lifetime so the returned `Self` can borrow the request. The body constructs
/// `Self { #ident: <init>, ... }` with one init expression per parsed field,
/// matched to its [`FieldKind`]:
///
/// | FieldKind                            | init expression                                       |
/// | ------------------------------------ | ----------------------------------------------------- |
/// | `Body`                               | `__req.json::<#ty>().expect("…")`                     |
/// | `QueryParams` (HashMap shape)        | `::core::clone::Clone::clone(&__req.query_params)`    |
/// | `QueryParams` (typed `<T>` shape)    | `::core::unimplemented!("…JOLTR-RS-055+…")`            |
/// | `Request` (by-value bare `Request`)  | `<::joltr_core::Request as Clone>::clone(__req)`       |
/// | `Request` (by-ref `&Request`)        | `__req`                                                |
/// | `Other`                              | `<#ty as ::core::default::Default>::default()`        |
///
/// The helper is `#[doc(hidden)]` and prefixed with the macro-internal
/// double-underscore convention — users shouldn't reach for it directly. It IS
/// `pub` because the wrapper service in [`expand_layer_impl`] sits in a
/// sibling impl block and a future PRD will call into it from there; without
/// `pub` the wrapper couldn't name the method.
///
/// Decisions pinned at 053 (split out so future iterations can find them):
///
/// 1. **Helper method, not inline splice into `call()`.** The wrapper service's
///    `call()` is generic over `__Req` (per 051). Inlining `__req.json::<T>()`
///    would require `__Req: ::joltr_core::Request`, breaking 051's
///    `Service<()>` test surface. The standalone helper takes
///    `&::joltr_core::Request` directly, lets unit + integration tests call it
///    without going through the tower stack, and keeps the wrapper's generic
///    surface intact. Replacing the 052 chain markers with calls into the
///    helper is deferred to whichever PRD resolves the wrapper-vs-Request-type
///    design (likely after JOLTR-RS-055-058 lands a JoltR-aware tower layer).
/// 2. **`Self { #ident: <init>, ... }` literal, not `Self::default()` +
///    field assignments.** Three considered shapes: (a) `let mut __mw =
///    Self::default(); __mw.body = ...; __mw` — rejected because it requires
///    the user's struct to impl `Default`, which forces a `#[derive(Default)]`
///    that wouldn't fit a struct with a `req: Request` field (Request doesn't
///    impl Default). (b) Per-field assignment via builder — overkill for one
///    callsite. (c) Struct literal with per-field init exprs (current choice)
///    — naming each field explicitly compose-checks against the struct's
///    declared fields at the macro expansion site, and only requires Default
///    on each `Other`-kinded field's type (matching the spec for Other).
/// 3. **`Body` extraction uses `.expect(...)` not `.unwrap_or_default()`.**
///    The latter would require `T: Default` on the body type; the former
///    surfaces a clear panic if the body fails to deserialize. JOLTR-RS-062
///    will replace this with proper Result-based error handling once the
///    framework's typed-error surface lands; for now `.expect` matches the
///    spec's "extract body into struct" verbiage without an extra Default
///    bound on user types.
/// 4. **By-ref `&Request` borrows the helper argument.** The generated helper
///    impl preserves the user's generics and, when a request-reference field is
///    present, uses that lifetime on the `__req` parameter so `Self` can hold
///    the active request borrow. Typed `QueryParams<T>` is handled above through
///    the query deserializer.
/// 5. **`Other` fields use `<#ty as Default>::default()`.** The spec says the
///    Other catch-all is `Default::default()` per-request. Each Other-kinded
///    field's type must therefore impl `Default`; the struct literal makes
///    this a compile-time error pinned to the macro expansion site
///    (clearer than a deferred runtime panic).
/// 6. **Unit struct emits `Self {}` (empty braces).** Rust accepts empty
///    struct-literal braces for both unit structs (`struct Marker;`) and
///    named-empty structs (`struct Empty {}`); using the same shape for both
///    keeps the codegen branch-free. Pinned by
///    `expand_extraction_handles_unit_struct`.
fn expand_extraction(parsed: &AutoMiddlewareInput) -> TokenStream {
    let ident = &parsed.ident;
    let (impl_generics, ty_generics, where_clause) = parsed.generics.split_for_impl();
    let request_ref_ty = helper_request_ref_ty(parsed);
    let field_inits = parsed.fields.iter().map(|f| {
        let f_ident = &f.ident;
        let init_expr = field_init_expr(f);
        quote! { #f_ident: #init_expr }
    });
    quote! {
        #[automatically_derived]
        impl #impl_generics #ident #ty_generics #where_clause {
            /// JOLTR-RS-053: per-derive extraction helper. Constructs an
            /// instance of this middleware struct from a
            /// `&::joltr_core::Request` by running per-field extraction (body,
            /// query params, request injection) per the rules in `FieldKind`.
            ///
            /// `#[doc(hidden)]` because it's macro-internal. JOLTR-RS-054 will
            /// exercise this method at runtime to verify per-field population.
            #[doc(hidden)]
            pub fn __jolt_extract_from(__req: #request_ref_ty) -> Self {
                match Self::__jolt_try_extract_from(__req) {
                    ::core::result::Result::Ok(__jolt_middleware) => __jolt_middleware,
                    ::core::result::Result::Err(_) => {
                        ::core::panic!("JoltR auto-middleware: query parameter extraction failed")
                    }
                }
            }

            #[doc(hidden)]
            pub fn __jolt_try_extract_from(
                __req: #request_ref_ty,
            ) -> ::core::result::Result<Self, ::joltr_core::parse_query::QueryErrorResponse> {
                ::core::result::Result::Ok(Self {
                    #(#field_inits),*
                })
            }
        }
    }
}

fn helper_request_ref_ty(parsed: &AutoMiddlewareInput) -> TokenStream {
    if let Some(lifetime) = by_ref_request_lifetime(parsed) {
        quote! { &#lifetime ::joltr_core::Request }
    } else {
        quote! { &::joltr_core::Request }
    }
}

fn by_ref_request_lifetime(parsed: &AutoMiddlewareInput) -> Option<Lifetime> {
    parsed.fields.iter().find_map(|field| {
        if field.kind != FieldKind::Request {
            return None;
        }
        let Type::Reference(reference) = &field.ty else {
            return None;
        };
        if reference.mutability.is_some() || !type_path_ends_with(&reference.elem, "Request") {
            return None;
        }
        reference.lifetime.clone().or_else(|| {
            parsed.generics.params.iter().find_map(|param| match param {
                syn::GenericParam::Lifetime(lifetime) => Some(lifetime.lifetime.clone()),
                _ => None,
            })
        })
    })
}

fn typed_query_params_init_expr(field_ty: &Type, inner_ty: &Type) -> TokenStream {
    quote! {
        {
            let __jolt_query = ::joltr_core::parse_query::deserialize_query::<#inner_ty>(&__req.query_params)
                .map_err(|__jolt_err| ::joltr_core::bad_request_for_query_error(&__jolt_err))?;
            <#field_ty as ::core::convert::From<_>>::from(__jolt_query)
        }
    }
}

fn raw_query_params_init_expr(field_ty: &Type) -> TokenStream {
    if is_hashmap_string_string(field_ty) {
        quote! { ::core::clone::Clone::clone(&__req.query_params) }
    } else {
        quote! {
            <#field_ty as ::core::convert::From<_>>::from(
                ::core::clone::Clone::clone(&__req.query_params)
            )
        }
    }
}

/// Choose the per-field extraction expression for `field`. Split out from
/// [`expand_extraction`] so unit tests can exercise the dispatch directly and
/// so future PRD items have a single place to update (the table above mirrors
/// the match arms here).
fn field_init_expr(field: &AutoMiddlewareField) -> TokenStream {
    let f_ty = &field.ty;
    match field.kind {
        FieldKind::Body => quote! {
            __req
                .json::<#f_ty>()
                .expect("JoltR auto-middleware: body deserialization failed (JOLTR-RS-062 will replace this panic with typed Result handling)")
        },
        FieldKind::QueryParams => {
            if let Some(inner) = query_params_inner_type(f_ty) {
                typed_query_params_init_expr(f_ty, inner)
            } else {
                raw_query_params_init_expr(f_ty)
            }
        }
        FieldKind::Request => {
            if matches!(f_ty, Type::Reference(_)) {
                quote! { __req }
            } else {
                quote! { <::joltr_core::Request as ::core::clone::Clone>::clone(__req) }
            }
        }
        FieldKind::Other => quote! {
            <#f_ty as ::core::default::Default>::default()
        },
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
        let names: Vec<String> = parsed.fields.iter().map(|f| f.ident.to_string()).collect();
        assert_eq!(
            names,
            vec!["body", "query_params", "headers", "req", "count"]
        );
    }

    #[test]
    fn rejects_enum() {
        let input = parse_derive("enum Bad { A, B }");
        let err = parse_auto_middleware_input(input).expect_err("enum must be rejected");
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
        let err = parse_auto_middleware_input(input).expect_err("union must be rejected");
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
        let err = parse_auto_middleware_input(input).expect_err("tuple struct must be rejected");
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
        // in joltr-core/tests/auto_middleware_derive.rs exercises the same
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
            rendered.contains("__JOLTR_AUTO_MIDDLEWARE_FIELD_COUNT"),
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
        // missing __JOLTR_AUTO_MIDDLEWARE_FIELD_COUNT references.
        let input = parse_derive("enum Bad { A }");
        let rendered = expand_auto_middleware(input).to_string();
        assert!(
            rendered.contains("compile_error"),
            "enum must surface compile_error, rendered: {rendered}"
        );
        assert!(
            !rendered.contains("__JOLTR_AUTO_MIDDLEWARE_FIELD_COUNT"),
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
        assert_eq!(
            kinds,
            vec![FieldKind::Other, FieldKind::Body, FieldKind::Other]
        );
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
            !rendered.contains("__JOLTR_AUTO_MIDDLEWARE_FIELD_COUNT"),
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
        // Path qualification on the inner type — `&::joltr_core::Request`,
        // `crate::Request`, `&framework::request::Request`. The
        // last-path-segment rule via `type_path_ends_with` covers these
        // uniformly. Pinned because users in larger codebases will namespace
        // the import.
        let bare = parse_derive(
            r#"
            struct Mw {
                req: ::joltr_core::Request,
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
        // 050 — JOLTR-RS-055 introduces the CorsConfig shape.
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
        // `__JOLTR_AUTO_MIDDLEWARE_CORS: bool = true` when `#[cors]` is on the
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
            rendered.contains("__JOLTR_AUTO_MIDDLEWARE_CORS"),
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
            rendered.contains("__JOLTR_AUTO_MIDDLEWARE_CORS"),
            "expected hidden cors-flag const even when false, rendered: {rendered}"
        );
        assert!(
            rendered.contains(": bool = false"),
            "expected cors = false literal, rendered: {rendered}"
        );
    }

    // ----- JOLTR-RS-051: expand_layer_impl -----
    //
    // The unit tests below cover the rendered token shape of
    // [`expand_layer_impl`] directly so we can pin individual emission decisions
    // (wrapper naming, `tower::Layer` impl shape, `tower::Service` delegation)
    // without going through the slower compile-and-run path. The integration
    // test in `joltr-core/tests/auto_middleware_derive.rs` covers the
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
        assert_eq!(svc.to_string(), "__JoltRAutoMiddlewareMyMwService");
    }

    #[test]
    fn expand_layer_impl_emits_wrapper_service_struct() {
        // The wrapper is a free-standing struct generic over the inner service
        // type `__S`. JOLTR-RS-053 will not change the storage shape — only the
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
            rendered.contains("pub struct __JoltRAutoMiddlewareAuthMwService < __S >"),
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
        // `impl<__S> ::joltr_core::tower::Layer<__S> for <UserIdent>` with a
        // `type Service = <wrapper>` and a `fn layer(&self, inner: __S)`. The
        // path is `::joltr_core::tower` (not bare `::tower`) because joltr-core
        // re-exports tower so user crates don't have to depend on it.
        let input = parse_derive("struct AuthMw;");
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        assert!(
            rendered.contains("impl < __S > :: joltr_core :: tower :: Layer < __S > for AuthMw"),
            "Layer impl header must match, rendered: {rendered}"
        );
        assert!(
            rendered.contains("type Service = __JoltRAutoMiddlewareAuthMwService < __S >"),
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
        // `self.inner` — JOLTR-RS-052 will wrap the delegation in the
        // middleware-ordering chain; JOLTR-RS-053 will splice per-field
        // extraction calls before it. Both still call through to inner, so
        // the `Service<__Req> for <wrapper>` impl shape stays load-bearing.
        let input = parse_derive("struct AuthMw;");
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        assert!(
            rendered.contains(
                "impl < __S , __Req > :: joltr_core :: tower :: Service < __Req > for __JoltRAutoMiddlewareAuthMwService < __S >"
            ),
            "Service impl header must match, rendered: {rendered}"
        );
        assert!(
            rendered.contains("__S : :: joltr_core :: tower :: Service < __Req >"),
            "where-clause bound must match, rendered: {rendered}"
        );
        assert!(
            rendered.contains("fn call (& mut self , __req : __Req) -> Self :: Future"),
            "Service::call signature must match, rendered: {rendered}"
        );
        // `quote!`'s tokens print double-`>` without a space when one closes a
        // nested generic (`Service<__Req>>::call`). Match that exact shape.
        assert!(
            rendered.contains(
                ":: joltr_core :: tower :: Service < __Req >> :: call (& mut self . inner , __req)"
            ),
            "call() must delegate to inner.call, rendered: {rendered}"
        );
        assert!(
            rendered.contains(":: joltr_core :: tower :: Service < __Req >> :: poll_ready (& mut self . inner , __cx)"),
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
                "impl < __S : :: core :: clone :: Clone > :: core :: clone :: Clone for __JoltRAutoMiddlewareAuthMwService < __S >"
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
            rendered.contains("__JOLTR_AUTO_MIDDLEWARE_FIELD_COUNT"),
            "field-count const must remain, rendered: {rendered}"
        );
        assert!(
            rendered.contains("__JOLTR_AUTO_MIDDLEWARE_CORS"),
            "cors const must remain, rendered: {rendered}"
        );
        assert!(
            rendered.contains(":: joltr_core :: tower :: Layer < __S > for ChainedMw"),
            "Layer impl must be emitted alongside marker consts, rendered: {rendered}"
        );
        assert!(
            rendered.contains("__JoltRAutoMiddlewareChainedMwService"),
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
        // emitted, no fields to extract. JOLTR-RS-053 will iterate
        // `parsed.fields` to splice extraction; on a unit struct that
        // iteration yields nothing, so the call() body stays a pure
        // delegation — no special-case handling required.
        let input = parse_derive("struct Marker;");
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        assert!(
            rendered.contains("__JoltRAutoMiddlewareMarkerService"),
            "wrapper struct must be emitted for unit struct, rendered: {rendered}"
        );
        assert!(
            rendered.contains(":: joltr_core :: tower :: Layer < __S > for Marker"),
            "Layer impl must be emitted for unit struct, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_emits_layer_impl_even_when_cors_attribute_present() {
        // The cors flag does NOT change the layer-impl shape at 051 — the
        // wrapper service is identical for `#[cors]`-bearing and bare structs.
        // JOLTR-RS-052 reads `parsed.cors` to splice a cors STEP MARKER into
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
            rendered.contains(":: joltr_core :: tower :: Layer < __S > for WithCors"),
            "Layer impl must be emitted for cors-attr struct too, rendered: {rendered}"
        );
    }

    // ----- JOLTR-RS-052: middleware_chain + expand_layer_impl chain splicing -----
    //
    // These tests pin the per-derive chain construction (`middleware_chain`)
    // and the rendered shape of the spliced step markers in `call()`. PRD
    // verification: "Ordering logic generates correct chained calls."

    #[test]
    fn middleware_chain_is_empty_for_bare_unit_struct() {
        // Unit struct, no `#[cors]`, no fields → empty chain. The wrapper's
        // call() body is then a pure delegation, identical to 051's pre-chain
        // shape. Pinned so a regression that emits markers for a no-op
        // middleware surfaces immediately.
        let input = parse_derive("struct Marker;");
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(middleware_chain(&parsed), Vec::<MiddlewareStep>::new());
    }

    #[test]
    fn middleware_chain_pushes_cors_when_attribute_present() {
        // Single-step chain: just `#[cors]`, no extraction fields. The chain
        // is `[Cors]` — cors is the only present step, parse-query/parse-body
        // omitted because no QueryParams/Body field exists.
        let input = parse_derive(
            r#"
            #[cors]
            struct CorsOnly;
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(middleware_chain(&parsed), vec![MiddlewareStep::Cors]);
    }

    #[test]
    fn middleware_chain_pushes_parse_query_when_query_params_field_present() {
        // Single-step chain: just a QueryParams<T> field. ParseQuery is
        // active; Cors/ParseBody omitted.
        let input = parse_derive(
            r#"
            struct QueryOnly {
                q: QueryParams<Filters>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(middleware_chain(&parsed), vec![MiddlewareStep::ParseQuery]);
    }

    #[test]
    fn middleware_chain_pushes_parse_body_when_body_field_present() {
        // Single-step chain: just a `body: T` field. ParseBody is active;
        // Cors/ParseQuery omitted.
        let input = parse_derive(
            r#"
            struct BodyOnly {
                body: CreateUserRequest,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(middleware_chain(&parsed), vec![MiddlewareStep::ParseBody]);
    }

    #[test]
    fn middleware_chain_orders_steps_canonically() {
        // PRD canonical order: auth → cors → log → parse-query → parse-body
        // → user → handler. With all three implementable steps active
        // (`#[cors]` + QueryParams + Body), the chain must be exactly
        // `[Cors, ParseQuery, ParseBody]` — NOT the field declaration order.
        let input = parse_derive(
            r#"
            #[cors]
            struct AllThree {
                body: CreateUserRequest,
                query: QueryParams<Filters>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(
            middleware_chain(&parsed),
            vec![
                MiddlewareStep::Cors,
                MiddlewareStep::ParseQuery,
                MiddlewareStep::ParseBody,
            ]
        );
    }

    #[test]
    fn middleware_chain_order_independent_of_field_declaration_order() {
        // The struct lists `query` BEFORE `body`; the chain still orders
        // ParseQuery before ParseBody — but that's not what this test
        // verifies. What it verifies is that swapping the field order
        // (body first, then query) does NOT swap the chain order. The
        // chain is per-PRD canonical, not source-order.
        let input = parse_derive(
            r#"
            struct BodyFirst {
                body: CreateUserRequest,
                query: QueryParams<Filters>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(
            middleware_chain(&parsed),
            vec![MiddlewareStep::ParseQuery, MiddlewareStep::ParseBody],
            "ParseQuery must precede ParseBody regardless of field declaration order"
        );
    }

    #[test]
    fn middleware_chain_coalesces_multiple_query_params_fields() {
        // Two QueryParams<T> fields → ParseQuery step appears ONCE in the
        // chain. 053's per-field extraction will fan out the field-level
        // work inside the single step; the chain itself stays per-step. A
        // regression that pushed one chain entry per matching field would
        // surface here.
        let input = parse_derive(
            r#"
            struct TwoQueries {
                q1: QueryParams<Filters>,
                q2: QueryParams<Other>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(middleware_chain(&parsed), vec![MiddlewareStep::ParseQuery]);
    }

    #[test]
    fn middleware_chain_skips_other_kinded_fields() {
        // `headers: HashMap<String, Vec<u8>>` and primitives classify as
        // `Other`. A struct of only `Other` fields produces an empty chain —
        // no extraction step is implied by a field the framework doesn't
        // recognise.
        let input = parse_derive(
            r#"
            struct AllOther {
                headers: HashMap<String, Vec<u8>>,
                count: usize,
                flag: bool,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(middleware_chain(&parsed), Vec::<MiddlewareStep>::new());
    }

    #[test]
    fn middleware_chain_request_only_field_does_not_push_a_step() {
        // A `req: &Request` field is `FieldKind::Request` (049). Request
        // injection is a per-field shape that 053 will splice INSIDE the
        // existing call() body — it isn't a step in the canonical chain. So
        // a struct with only a Request field produces an empty chain. Pinned
        // because Request fields could plausibly have been modeled as a
        // chain step; the decision is that they aren't.
        let input = parse_derive(
            r#"
            struct ReqOnly<'a> {
                req: &'a Request,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(middleware_chain(&parsed), Vec::<MiddlewareStep>::new());
    }

    #[test]
    fn middleware_step_token_tags_are_namespaced_and_stable() {
        // The marker tags are public-API-like — they're the substring
        // 053 will splice on. Pinning them here ensures a typo-rename
        // (`step::cors` → `step::CORS`) doesn't silently break 053's splice
        // logic across iterations.
        assert_eq!(
            MiddlewareStep::Cors.token_tag(),
            "joltr::middleware::step::cors"
        );
        assert_eq!(
            MiddlewareStep::ParseQuery.token_tag(),
            "joltr::middleware::step::parse_query"
        );
        assert_eq!(
            MiddlewareStep::ParseBody.token_tag(),
            "joltr::middleware::step::parse_body"
        );
    }

    #[test]
    fn expand_layer_impl_emits_no_chain_markers_for_empty_chain() {
        // Bare unit struct → no chain steps → no marker string literals in
        // the rendered output. The wrapper's call() body is the bare
        // delegation, bit-for-bit equivalent to 051's pre-chain shape.
        let input = parse_derive("struct Marker;");
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        assert!(
            !rendered.contains("joltr::middleware::step::"),
            "empty chain must produce no step markers, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_layer_impl_emits_cors_marker_for_cors_attribute() {
        // `#[cors]` struct → cors marker statement appears in rendered call()
        // body. The marker is a typed-`&str` let binding to the stable tag.
        let input = parse_derive(
            r#"
            #[cors]
            struct CorsOnly;
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        assert!(
            rendered.contains("\"joltr::middleware::step::cors\""),
            "cors marker literal must appear, rendered: {rendered}"
        );
        assert!(
            rendered.contains("let _ : & :: core :: primitive :: str ="),
            "marker statement shape (typed &str discard) must match, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_layer_impl_emits_parse_query_marker_for_query_field() {
        let input = parse_derive(
            r#"
            struct QueryMw {
                q: QueryParams<Filters>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        assert!(
            rendered.contains("\"joltr::middleware::step::parse_query\""),
            "parse_query marker literal must appear, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_layer_impl_emits_parse_body_marker_for_body_field() {
        let input = parse_derive(
            r#"
            struct BodyMw {
                body: CreateUserRequest,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        assert!(
            rendered.contains("\"joltr::middleware::step::parse_body\""),
            "parse_body marker literal must appear, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_layer_impl_emits_chain_markers_in_canonical_order() {
        // PRD verification: "Ordering logic generates correct chained calls."
        // The fields are declared `body` BEFORE `query` and the struct also
        // carries `#[cors]`. The rendered call() body MUST contain the three
        // markers in canonical order (cors → parse_query → parse_body), NOT
        // in field declaration order. Verified by substring position.
        let input = parse_derive(
            r#"
            #[cors]
            struct AllThree {
                body: CreateUserRequest,
                query: QueryParams<Filters>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        let cors_pos = rendered
            .find("\"joltr::middleware::step::cors\"")
            .expect("cors marker present");
        let query_pos = rendered
            .find("\"joltr::middleware::step::parse_query\"")
            .expect("parse_query marker present");
        let body_pos = rendered
            .find("\"joltr::middleware::step::parse_body\"")
            .expect("parse_body marker present");
        assert!(
            cors_pos < query_pos,
            "cors must precede parse_query, rendered: {rendered}"
        );
        assert!(
            query_pos < body_pos,
            "parse_query must precede parse_body, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_layer_impl_chain_markers_precede_inner_call_delegation() {
        // The terminal expression of call() is still
        // `<__S as Service<__Req>>::call(&mut self.inner, __req)`. The chain
        // markers must appear BEFORE that delegation in source order — they're
        // statements (`let _: &str = ...;`) that 053 will replace with the
        // real step bodies executed before the inner call.
        let input = parse_derive(
            r#"
            #[cors]
            struct WithCors;
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        let cors_pos = rendered
            .find("\"joltr::middleware::step::cors\"")
            .expect("cors marker present");
        let inner_call_pos = rendered
            .find(
                ":: joltr_core :: tower :: Service < __Req >> :: call (& mut self . inner , __req)",
            )
            .expect("inner.call delegation present");
        assert!(
            cors_pos < inner_call_pos,
            "chain marker must precede inner.call, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_layer_impl_runs_extraction_before_inner_call_for_fields() {
        // PRD #24: field-bearing middleware must run the generated extraction
        // helper before handing the request to the wrapped service. The helper
        // only accepts `joltr_core::Request`, so the generic tower service uses
        // an `Any` downcast and executes the helper on that request shape.
        let input = parse_derive(
            r#"
            struct WithBody {
                body: CreateUserRequest,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        let extract_pos = rendered
            .find("WithBody :: __jolt_try_extract_from (__jolt_req)")
            .expect("extraction helper call present");
        let inner_call_pos = rendered
            .find(
                ":: joltr_core :: tower :: Service < __Req >> :: call (& mut self . inner , __req)",
            )
            .expect("inner.call delegation present");
        assert!(
            extract_pos < inner_call_pos,
            "extraction must run before inner.call, rendered: {rendered}"
        );
        assert!(
            rendered.contains("__Req : :: core :: any :: Any"),
            "field-bearing middleware must bound request type for downcast, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_layer_impl_chain_markers_are_inside_call_function_body() {
        // The markers must live inside `fn call(...)` — NOT inside
        // `poll_ready`, NOT outside any fn. A regression that hoisted them
        // to a sibling impl block would silently break 053's splice site.
        // Verified by checking the marker's position relative to
        // `fn call` and `fn poll_ready`.
        let input = parse_derive(
            r#"
            #[cors]
            struct WithCors;
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        let call_fn_pos = rendered
            .find("fn call (& mut self , __req : __Req)")
            .expect("fn call signature present");
        let cors_pos = rendered
            .find("\"joltr::middleware::step::cors\"")
            .expect("cors marker present");
        assert!(
            call_fn_pos < cors_pos,
            "chain marker must appear after `fn call(...)` signature, rendered: {rendered}"
        );
        // poll_ready precedes call in our emission order; ensure the marker
        // sits AFTER poll_ready (so it's in call's body, not poll_ready's).
        let poll_ready_pos = rendered
            .find("fn poll_ready")
            .expect("fn poll_ready signature present");
        assert!(
            poll_ready_pos < cors_pos,
            "chain marker must NOT be inside poll_ready, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_layer_impl_preserves_inner_call_delegation_with_chain_present() {
        // With chain markers spliced in, the terminal inner.call delegation
        // is STILL emitted — chain markers are additive, not replacements.
        // Pinned because a regression that replaced the delegation with a
        // chain step would break tower composition.
        let input = parse_derive(
            r#"
            #[cors]
            struct WithCors {
                body: CreateUserRequest,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        assert!(
            rendered.contains(
                ":: joltr_core :: tower :: Service < __Req >> :: call (& mut self . inner , __req)"
            ),
            "inner.call delegation must still be emitted alongside chain markers, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_layer_impl_coalesces_multiple_body_or_query_into_single_marker() {
        // Two QueryParams<T> fields → ParseQuery marker appears ONCE in the
        // rendered output. Pinned because a regression that emitted one
        // marker per matching field would silently bloat the chain. The
        // substring count must be exactly 1.
        let input = parse_derive(
            r#"
            struct TwoQueries {
                q1: QueryParams<Filters>,
                q2: QueryParams<Other>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_layer_impl(&parsed).to_string();
        let marker_count = rendered
            .matches("\"joltr::middleware::step::parse_query\"")
            .count();
        assert_eq!(
            marker_count, 1,
            "ParseQuery marker must appear exactly once for two QueryParams fields, rendered: {rendered}"
        );
    }

    // ----- JOLTR-RS-053: expand_extraction (per-field extraction codegen) -----
    //
    // The unit tests below pin the rendered token shape of `expand_extraction`
    // for each FieldKind dispatch. PRD verification: "Generated call() extracts
    // body into struct, query into struct, req ref into struct field." The
    // integration test in `joltr-core/tests/auto_middleware_derive.rs` is the
    // runtime witness; these unit tests pin the codegen shape so a regression
    // that changed the dispatch (e.g. swapped Body and Other init exprs)
    // surfaces here without going through the slower compile-and-run path.

    #[test]
    fn expand_extraction_emits_helper_method_on_user_struct() {
        // The helper is `pub fn __jolt_extract_from(__req: &::joltr_core::Request)
        // -> Self` on an `impl <Ident>` block, marked `#[doc(hidden)]` and
        // `#[automatically_derived]`. Pinning the surface here so a future
        // refactor that renamed the method or changed its visibility surfaces
        // immediately (the wrapper service in 052/future iterations needs to
        // call into this method, and a rename would silently break the splice).
        let input = parse_derive("struct Empty {}");
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_extraction(&parsed).to_string();
        assert!(
            rendered.contains("# [automatically_derived]"),
            "extraction impl must be tagged #[automatically_derived], rendered: {rendered}"
        );
        assert!(
            rendered.contains("impl Empty"),
            "extraction impl must target the user's struct, rendered: {rendered}"
        );
        assert!(
            rendered.contains("# [doc (hidden)]"),
            "helper method must be #[doc(hidden)], rendered: {rendered}"
        );
        assert!(
            rendered.contains(
                "pub fn __jolt_extract_from (__req : & :: joltr_core :: Request) -> Self"
            ),
            "helper signature must take &::joltr_core::Request and return Self, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_extraction_initializes_body_field_via_json() {
        // Body kind → init expression is `__req.json::<T>().expect(...)`. The
        // PRD-mandated "extracts body into struct" verification: a `body: T`
        // field gets deserialized via serde_json with the user's T spliced
        // into the turbofish. JOLTR-RS-062 will replace the .expect with typed
        // Result handling; until then a panic on bad-body is acceptable.
        let input = parse_derive(
            r#"
            struct WithBody {
                body: CreateUserRequest,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_extraction(&parsed).to_string();
        assert!(
            rendered.contains("body : __req . json :: < CreateUserRequest > ()"),
            "body field must be initialised via __req.json::<T>(), rendered: {rendered}"
        );
        assert!(
            rendered.contains(". expect ("),
            "body extraction must use .expect for now (JOLTR-RS-062 future Result handling), rendered: {rendered}"
        );
    }

    #[test]
    fn expand_extraction_initializes_hashmap_query_params_via_clone() {
        // QueryParams (HashMap shape) → init expression clones
        // `__req.query_params`. The PRD-mandated "query into struct" verification
        // for the raw-map shape: a `query_params: HashMap<String, String>` field
        // gets a copy of the request's parsed query map.
        let input = parse_derive(
            r#"
            struct WithQuery {
                query_params: HashMap<String, String>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_extraction(&parsed).to_string();
        assert!(
            rendered.contains(
                "query_params : :: core :: clone :: Clone :: clone (& __req . query_params)"
            ),
            "HashMap-shape query_params must be initialised via Clone::clone of req.query_params, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_extraction_initializes_typed_query_params_via_deserializer() {
        // Typed `QueryParams<T>` → deserialize the request query map into T,
        // then wrap it back into the user's QueryParams field type. Invalid
        // query values map to the existing bad-request response helper.
        let input = parse_derive(
            r#"
            struct WithTypedQuery {
                q: QueryParams<Filters>,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_extraction(&parsed).to_string();
        assert!(
            rendered.contains("deserialize_query :: < Filters > (& __req . query_params)"),
            "typed QueryParams<T> field must deserialize into the inner type, rendered: {rendered}"
        );
        assert!(
            rendered.contains("bad_request_for_query_error"),
            "typed query errors must map through the bad-request response helper, rendered: {rendered}"
        );
        assert!(
            rendered.contains("< QueryParams < Filters > as :: core :: convert :: From < _ >> :: from"),
            "typed query result must be wrapped into the declared QueryParams<T> field type, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_extraction_initializes_by_value_request_via_clone() {
        // Bare `Request` (by-value) → init expression clones the active
        // request via the fully-qualified `<::joltr_core::Request as Clone>::clone`
        // path. The fully-qualified syntax defends against a user shadowing
        // `Clone` in their crate. PRD-mandated "req ref into struct field"
        // verification: a `req: Request` field gets a clone of the inbound
        // request.
        let input = parse_derive(
            r#"
            struct WithReq {
                req: Request,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_extraction(&parsed).to_string();
        assert!(
            rendered.contains(
                "req : < :: joltr_core :: Request as :: core :: clone :: Clone > :: clone (__req)"
            ),
            "by-value Request field must be initialised via fully-qualified Clone::clone, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_extraction_initializes_by_ref_request_from_borrowed_request() {
        // `&Request` (by-ref) → init expression borrows the active request.
        // The helper signature threads the user's request lifetime through the
        // generated impl, so the returned middleware can hold the request ref.
        let input = parse_derive(
            r#"
            struct WithReq<'a> {
                req: &'a Request,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_extraction(&parsed).to_string();
        assert!(
            rendered.contains("impl < 'a > WithReq < 'a >"),
            "extraction impl must preserve user lifetime generics, rendered: {rendered}"
        );
        assert!(
            rendered.contains("__req : & 'a :: joltr_core :: Request"),
            "helper request parameter must use the user's request lifetime, rendered: {rendered}"
        );
        assert!(
            rendered.contains("req : __req"),
            "by-ref &Request field must be initialised from the borrowed request, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_extraction_initializes_other_field_via_default() {
        // Other kind → init expression is `<T as Default>::default()`. Per
        // the spec, framework-unknown fields use `Default::default()`
        // per-request. The fully-qualified syntax defends against a user
        // shadowing `Default` in their crate AND avoids ambiguity if the
        // field type implements multiple traits with a `default` method.
        let input = parse_derive(
            r#"
            struct WithOther {
                count: usize,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_extraction(&parsed).to_string();
        assert!(
            rendered.contains("count : < usize as :: core :: default :: Default > :: default ()"),
            "Other field must be initialised via fully-qualified Default::default, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_extraction_handles_unit_struct() {
        // Unit struct (`struct Marker;`) → no fields to initialise. The struct
        // literal renders as `Self { }` (empty braces), which Rust accepts
        // for both unit and named-empty structs. Pinned so a regression that
        // tried to special-case unit structs with bare `Self` (no braces)
        // doesn't silently change the codegen.
        let input = parse_derive("struct Marker;");
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_extraction(&parsed).to_string();
        assert!(
            rendered.contains("Self { }"),
            "unit struct must render as Self {{ }} (empty braces), rendered: {rendered}"
        );
        // No `unimplemented!`, no `__req.json`, no `Default::default` — none
        // of the per-field init exprs fire on a struct with zero fields.
        assert!(
            !rendered.contains(":: core :: unimplemented !"),
            "no placeholder for zero-field struct, rendered: {rendered}"
        );
        assert!(
            !rendered.contains("__req . json"),
            "no body extraction for zero-field struct, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_extraction_handles_mixed_kinds() {
        // End-to-end shape: a struct that mixes Body, QueryParams (HashMap),
        // Request (by-value), and Other must produce one init expression per
        // field with the right shape per kind. The struct literal is rendered
        // in field declaration order — that's important because the user's
        // struct definition fixes the order, and a struct literal that
        // reorders fields would compile but rely on Rust's positional-with-
        // names construction (still correct, but worth pinning).
        let input = parse_derive(
            r#"
            struct Mixed {
                body: CreateUserRequest,
                query_params: HashMap<String, String>,
                req: Request,
                count: usize,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        let rendered = expand_extraction(&parsed).to_string();
        assert!(
            rendered.contains("body : __req . json :: < CreateUserRequest > ()"),
            "Body init expression must appear, rendered: {rendered}"
        );
        assert!(
            rendered.contains(
                "query_params : :: core :: clone :: Clone :: clone (& __req . query_params)"
            ),
            "QueryParams init expression must appear, rendered: {rendered}"
        );
        assert!(
            rendered.contains(
                "req : < :: joltr_core :: Request as :: core :: clone :: Clone > :: clone (__req)"
            ),
            "Request init expression must appear, rendered: {rendered}"
        );
        assert!(
            rendered.contains("count : < usize as :: core :: default :: Default > :: default ()"),
            "Other init expression must appear, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_auto_middleware_includes_extraction_method() {
        // End-to-end: `expand_auto_middleware` splices the extraction method
        // into the rendered output alongside the marker consts and the layer
        // impl. All three observable surfaces coexist after 053.
        let input = parse_derive(
            r#"
            struct Mw {
                body: CreateUserRequest,
            }
            "#,
        );
        let rendered = expand_auto_middleware(input).to_string();
        assert!(
            rendered.contains("__JOLTR_AUTO_MIDDLEWARE_FIELD_COUNT"),
            "marker const must remain, rendered: {rendered}"
        );
        assert!(
            rendered.contains(":: joltr_core :: tower :: Layer < __S > for Mw"),
            "Layer impl must remain, rendered: {rendered}"
        );
        assert!(
            rendered.contains("__jolt_extract_from"),
            "extraction helper must be emitted, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_auto_middleware_with_body_query_and_request_fields_produces_all_surfaces() {
        // JOLTR-RS-054: declare struct with body + query_params(HashMap) + req(Request)
        // fields, expand through the full pipeline, verify every observable surface
        // (marker consts, Layer/Service impl, extraction method with all three
        // field-initialization expressions, chain markers for query and body steps).
        let input = parse_derive(
            r#"
            struct Mixed {
                body: CreateUserRequest,
                query_params: HashMap<String, String>,
                req: Request,
            }
            "#,
        );
        let rendered = expand_auto_middleware(input).to_string();
        assert!(
            rendered.contains("__JOLTR_AUTO_MIDDLEWARE_FIELD_COUNT"),
            "marker const for field count, rendered: {rendered}"
        );
        assert!(
            rendered.contains("__JOLTR_AUTO_MIDDLEWARE_CORS"),
            "marker const for cors, rendered: {rendered}"
        );
        assert!(
            rendered.contains(":: joltr_core :: tower :: Layer < __S > for Mixed"),
            "Layer impl for the struct, rendered: {rendered}"
        );
        assert!(
            rendered.contains(":: joltr_core :: tower :: Service <"),
            "Service impl for the wrapper, rendered: {rendered}"
        );
        assert!(
            rendered.contains("fn __jolt_extract_from"),
            "extraction helper method, rendered: {rendered}"
        );
        assert!(
            rendered.contains("__req . json :: < CreateUserRequest > ()"),
            "body extraction via json, rendered: {rendered}"
        );
        assert!(
            rendered.contains(":: core :: clone :: Clone :: clone (& __req . query_params)"),
            "query_params extraction via clone, rendered: {rendered}"
        );
        assert!(
            rendered.contains(
                ":: joltr_core :: Request as :: core :: clone :: Clone > :: clone (__req)"
            ),
            "req extraction via Clone on Request, rendered: {rendered}"
        );
        assert!(
            rendered.contains("joltr::middleware::step::parse_query"),
            "parse_query chain marker, rendered: {rendered}"
        );
        assert!(
            rendered.contains("joltr::middleware::step::parse_body"),
            "parse_body chain marker, rendered: {rendered}"
        );
    }
}
