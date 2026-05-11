//! `#[derive(PatchQuery)]` proc-macro derive — phase26 field parsing.
//!
//! Phase26 ladder:
//! - JOLT-RS-110: parse the struct's named fields and their types into
//!   [`PatchQueryInput`] + [`PatchQueryField`]. The derive emits a minimal
//!   hidden marker (`__JOLT_PATCH_QUERY_FIELD_COUNT: usize`) so an
//!   integration test can verify the derive compiled and parsed the field
//!   count without depending on later codegen.
//! - JOLT-RS-111: parse the struct-level `#[patch("users")]` attribute to
//!   extract the target table name.
//! - JOLT-RS-112 (this iteration): detect `Optional<T>` fields via
//!   [`optional_inner_type`], extract the inner `T`, and mark each field
//!   with [`PatchQueryField::is_optional`] + [`PatchQueryField::inner_type`].
//!   The derive emits an additional `__JOLT_PATCH_QUERY_OPTIONAL_COUNT:
//!   usize` hidden marker so the integration test can observe the
//!   classification.
//! - JOLT-RS-113: build the [`Vec<PatchField>`] internal representation
//!   carrying `name`, `column_name`, `is_optional`, `inner_type`.
//! - JOLT-RS-114..117: codegen `fn to_patch_query(&self, id_column: &str,
//!   id_value: &impl ToSql) -> (String, Vec<&dyn ToSql>)` and the
//!   `Some(_) → SET col = $N` / `Null → SET col = NULL` /
//!   `NotProvided → skip` dispatch.
//!
//! The parse entry point is split out from `lib.rs` so it can be unit-tested
//! against a `proc_macro2::TokenStream` / parsed `syn::DeriveInput`
//! (proc-macro entry points themselves cannot be invoked outside the
//! compiler). Mirrors the same split established by
//! [`crate::auto_middleware::parse_auto_middleware_input`] (JOLT-RS-046).

use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    parse2, Data, DataStruct, DeriveInput, Fields, GenericArgument, Ident, LitStr, Meta,
    PathArguments, Type,
};

