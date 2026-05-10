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
//! - JOLT-RS-049 (this iteration): mark request-injection fields. Type-based
//!   rule, regardless of name: any field whose type is `Request` or `&Request`
//!   (with or without an explicit lifetime) is [`FieldKind::Request`]. Mutable
//!   references (`&mut Request`) are NOT matched — middleware injection is the
//!   shared-reference shape per the spec, and excluding mut refs keeps the
//!   surface narrow. Path qualification on the inner type is allowed
//!   (`crate::Request`, `&::jolt_core::Request`) via last-path-segment
//!   matching. The Request rule lives between QueryParams and the body-name
//!   rule so a hypothetical `body: &Request` classifies as Request, pinning
//!   type-before-name precedence consistently with 048's QueryParams rule.
//! - JOLT-RS-050 will detect the `#[cors]` struct-level attribute.
//!
//! The parse entry point is split out from `lib.rs` so it can be unit-tested
//! against a `proc_macro2::TokenStream` / parsed `syn::DeriveInput`
//! (proc-macro entry points themselves cannot be invoked outside the compiler).

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DataStruct, DeriveInput, Fields, GenericArgument, Ident, PathArguments, Type};

/// Parsed shape of a `#[derive(AutoMiddleware)]` input.
///
/// JOLT-RS-046 only captures the struct identifier and per-field metadata;
/// later phase10/11 items extend [`AutoMiddlewareField`] with kind
/// classification (body, query, req) and append struct-level attribute parsing
/// (e.g. `#[cors]`). The struct ident is kept verbatim from the source so
/// codegen can emit `impl <ident>` blocks targeting the user's type.
#[derive(Debug)]
#[allow(dead_code)] // ident is consumed by expand_auto_middleware; fields are read by tests this iteration. JOLT-RS-047+ wires kind classification on top.
pub(crate) struct AutoMiddlewareInput {
    pub(crate) ident: Ident,
    pub(crate) fields: Vec<AutoMiddlewareField>,
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
    match input.data {
        Data::Struct(s) => {
            let fields = parse_struct_fields(&s, &ident)?;
            Ok(AutoMiddlewareInput { ident, fields })
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
/// Parses via [`parse_auto_middleware_input`] and emits a hidden marker impl:
///
/// ```ignore
/// #[automatically_derived]
/// impl <Struct> {
///     #[doc(hidden)]
///     pub const __JOLT_AUTO_MIDDLEWARE_FIELD_COUNT: usize = N;
/// }
/// ```
///
/// The const is the minimal observable artifact for the 046-mandated
/// "derive compiles on a struct with various field types" verification: an
/// integration test can `assert_eq!(MyStruct::__JOLT_AUTO_MIDDLEWARE_FIELD_COUNT, N)`
/// to confirm the derive ran AND the field count matches what the user wrote.
/// JOLT-RS-051+ will replace this marker with the real `tower::Layer` impl;
/// the const can stay as a `cfg(test)`-gated parse witness or be removed at
/// that point.
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
    quote! {
        #[automatically_derived]
        impl #ident {
            #[doc(hidden)]
            pub const __JOLT_AUTO_MIDDLEWARE_FIELD_COUNT: usize = #field_count;
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
}
