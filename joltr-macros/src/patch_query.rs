//! `#[derive(PatchQuery)]` proc-macro derive — phase26 field parsing + phase27 codegen.
//!
//! Phase26 ladder:
//! - JOLTR-RS-110: parse the struct's named fields and their types into
//!   [`PatchQueryInput`]. The derive emits a hidden marker
//!   (`__JOLTR_PATCH_QUERY_FIELD_COUNT: usize`) so an integration test can
//!   verify the derive compiled and parsed the field count without depending
//!   on later codegen.
//! - JOLTR-RS-111: parse the struct-level `#[patch("users")]` attribute to
//!   extract the target table name.
//! - JOLTR-RS-112: detect `Optional<T>` fields via [`optional_inner_type`],
//!   extract the inner `T`, emit `__JOLTR_PATCH_QUERY_OPTIONAL_COUNT`.
//! - JOLTR-RS-113: graduate the per-field representation to [`PatchField`] —
//!   the canonical internal representation carrying `name`, `column_name`,
//!   `is_optional`, `inner_type`.
//!
//! Phase27 ladder:
//! - JOLTR-RS-114: generate `fn to_patch_query(&self,
//!   id_column: &str, id_value: &impl ToSql) -> (String, Vec<&dyn ToSql>)`
//!   that walks each field and builds the SET clause via [`generate_to_patch_query`].
//! - JOLTR-RS-115 (this iteration): emit `$N` parameter notation
//!   (`column = $1` instead of bare `column = 1`) in both the Optional::Some
//!   arm and the plain-field branch, and in the WHERE clause.
//! - JOLTR-RS-116: validate the parameterized query uses `$1, $2, ...`
//!   bindings exclusively (never string-interpolated values).
//! - JOLTR-RS-117: closing test bundle for phase27.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    parse2, Data, DataStruct, DeriveInput, Fields, GenericArgument, Ident, LitStr, Meta,
    PathArguments, Type,
};

/// Check whether `ty` is `Optional<T>` and, if so, return a reference to the
/// inner `T`.
///
/// JOLTR-RS-112: a field typed `Optional<String>` has `is_optional = true` and
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
/// JOLTR-RS-110 captures the struct identifier and per-field metadata. JOLTR-RS-111
/// adds the struct-level `#[patch("table_name")]` attribute extraction into
/// [`table_name`]. JOLTR-RS-112 added `Optional<T>` field detection.
/// JOLTR-RS-113 graduates the per-field representation from `PatchQueryField` to
/// [`PatchField`], the canonical internal representation consumed by later
/// codegen slices (114+).
#[derive(Debug)]
pub(crate) struct PatchQueryInput {
    pub(crate) ident: Ident,
    pub(crate) fields: Vec<PatchField>,
    pub(crate) table_name: Option<String>,
}

/// Internal representation of one field on a `#[derive(PatchQuery)]` struct.
///
/// JOLTR-RS-113: the canonical IR carrying everything codegen (114+) needs:
///
/// - [`name`]: the Rust field ident, preserved for `quote!` interpolation.
/// - [`column_name`]: the SQL column name. Initially the snake_case rendering
///   of `name`; a future `#[patch(column = "...")]` field-level attribute will
///   override this independently.
/// - [`is_optional`]: `true` when the field's type is `Optional<T>` (tri-state
///   enum), `false` otherwise. Classified by [`optional_inner_type`].
/// - [`inner_type`]: when `is_optional` is `true`, the inner `T` of
///   `Optional<T>`; `None` for plain fields.
#[derive(Debug, Clone)]
pub(crate) struct PatchField {
    pub(crate) name: Ident,
    pub(crate) column_name: String,
    pub(crate) is_optional: bool,
    #[allow(dead_code)]
    pub(crate) inner_type: Option<Type>,
}

