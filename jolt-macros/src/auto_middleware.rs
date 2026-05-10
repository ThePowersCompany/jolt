//! `#[derive(AutoMiddleware)]` proc-macro derive — phase10 field parsing.
//!
//! Phase10 ladder:
//! - JOLT-RS-046: parsed the struct's fields and their types into
//!   [`AutoMiddlewareInput`] + [`AutoMiddlewareField`]. The derive emits a
//!   minimal hidden marker so an integration test can verify the derive
//!   compiled and parsed without depending on later codegen.
//! - JOLT-RS-047 (this iteration): classify each parsed field with a
//!   [`FieldKind`]. The body-candidate rule fires when `field.ident == "body"`
//!   per the spec ("auto-applies body parsing if `body: T` field exists"); the
//!   field's type is captured verbatim so the body-extraction codegen in 053
//!   can name `T` in `__req.json::<T>()`. JOLT-RS-048/049 extend [`FieldKind`]
//!   with the `QueryParams` and `Request` variants.
//! - JOLT-RS-048 will mark query-extraction fields (`QueryParams<T>` or
//!   `HashMap<String, String>`).
//! - JOLT-RS-049 will mark request-injection fields (`&Request` or `Request`).
//! - JOLT-RS-050 will detect the `#[cors]` struct-level attribute.
//!
//! The parse entry point is split out from `lib.rs` so it can be unit-tested
//! against a `proc_macro2::TokenStream` / parsed `syn::DeriveInput`
//! (proc-macro entry points themselves cannot be invoked outside the compiler).

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DataStruct, DeriveInput, Fields, Ident, Type};

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
/// Detection is name-based per the spec ("auto-applies body parsing if
/// `body: T` field exists"). 048 will add `QueryParams` (matching `query_params`
/// by name and `QueryParams<T>` / `HashMap<String, String>` by type); 049 will
/// add `Request` (matching `req` by name and `Request` / `&Request` by type).
/// Until those land, every non-`body` field falls through to [`FieldKind::Other`]
/// — including ones the spec eventually marks as Query/Request — which is the
/// correct behavior for 047 in isolation: those fields aren't body-candidates,
/// and their final classification is 048/049's responsibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldKind {
    /// Body-candidate. The spec triggers JSON body parsing when a field named
    /// `body: T` is present; codegen in 053 will emit `__req.json::<T>()` and
    /// assign the result into the `body` field of the constructed middleware.
    Body,
    /// Catch-all for fields not yet classified. 048/049 narrow this further by
    /// adding `QueryParams` and `Request` variants. After phase10 closes, an
    /// `Other` field is one the user added that doesn't trigger any framework
    /// extraction — codegen treats it as `Default::default()` per-request (the
    /// same default-construction contract used by `#[endpoint]` wrappers; see
    /// JOLT-RS-043's progress notes).
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
                let kind = classify_field(&field_ident);
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

fn classify_field(ident: &Ident) -> FieldKind {
    if ident == "body" {
        FieldKind::Body
    } else {
        FieldKind::Other
    }
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
    fn body_field_classification_does_not_depend_on_type() {
        // Detection is name-based: even if the user writes `body: Request`
        // (where `Request` is the framework type 049 will detect), 047 still
        // marks it as Body because the field is NAMED `body`. 048/049 are
        // free to add a precedence rule later (e.g. a Request-typed `body`
        // field is treated as Request, not Body) — but that's their call,
        // not 047's. Pinning the name-only behavior so a future change is
        // explicit.
        let input = parse_derive(
            r#"
            struct WithRequestNamedBody {
                body: Request,
            }
            "#,
        );
        let parsed = parse_auto_middleware_input(input).expect("parses");
        assert_eq!(parsed.fields[0].kind, FieldKind::Body);
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
}