/// Check whether `ty` is `Optional<T>` and, if so, return a reference to the
/// inner `T`.
///
/// JOLT-RS-112: a field typed `Optional<String>` has `is_optional = true` and
/// `inner_type = Some(Type::Path("String"))`. A field typed `u32`, `Vec<T>`,
/// or `MyType` has `is_optional = false` and `inner_type = None`.
///
/// Detection inspects the type-path structure: the outermost path segment must
/// be identically `Optional` and must carry exactly one angle-bracketed
/// generic type argument. Named-`where`-clause generics (e.g.
/// `Optional<T> where T: Clone`) are not angle-bracketed and will not match —
/// the user's struct-level `where` clauses live outside the field type's
/// grammar.
///
/// `Optional` is matched by identity alone — no path resolution (no attempt
/// to confirm `Optional` resolves to a concrete definition). This is a
/// deliberate choice: the derive operates on syntax, not semantics; the
/// compiler will later type-check that the user's `Optional` actually exists
/// and carries the expected variants. If a user names their own type
/// `Optional` with different semantics, the compiler's type-system catches
/// the mismatch, not the derive parser.
pub(crate) fn optional_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(type_path) = ty {
        let segments = &type_path.path.segments;
        if let Some(last) = segments.last() {
            if last.ident == "Optional" {
                if let PathArguments::AngleBracketed(args) = &last.arguments {
                    let generic_args = &args.args;
                    if generic_args.len() == 1 {
                        if let GenericArgument::Type(inner) = &generic_args[0] {
                            return Some(inner);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Parsed shape of a `#[derive(PatchQuery)]` input.
///
/// JOLT-RS-110 captures the struct identifier and per-field metadata (name +
/// type, verbatim). JOLT-RS-111 adds the struct-level `#[patch("table_name")]`
/// attribute extraction into [`table_name`]. JOLT-RS-112 adds
/// [`PatchQueryField::is_optional`] + [`PatchQueryField::inner_type`]
/// classification to each field via [`optional_inner_type`]. Later phase26
/// slices extend [`PatchQueryField`] with column-name overrides; this shape
/// keeps the representation minimal until 113 graduates to the `PatchField`
/// IR.
#[derive(Debug)]
pub(crate) struct PatchQueryInput {
    pub(crate) ident: Ident,
    pub(crate) fields: Vec<PatchQueryField>,
    pub(crate) table_name: Option<String>,
}

/// One field on a `#[derive(PatchQuery)]` struct.
///
/// JOLT-RS-110 captures the ident and type verbatim from `syn::Field`. JOLT-RS-112
/// extends with [`is_optional`] and [`inner_type`] classification via
/// [`optional_inner_type`]: if the field's type is `Optional<T>`,
/// `is_optional` is `true` and `inner_type` holds `T`; otherwise `false` and
/// `None`. JOLT-RS-113 will fold this into the `PatchField` internal
/// representation.
#[derive(Debug, Clone)]
pub(crate) struct PatchQueryField {
    #[allow(dead_code)]
    pub(crate) ident: Ident,
    #[allow(dead_code)]
    pub(crate) ty: Type,
    pub(crate) is_optional: bool,
    #[allow(dead_code)]
    pub(crate) inner_type: Option<Type>,
}

/// Parse a `DeriveInput` into [`PatchQueryInput`].
///
/// Acceptance rules (mirror [`crate::auto_middleware::parse_auto_middleware_input`]'s
/// decision set from JOLT-RS-046):
/// - Must be a `struct`. Enums and unions are rejected with a span pointing
///   at the offending keyword.
/// - Named-fields struct → captured field-by-field.
/// - Unit struct → accepted with an empty field list. A patch-target with no
///   updatable columns is degenerate but not malformed; later slices will
///   surface a clearer error at codegen time (an UPDATE with no SET clause
///   is a SQL error, not a parse error).
/// - Tuple struct → rejected. The SET clause's column names come from the
///   struct's named fields; positional fields can't carry that meaning, so
///   accepting them would force a separate naming rule that doesn't compose
///   with named-field structs.
pub(crate) fn parse_patch_query_input(
    input: DeriveInput,
) -> syn::Result<PatchQueryInput> {
    let ident = input.ident.clone();
    let table_name = parse_patch_table_attr(&input.attrs)?;
    match input.data {
        Data::Struct(s) => {
            let fields = parse_struct_fields(&s, &ident)?;
            Ok(PatchQueryInput { ident, fields, table_name })
        }
        Data::Enum(e) => Err(syn::Error::new_spanned(
            e.enum_token,
            "#[derive(PatchQuery)] can only be applied to structs, not enums",
        )),
        Data::Union(u) => Err(syn::Error::new_spanned(
            u.union_token,
            "#[derive(PatchQuery)] can only be applied to structs, not unions",
        )),
    }
}

/// Parse the struct-level `#[patch("table_name")]` attribute.
///
/// JOLT-RS-111: walks the DeriveInput's attrs for one matching
/// `#[patch("table_name")]`, extracts the string-literal argument, and returns
/// it. If the attribute is absent, returns `Ok(None)` (table-name-less patches
/// are syntactically valid; downstream codegen surfaces the error). If the
/// attribute is present but malformed (wrong shape, missing string literal,
/// duplicate), returns a `syn::Error`.
fn parse_patch_table_attr(
    attrs: &[syn::Attribute],
) -> syn::Result<Option<String>> {
    let mut found: Option<String> = None;
    for attr in attrs {
        if !attr.path().is_ident("patch") {
            continue;
        }
        let meta = &attr.meta;
        let inner: TokenStream = match meta {
            Meta::List(list) => list.tokens.clone(),
            _ => {
                return Err(syn::Error::new_spanned(
                    meta,
                    "expected #[patch(\"table_name\")] with a string literal argument",
                ));
            }
        };
        let lit: LitStr = parse2(inner).map_err(|_| {
            syn::Error::new_spanned(
                meta,
                "expected #[patch(\"table_name\")] with a single string literal argument",
            )
        })?;
        if found.is_some() {
            return Err(syn::Error::new_spanned(
                meta,
                "duplicate #[patch(\"...\")] attribute: only one table name is allowed",
            ));
        }
        found = Some(lit.value());
    }
    Ok(found)
}

fn parse_struct_fields(
    data: &DataStruct,
    owner: &Ident,
) -> syn::Result<Vec<PatchQueryField>> {
    match &data.fields {
        Fields::Named(named) => {
            let mut out = Vec::with_capacity(named.named.len());
            for field in &named.named {
                let field_ident = field
                    .ident
                    .clone()
                    .expect("Fields::Named guarantees every field has an ident");
                let is_optional = optional_inner_type(&field.ty).is_some();
                let inner_type = optional_inner_type(&field.ty).cloned();
                out.push(PatchQueryField {
                    ident: field_ident,
                    ty: field.ty.clone(),
                    is_optional,
                    inner_type,
                });
            }
            Ok(out)
        }
        Fields::Unit => Ok(Vec::new()),
        Fields::Unnamed(_) => Err(syn::Error::new_spanned(
            owner,
            "#[derive(PatchQuery)] requires named fields (tuple structs aren't supported; \
             SET-clause column names come from field idents)",
        )),
    }
}

/// Top-level driver for `#[derive(PatchQuery)]`.
///
/// Parses via [`parse_patch_query_input`] and emits a hidden marker impl
/// carrying `__JOLT_PATCH_QUERY_FIELD_COUNT: usize` and
/// `__JOLT_PATCH_QUERY_OPTIONAL_COUNT: usize` so the integration test in
/// `jolt-core/tests/patch_query_derive.rs` can witness that parsing observed
/// the right field count and optional-field classification. Later slices
/// (113-117) extend the emission with the `to_patch_query` method itself.
///
/// On parse failure the emission is a single `compile_error!` token (with the
/// span the parser attached) — no marker impl, no partial codegen. Mirrors
/// [`crate::auto_middleware::expand_auto_middleware`]'s contract from
/// JOLT-RS-046.
pub(crate) fn expand_patch_query(input: DeriveInput) -> TokenStream {
    let parsed = match parse_patch_query_input(input) {
        Ok(p) => p,
        Err(err) => return err.to_compile_error(),
    };
    let ident = &parsed.ident;
    let field_count = parsed.fields.len();
    let optional_count = parsed.fields.iter().filter(|f| f.is_optional).count();
    let table_name_const = match &parsed.table_name {
        Some(t) => quote! {
            #[doc(hidden)]
            pub const __JOLT_PATCH_QUERY_TABLE_NAME: &'static str = #t;
        },
        None => {
            quote! {
                #[doc(hidden)]
                pub const __JOLT_PATCH_QUERY_TABLE_NAME: ::core::option::Option<&'static str> = ::core::option::Option::None;
            }
        }
    };
    quote! {
        #[automatically_derived]
        impl #ident {
            #[doc(hidden)]
            pub const __JOLT_PATCH_QUERY_FIELD_COUNT: usize = #field_count;
            #[doc(hidden)]
            pub const __JOLT_PATCH_QUERY_OPTIONAL_COUNT: usize = #optional_count;
            #table_name_const
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

    fn with_patch_attr(mut input: DeriveInput, table: &str) -> DeriveInput {
        let meta: syn::Meta = syn::parse_quote!(patch(#table));
        let attr = syn::Attribute {
            pound_token: syn::token::Pound::default(),
            style: syn::AttrStyle::Outer,
            bracket_token: syn::token::Bracket::default(),
            meta,
        };
        input.attrs.push(attr);
        input
    }

    fn with_non_list_patch_attr(mut input: DeriveInput) -> DeriveInput {
        let meta: syn::Meta = syn::parse_quote!(patch = "users");
        let attr = syn::Attribute {
            pound_token: syn::token::Pound::default(),
            style: syn::AttrStyle::Outer,
            bracket_token: syn::token::Bracket::default(),
            meta,
        };
        input.attrs.push(attr);
        input
    }

    fn with_non_string_patch_attr(mut input: DeriveInput) -> DeriveInput {
        let meta: syn::Meta = syn::parse_quote!(patch(42));
        let attr = syn::Attribute {
            pound_token: syn::token::Pound::default(),
            style: syn::AttrStyle::Outer,
            bracket_token: syn::token::Bracket::default(),
            meta,
        };
        input.attrs.push(attr);
        input
    }

    #[test]
    fn parses_unit_struct_as_zero_fields() {
        let input = parse_derive("struct EmptyPatch;");
        let parsed = parse_patch_query_input(input).expect("unit struct parses");
        assert_eq!(parsed.ident, "EmptyPatch");
        assert!(parsed.fields.is_empty(), "unit struct has zero fields");
    }

    #[test]
    fn parses_struct_with_named_fields() {
        let input = parse_derive(
            r#"
            struct UserPatch {
                name: Optional<String>,
                email: Optional<String>,
                age: Optional<u32>,
            }
            "#,
        );
        let parsed = parse_patch_query_input(input).expect("named-field struct parses");
        assert_eq!(parsed.ident, "UserPatch");
        assert_eq!(parsed.fields.len(), 3);
        let names: Vec<String> = parsed.fields.iter().map(|f| f.ident.to_string()).collect();
        assert_eq!(names, vec!["name", "email", "age"]);
    }

    #[test]
    fn parses_struct_with_optional_and_plain_fields() {
        // The PRD-mandated verification: a struct mixing Optional<T> fields and
        // plain (non-Optional) fields must parse — 112 will later classify the
        // Optional fields as tri-state and leave the plain ones alone, but at
        // 110 every field is captured verbatim regardless of optional-ness.
        let input = parse_derive(
            r#"
            struct MixedPatch {
                name: Optional<String>,
                count: u32,
                tags: Vec<String>,
                bio: Optional<String>,
            }
            "#,
        );
        let parsed = parse_patch_query_input(input).expect("mixed struct parses");
        assert_eq!(parsed.fields.len(), 4);
        let names: Vec<String> = parsed.fields.iter().map(|f| f.ident.to_string()).collect();
        assert_eq!(names, vec!["name", "count", "tags", "bio"]);
    }

    #[test]
    fn preserves_field_types_verbatim() {
        // 112 will need to inspect the type to detect `Optional<T>` and extract
        // the inner `T`. Pin that the type is preserved verbatim from syn —
        // a regression that flattened the type (e.g. dropped generic args)
        // would surface here as a missing inner segment.
        let input = parse_derive(
            r#"
            struct TypePatch {
                title: Optional<String>,
            }
            "#,
        );
        let parsed = parse_patch_query_input(input).expect("parses");
        assert_eq!(parsed.fields.len(), 1);
        let f0_ty = &parsed.fields[0].ty;
        let rendered = quote! { #f0_ty }.to_string();
        assert!(
            rendered.contains("Optional") && rendered.contains("String"),
            "field type must preserve Optional<String> shape, got: {rendered}"
        );
    }

    #[test]
    fn rejects_enum() {
        let input = parse_derive("enum Bad { A, B }");
        let err = parse_patch_query_input(input).expect_err("enum must be rejected");
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
        let err = parse_patch_query_input(input).expect_err("union must be rejected");
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
        let input = parse_derive("struct TuplePatch(String, u32);");
        let err = parse_patch_query_input(input).expect_err("tuple struct must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("named fields"),
            "diagnostic must mention named fields, got: {msg}"
        );
    }

    #[test]
    fn expand_emits_field_count_marker_const() {
        // Witness that expansion of a well-formed input produces an `impl` block
        // carrying the field-count marker. We can't run the macro through the
        // compiler from here, but we can substring-check the emitted token
        // stream.
        let input = parse_derive(
            r#"
            struct UserPatch {
                name: Optional<String>,
                email: Optional<String>,
            }
            "#,
        );
        let out = expand_patch_query(input).to_string();
        assert!(
            out.contains("__JOLT_PATCH_QUERY_FIELD_COUNT"),
            "emission must declare the marker const, got: {out}"
        );
        assert!(
            out.contains("2usize") || out.contains("2 usize") || out.contains(": usize = 2"),
            "marker const must carry the parsed field count (2), got: {out}"
        );
    }

    #[test]
    fn expand_emits_compile_error_on_enum() {
        // A parse failure must surface as a `compile_error!` token, not a
        // partial codegen — mirrors JOLT-RS-046's contract.
        let input = parse_derive("enum Bad { A, B }");
        let out = expand_patch_query(input).to_string();
        assert!(
            out.contains("compile_error"),
            "parse failure must emit compile_error!, got: {out}"
        );
        assert!(
            !out.contains("__JOLT_PATCH_QUERY_FIELD_COUNT"),
            "no partial codegen on parse failure, got: {out}"
        );
    }

    // ── JOLT-RS-111: #[patch("table_name")] attribute parsing ──

    #[test]
    fn parse_patch_table_attr_extracts_table_name() {
        let input = with_patch_attr(
            parse_derive(
                r#"
                struct UserPatch {
                    name: Optional<String>,
                }
                "#,
            ),
            "users",
        );
        let parsed = parse_patch_query_input(input).expect("parses");
        assert_eq!(parsed.table_name.as_deref(), Some("users"));
    }

    #[test]
    fn parse_patch_table_attr_returns_none_when_absent() {
        let input = parse_derive(
            r#"
            struct NoTable {
                name: Optional<String>,
            }
            "#,
        );
        let parsed = parse_patch_query_input(input).expect("parses");
        assert_eq!(parsed.table_name, None);
    }

    #[test]
    fn parse_patch_table_attr_rejects_duplicate_attribute() {
        let input = with_patch_attr(
            with_patch_attr(
                parse_derive(
                    r#"
                    struct DupePatch {
                        name: Optional<String>,
                    }
                    "#,
                ),
                "users",
            ),
            "accounts",
        );
        let err = parse_patch_query_input(input)
            .expect_err("duplicate #[patch] must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate"),
            "diagnostic must mention duplicate, got: {msg}"
        );
    }

    #[test]
    fn parse_patch_table_attr_rejects_non_string_argument() {
        let input = with_non_string_patch_attr(
            parse_derive(
                r#"
                struct BadPatch {
                    name: Optional<String>,
                }
                "#,
            ),
        );
        let err = parse_patch_query_input(input)
            .expect_err("non-string #[patch] arg must be rejected");
        assert!(
            err.to_string().contains("string literal"),
            "diagnostic must mention string literal, got: {err}"
        );
    }

    #[test]
    fn parse_patch_table_attr_rejects_non_list_form() {
        let input = with_non_list_patch_attr(
            parse_derive(
                r#"
                struct BadPatch {
                    name: Optional<String>,
                }
                "#,
            ),
        );
        let err = parse_patch_query_input(input)
            .expect_err("NameValue #[patch] must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("string literal") || msg.contains("expected"),
            "diagnostic must describe expected shape, got: {msg}"
        );
    }

    #[test]
    fn expand_emits_table_name_when_present() {
        let input = with_patch_attr(
            parse_derive(
                r#"
                struct UserPatch {
                    name: Optional<String>,
                }
                "#,
            ),
            "users",
        );
        let out = expand_patch_query(input).to_string();
        assert!(
            out.contains("__JOLT_PATCH_QUERY_TABLE_NAME"),
            "emission must declare the table-name const, got: {out}"
        );
        assert!(
            out.contains("\"users\""),
            "table-name const must carry the parsed table name (\"users\"), got: {out}"
        );
    }

    #[test]
    fn expand_emits_none_table_name_when_absent() {
        let input = parse_derive(
            r#"
            struct NoTable {
                name: Optional<String>,
            }
            "#,
        );
        let out = expand_patch_query(input).to_string();
        assert!(
            out.contains("__JOLT_PATCH_QUERY_TABLE_NAME"),
            "emission must declare the table-name const even when absent, got: {out}"
        );
        assert!(
            out.contains("Option::None") || out.contains("core :: option :: Option :: None"),
            "absent table must emit Option::None, got: {out}"
        );
    }

    // ── JOLT-RS-112: Optional<T> detection ──

    #[test]
    fn detects_optional_string_field() {
        let input = parse_derive(
            r#"
            struct UserPatch {
                name: Optional<String>,
            }
            "#,
        );
        let parsed = parse_patch_query_input(input).expect("parses");
        assert_eq!(parsed.fields.len(), 1);
        let f = &parsed.fields[0];
        assert!(f.is_optional, "Optional<String> must be detected as optional");
        assert!(f.inner_type.is_some(), "inner_type must be Some(String)");
        let inner = f.inner_type.as_ref().unwrap();
        let rendered = quote! { #inner }.to_string();
        assert!(
            rendered == "String",
            "inner type must be String, got: {rendered}"
        );
    }

    #[test]
    fn detects_optional_u32_field() {
        let input = parse_derive(
            r#"
            struct NumPatch {
                count: Optional<u32>,
            }
            "#,
        );
        let parsed = parse_patch_query_input(input).expect("parses");
        let f = &parsed.fields[0];
        assert!(f.is_optional, "Optional<u32> must be detected as optional");
        let inner = f.inner_type.as_ref().unwrap();
        let rendered = quote! { #inner }.to_string();
        assert!(
            rendered == "u32",
            "inner type must be u32, got: {rendered}"
        );
    }

    #[test]
    fn plain_field_is_not_optional() {
        let input = parse_derive(
            r#"
            struct PlainPatch {
                view_count: u64,
                title: String,
            }
            "#,
        );
        let parsed = parse_patch_query_input(input).expect("parses");
        assert_eq!(parsed.fields.len(), 2);
        for f in &parsed.fields {
            assert!(
                !f.is_optional,
                "field {} must not be detected as optional",
                f.ident
            );
            assert!(
                f.inner_type.is_none(),
                "plain field {} must have inner_type=None",
                f.ident
            );
        }
    }

    #[test]
    fn vec_field_is_not_optional() {
        // Vec<String> looks like a generic type with one arg, so the detection
        // must NOT treat it as Optional — it checks the ident, not just the
        // angle-bracket shape.
        let input = parse_derive(
            r#"
            struct ListPatch {
                tags: Vec<String>,
            }
            "#,
        );
        let parsed = parse_patch_query_input(input).expect("parses");
        let f = &parsed.fields[0];
        assert!(!f.is_optional, "Vec<String> must not be detected as optional");
        assert!(f.inner_type.is_none(), "Vec<String> must have inner_type=None");
    }

    #[test]
    fn mixed_optional_and_plain_fields_are_classified_correctly() {
        let input = parse_derive(
            r#"
            struct MixedPatch {
                name: Optional<String>,
                count: u32,
                tags: Vec<String>,
                bio: Optional<String>,
            }
            "#,
        );
        let parsed = parse_patch_query_input(input).expect("parses");
        assert_eq!(parsed.fields.len(), 4);
        assert!(parsed.fields[0].is_optional);
        assert!(!parsed.fields[1].is_optional);
        assert!(!parsed.fields[2].is_optional);
        assert!(parsed.fields[3].is_optional);
        assert!(parsed.fields[0].inner_type.is_some());
        assert!(parsed.fields[1].inner_type.is_none());
        assert!(parsed.fields[2].inner_type.is_none());
        assert!(parsed.fields[3].inner_type.is_some());
    }

    #[test]
    fn expand_emits_optional_count_marker() {
        let input = parse_derive(
            r#"
            struct UserPatch {
                name: Optional<String>,
                email: Optional<String>,
                bio: Optional<String>,
            }
            "#,
        );
        let out = expand_patch_query(input).to_string();
        assert!(
            out.contains("__JOLT_PATCH_QUERY_OPTIONAL_COUNT"),
            "emission must declare the optional-count const, got: {out}"
        );
        assert!(
            out.contains(": usize = 3") || out.contains("3usize"),
            "optional-count const must carry 3, got: {out}"
        );
    }

    #[test]
    fn expand_emits_zero_optional_count_for_plain_struct() {
        let input = parse_derive(
            r#"
            struct PlainPatch {
                count: u32,
                name: String,
            }
            "#,
        );
        let out = expand_patch_query(input).to_string();
        assert!(
            out.contains(": usize = 0") || out.contains("0usize"),
            "optional-count const must be 0 for all-plain struct, got: {out}"
        );
    }

    #[test]
    fn optional_inner_type_returns_none_for_non_path_types() {
        use syn::Type;
        let ty: Type = syn::parse_quote! { &str };
        assert!(optional_inner_type(&ty).is_none(), "&str is not Optional");
    }

    #[test]
    fn optional_inner_type_returns_none_for_no_arg_optional() {
        // Bare `Optional` without angle-bracketed args — not Optional<T>.
        let ty: Type = syn::parse_quote! { Optional };
        assert!(
            optional_inner_type(&ty).is_none(),
            "bare Optional (no generic arg) is not Optional<T>"
        );
    }

    #[test]
    fn optional_inner_type_returns_none_for_two_arg_type() {
        // `Result<T, E>` has two angle-bracket args — must not be mistaken for
        // Optional<T> (which has exactly one).
        let ty: Type = syn::parse_quote! { Result<String, ()> };
        assert!(
            optional_inner_type(&ty).is_none(),
            "Result<_, _> must not be mistaken for Optional<T>"
        );
    }
}