/// Parse a `DeriveInput` into [`PatchQueryInput`].
///
/// Acceptance rules (mirror [`crate::auto_middleware::parse_auto_middleware_input`]'s
/// decision set from JOLTR-RS-046):
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
/// JOLTR-RS-111: walks the DeriveInput's attrs for one matching
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
) -> syn::Result<Vec<PatchField>> {
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
                let column_name = field_ident.to_string();
                out.push(PatchField {
                    name: field_ident,
                    column_name,
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
/// Parses via [`parse_patch_query_input`] and emits:
/// 1. Hidden marker consts (`__JOLTR_PATCH_QUERY_FIELD_COUNT`,
///    `__JOLTR_PATCH_QUERY_OPTIONAL_COUNT`, `__JOLTR_PATCH_QUERY_TABLE_NAME`).
/// 2. (JOLTR-RS-114+) The `fn to_patch_query(&self, id_column, id_value) ->
///    (String, Vec<&dyn ToSql>)` method that builds a parameterized SET clause.
///
/// On parse failure the emission is a single `compile_error!` token (with the
/// span the parser attached) — no marker impl, no partial codegen. Mirrors
/// [`crate::auto_middleware::expand_auto_middleware`]'s contract from
/// JOLTR-RS-046.
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
            pub const __JOLTR_PATCH_QUERY_TABLE_NAME: &'static str = #t;
        },
        None => {
            quote! {
                #[doc(hidden)]
                pub const __JOLTR_PATCH_QUERY_TABLE_NAME: ::core::option::Option<&'static str> = ::core::option::Option::None;
            }
        }
    };

    let to_patch_query_method = generate_to_patch_query(&parsed);

    quote! {
        #[automatically_derived]
        impl #ident {
            #[doc(hidden)]
            pub const __JOLTR_PATCH_QUERY_FIELD_COUNT: usize = #field_count;
            #[doc(hidden)]
            pub const __JOLTR_PATCH_QUERY_OPTIONAL_COUNT: usize = #optional_count;
            #table_name_const

            #to_patch_query_method
        }
    }
}

/// Generate the `fn to_patch_query(…) -> (String, Vec<&dyn ToSql>)` method
/// for JOLTR-RS-114.
///
/// For each field in the struct:
/// - If `Optional<T>`: generate a `match &self.{name}` with three arms:
///   `Some(val)` → push `"column = $N"` + `val` as param,
///   `Null` → push `"column = NULL"`,
///   `NotProvided` → skip.
/// - If plain (non-Optional): always include the field in the SET clause.
///
/// The WHERE clause uses `id_column` and `id_value` as the final parameter.
fn generate_to_patch_query(input: &PatchQueryInput) -> TokenStream {
    let table_name = match &input.table_name {
        Some(t) => t.as_str(),
        None => {
            // No #[patch("table")] attribute → skip emitting to_patch_query.
            // The marker consts (__JOLTR_PATCH_QUERY_TABLE_NAME = None) still
            // land so the integration test can observe the absence.
            return TokenStream::new();
        }
    };

    let mut field_arms: Vec<TokenStream> = Vec::with_capacity(input.fields.len());

    for field in &input.fields {
        let field_ident = &field.name;
        let col_name = &field.column_name;

        if field.is_optional {
            field_arms.push(quote! {
                match &self.#field_ident {
                    ::joltr_core::Optional::Some(val) => {
                        sets.push(format!("{} = ${}", #col_name, param_idx));
                        params.push(val);
                        param_idx += 1usize;
                    }
                    ::joltr_core::Optional::Null => {
                        sets.push(format!("{} = NULL", #col_name));
                    }
                    ::joltr_core::Optional::NotProvided => {}
                }
            });
        } else {
            field_arms.push(quote! {
                {
                    sets.push(format!("{} = ${}", #col_name, param_idx));
                    params.push(&self.#field_ident);
                    param_idx += 1usize;
                }
            });
        }
    }

    quote! {
        pub fn to_patch_query<'j>(
            &'j self,
            id_column: &str,
            id_value: &'j impl ::joltr_core::ToSql,
        ) -> (String, Vec<&'j dyn ::joltr_core::ToSql>) {
            let mut sets: Vec<String> = Vec::new();
            let mut params: Vec<&'j dyn ::joltr_core::ToSql> = Vec::new();
            let mut param_idx: usize = 1usize;

            #(#field_arms)*

            if sets.is_empty() {
                return (
                    String::from(
                        "UPDATE  SET  — no fields to update"
                    ),
                    params,
                );
            }

            let where_clause = format!(" WHERE {} = ${}", id_column, param_idx);
            params.push(id_value);

            let sql = format!("UPDATE {} SET {}{}", #table_name, sets.join(", "), where_clause);
            (sql, params)
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
        let names: Vec<String> = parsed.fields.iter().map(|f| f.name.to_string()).collect();
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
        let names: Vec<String> = parsed.fields.iter().map(|f| f.name.to_string()).collect();
        assert_eq!(names, vec!["name", "count", "tags", "bio"]);
    }

    #[test]
    fn preserves_field_types_via_inner_type() {
        // 113 graduates to PatchField which stores `inner_type` instead of the
        // raw `ty`. Pin that the inner type of an `Optional<String>` field
        // is preserved as `String`.
        let input = parse_derive(
            r#"
            struct TypePatch {
                title: Optional<String>,
            }
            "#,
        );
        let parsed = parse_patch_query_input(input).expect("parses");
        assert_eq!(parsed.fields.len(), 1);
        let inner = parsed.fields[0].inner_type.as_ref().expect("Optional<T> must have inner_type");
        let rendered = quote! { #inner }.to_string();
        assert!(
            rendered == "String",
            "inner type must be String, got: {rendered}"
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
            out.contains("__JOLTR_PATCH_QUERY_FIELD_COUNT"),
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
        // partial codegen — mirrors JOLTR-RS-046's contract.
        let input = parse_derive("enum Bad { A, B }");
        let out = expand_patch_query(input).to_string();
        assert!(
            out.contains("compile_error"),
            "parse failure must emit compile_error!, got: {out}"
        );
        assert!(
            !out.contains("__JOLTR_PATCH_QUERY_FIELD_COUNT"),
            "no partial codegen on parse failure, got: {out}"
        );
    }

    // ── JOLTR-RS-111: #[patch("table_name")] attribute parsing ──

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
            out.contains("__JOLTR_PATCH_QUERY_TABLE_NAME"),
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
            out.contains("__JOLTR_PATCH_QUERY_TABLE_NAME"),
            "emission must declare the table-name const even when absent, got: {out}"
        );
        assert!(
            out.contains("Option::None") || out.contains("core :: option :: Option :: None"),
            "absent table must emit Option::None, got: {out}"
        );
    }

    // ── JOLTR-RS-112: Optional<T> detection ──

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
                f.name
            );
            assert!(
                f.inner_type.is_none(),
                "plain field {} must have inner_type=None",
                f.name
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
            out.contains("__JOLTR_PATCH_QUERY_OPTIONAL_COUNT"),
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

    // ── JOLTR-RS-113: PatchField internal representation ──

    #[test]
    fn patch_field_name_is_rust_field_ident() {
        let input = parse_derive(
            r#"
            struct UserPatch {
                display_name: Optional<String>,
                email_address: String,
            }
            "#,
        );
        let parsed = parse_patch_query_input(input).expect("parses");
        assert_eq!(parsed.fields.len(), 2);
        assert_eq!(parsed.fields[0].name, "display_name");
        assert_eq!(parsed.fields[1].name, "email_address");
    }

    #[test]
    fn patch_field_column_name_defaults_to_rust_field_ident() {
        let input = parse_derive(
            r#"
            struct SnakePatch {
                first_name: Optional<String>,
                last_name: Optional<String>,
                view_count: u64,
            }
            "#,
        );
        let parsed = parse_patch_query_input(input).expect("parses");
        assert_eq!(parsed.fields.len(), 3);
        assert_eq!(parsed.fields[0].column_name, "first_name");
        assert_eq!(parsed.fields[1].column_name, "last_name");
        assert_eq!(parsed.fields[2].column_name, "view_count");
    }

    #[test]
    fn patch_field_is_optional_true_for_optional_type() {
        let input = parse_derive(
            r#"
            struct OptPatch {
                title: Optional<String>,
                count: u32,
                bio: Optional<String>,
            }
            "#,
        );
        let parsed = parse_patch_query_input(input).expect("parses");
        assert!(parsed.fields[0].is_optional);
        assert!(!parsed.fields[1].is_optional);
        assert!(parsed.fields[2].is_optional);
    }

    #[test]
    fn patch_field_inner_type_is_some_for_optional_some_for_plain_none() {
        let input = parse_derive(
            r#"
            struct InnerPatch {
                name: Optional<String>,
                age: Optional<u32>,
                title: String,
                tags: Vec<String>,
            }
            "#,
        );
        let parsed = parse_patch_query_input(input).expect("parses");
        // Optional<String> → inner_type = Some(String)
        assert!(parsed.fields[0].inner_type.is_some());
        let inner0_ref = parsed.fields[0].inner_type.as_ref().unwrap();
        let inner0_str = quote! { #inner0_ref }.to_string();
        assert_eq!(inner0_str, "String");
        // Optional<u32> → inner_type = Some(u32)
        assert!(parsed.fields[1].inner_type.is_some());
        let inner1_ref = parsed.fields[1].inner_type.as_ref().unwrap();
        let inner1_str = quote! { #inner1_ref }.to_string();
        assert_eq!(inner1_str, "u32");
        // String (plain) → inner_type = None
        assert!(parsed.fields[2].inner_type.is_none());
        // Vec<String> (plain) → inner_type = None
        assert!(parsed.fields[3].inner_type.is_none());
    }

    #[test]
    fn patch_field_ir_vec_built_from_derived_input() {
        // The PRD-mandated verification: "Internal representation built correctly
        // from syn types." Parse a struct with a mix of Optional and plain fields
        // and verify the entire Vec<PatchField> has correct name, column_name,
        // is_optional, and inner_type for every field.
        let input = parse_derive(
            r#"
            struct FullPatch {
                display_name: Optional<String>,
                bio: String,
                view_count: u64,
                tags: Vec<String>,
                avatar_url: Optional<String>,
            }
            "#,
        );
        let parsed = parse_patch_query_input(input).expect("parses");
        assert_eq!(parsed.fields.len(), 5);

        let f = &parsed.fields;
        // display_name: Optional<String>
        assert_eq!(f[0].name, "display_name");
        assert_eq!(f[0].column_name, "display_name");
        assert!(f[0].is_optional);
        assert!(f[0].inner_type.is_some());

        // bio: String
        assert_eq!(f[1].name, "bio");
        assert_eq!(f[1].column_name, "bio");
        assert!(!f[1].is_optional);
        assert!(f[1].inner_type.is_none());

        // view_count: u64
        assert_eq!(f[2].name, "view_count");
        assert_eq!(f[2].column_name, "view_count");
        assert!(!f[2].is_optional);
        assert!(f[2].inner_type.is_none());

        // tags: Vec<String>
        assert_eq!(f[3].name, "tags");
        assert_eq!(f[3].column_name, "tags");
        assert!(!f[3].is_optional);
        assert!(f[3].inner_type.is_none());

        // avatar_url: Optional<String>
        assert_eq!(f[4].name, "avatar_url");
        assert_eq!(f[4].column_name, "avatar_url");
        assert!(f[4].is_optional);
        assert!(f[4].inner_type.is_some());
    }

    // ── JOLTR-RS-114: to_patch_query method codegen ──

    #[test]
    fn expand_emits_to_patch_query_method() {
        let input = with_patch_attr(
            parse_derive(
                r#"
                struct UserPatch {
                    name: Optional<String>,
                    email: Optional<String>,
                }
                "#,
            ),
            "users",
        );
        let out = expand_patch_query(input).to_string();
        assert!(
            out.contains("fn to_patch_query"),
            "emission must contain the to_patch_query method, got: {out}"
        );
        assert!(
            out.contains("id_column") && out.contains("id_value"),
            "method must have id_column and id_value params, got: {out}"
        );
        assert!(
            out.contains("dyn :: joltr_core :: ToSql") || out.contains("dyn ::joltr_core::ToSql"),
            "return type must reference ::joltr_core::ToSql, got: {out}"
        );
    }

    #[test]
    fn expand_to_patch_query_references_table_name() {
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
            out.contains("UPDATE") && out.contains("users"),
            "generated SQL must reference the table name \"users\", got: {out}"
        );
    }

    #[test]
    fn expand_to_patch_query_includes_sets_and_params() {
        let input = with_patch_attr(
            parse_derive(
                r#"
                struct UserPatch {
                    name: Optional<String>,
                    bio: String,
                }
                "#,
            ),
            "users",
        );
        let out = expand_patch_query(input).to_string();
        assert!(
            out.contains("sets . push"),
            "generated method must push SET clause fragments, got: {out}"
        );
        assert!(
            out.contains("params . push"),
            "generated method must push parameters, got: {out}"
        );
    }

    #[test]
    fn expand_to_patch_query_generates_match_for_optional_field() {
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
            out.contains("Optional :: Some"),
            "optional field must match on Optional::Some, got: {out}"
        );
        assert!(
            out.contains("Optional :: Null"),
            "optional field must match on Optional::Null, got: {out}"
        );
        assert!(
            out.contains("Optional :: NotProvided"),
            "optional field must match on Optional::NotProvided, got: {out}"
        );
    }

    #[test]
    fn expand_to_patch_query_handles_plain_field_without_match() {
        // A plain (non-Optional) field should NOT generate a match block — it
        // is always included in the SET clause.
        let input = with_patch_attr(
            parse_derive(
                r#"
                struct UserPatch {
                    bio: String,
                }
                "#,
            ),
            "users",
        );
        let out = expand_patch_query(input).to_string();
        assert!(
            !out.contains("Optional :: Some"),
            "plain field must not generate Optional::Some match, got: {out}"
        );
    }

    #[test]
    fn expand_to_patch_query_not_emitted_without_table_name() {
        let input = parse_derive(
            r#"
            struct NoTable {
                name: Optional<String>,
            }
            "#,
        );
        let out = expand_patch_query(input).to_string();
        assert!(
            !out.contains("fn to_patch_query"),
            "missing #[patch(\"...\")] must not emit to_patch_query, got: {out}"
        );
        assert!(
            out.contains("__JOLTR_PATCH_QUERY_TABLE_NAME"),
            "marker consts must still be emitted, got: {out}"
        );
    }

    #[test]
    fn expand_to_patch_query_builds_where_clause_with_id_column() {
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
            out.contains("WHERE"),
            "generated SQL must contain WHERE clause, got: {out}"
        );
        assert!(
            out.contains("id_column"),
            "WHERE clause must reference id_column, got: {out}"
        );
    }

    #[test]
    fn expand_to_patch_query_empty_struct_return_early() {
        // An empty struct (no fields) has no SET clauses. The method should
        // return early with a placeholder string.
        let input = with_patch_attr(
            parse_derive("struct EmptyPatch;"),
            "empty_table",
        );
        let out = expand_patch_query(input).to_string();
        assert!(
            out.contains("fn to_patch_query"),
            "empty struct must still emit to_patch_query, got: {out}"
        );
        assert!(
            out.contains("is_empty"),
            "empty struct must check for empty sets, got: {out}"
        );
    }

    #[test]
    fn expand_to_patch_query_param_count_starts_at_one() {
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
            out.contains("param_idx : usize = 1usize") || out.contains("param_idx: usize = 1usize"),
            "param_idx must start at 1 for $1-based PostgreSQL bindings, got: {out}"
        );
    }

    #[test]
    fn expand_to_patch_query_param_idx_increments_after_push() {
        let input = with_patch_attr(
            parse_derive(
                r#"
                struct UserPatch {
                    name: Optional<String>,
                    email: Optional<String>,
                }
                "#,
            ),
            "users",
        );
        let out = expand_patch_query(input).to_string();
        assert!(
            out.contains("param_idx += 1usize"),
            "param_idx must increment after each param push, got: {out}"
        );
    }

    // ── JOLTR-RS-116: $N parameter notation + no value interpolation ──

    #[test]
    fn expand_to_patch_query_uses_dollar_n_placeholders_not_value_interpolation() {
        let input = with_patch_attr(
            parse_derive(
                r#"
                struct UserPatch {
                    name: Optional<String>,
                    email: Optional<String>,
                }
                "#,
            ),
            "users",
        );
        let out = expand_patch_query(input).to_string();

        assert!(
            out.contains("= ${"),
            "$N placeholder notation (= ${{param_idx}}) must be used for SET clauses, got: {out}"
        );
        assert!(
            out.contains("params . push"),
            "values must be pushed via params.push, not interpolated into SQL, got: {out}"
        );
        assert!(
            out.contains("UPDATE") && out.contains("users"),
            "generated SQL must reference table name, got: {out}"
        );
    }

    // ── JOLTR-RS-117: closing test bundle ──

    #[test]
    fn expand_to_patch_query_all_optional_emits_null_format_string() {
        let input = with_patch_attr(
            parse_derive(
                r#"
                struct AllOptPatch {
                    title: Optional<String>,
                    desc: Optional<String>,
                    tags: Optional<String>,
                }
                "#,
            ),
            "media",
        );
        let out = expand_patch_query(input).to_string();

        assert!(
            out.contains("\"{} = NULL\""),
            "Null arm must emit '{{}} = NULL' format string, got: {out}"
        );
        assert!(
            out.contains("\"{} = ${}\""),
            "Some arm must emit '{{}} = ${{}}' format string, got: {out}"
        );
        assert!(
            out.contains("Optional :: Some"),
            "must generate match for Optional::Some, got: {out}"
        );
        assert!(
            out.contains("Optional :: Null"),
            "must generate match for Optional::Null, got: {out}"
        );
        assert!(
            out.contains("Optional :: NotProvided"),
            "must generate match for Optional::NotProvided, got: {out}"
        );
    }

    #[test]
    fn expand_to_patch_query_mixed_optional_emits_both_plain_and_match_arms() {
        let input = with_patch_attr(
            parse_derive(
                r#"
                struct MixedOptPatch {
                    name: Optional<String>,
                    views: u64,
                    color: Optional<String>,
                }
                "#,
            ),
            "items",
        );
        let out = expand_patch_query(input).to_string();

        assert!(
            out.contains("Optional :: Some"),
            "optional field must have match arms, got: {out}"
        );
        assert!(
            out.contains("\"{} = ${}\""),
            "both Optional::Some and plain field must use $N notation, got: {out}"
        );
        assert!(
            out.contains("\"{} = NULL\""),
            "Null arm must emit NULL format string, got: {out}"
        );
        let count_optional_match = out.matches("Optional ::").count();
        assert!(
            count_optional_match >= 2,
            "at least 2 optional field match generators expected, got {} in: {out}",
            count_optional_match
        );
    }

    #[test]
    fn expand_to_patch_query_empty_struct_with_table_emits_early_return() {
        let input = with_patch_attr(
            parse_derive("struct EmptyTable;"),
            "logs",
        );
        let out = expand_patch_query(input).to_string();

        assert!(
            out.contains("fn to_patch_query"),
            "empty struct with table must emit to_patch_query, got: {out}"
        );
        assert!(
            out.contains("is_empty"),
            "empty struct must check sets.is_empty(), got: {out}"
        );
        assert!(
            out.contains("UPDATE") && out.contains("logs"),
            "generated code must reference table name 'logs', got: {out}"
        );
    }

    #[test]
    fn expand_to_patch_query_table_name_matches_attribute_value() {
        let input = with_patch_attr(
            parse_derive(
                r#"
                struct CustomerPatch {
                    email: Optional<String>,
                    name: Optional<String>,
                }
                "#,
            ),
            "customers",
        );
        let out = expand_patch_query(input).to_string();

        assert!(
            out.contains("\"customers\""),
            "table name 'customers' from #[patch(\"customers\")] must appear in generated code, got: {out}"
        );
        assert!(
            out.contains("UPDATE") && out.contains("customers"),
            "generated format string must interpolate the table name correctly, got: {out}"
        );
    }
}
