//! `#[derive(PatchQuery)]` proc-macro derive — phase26 field parsing.
//!
//! Phase26 ladder:
//! - JOLT-RS-110 (this iteration): parse the struct's named fields and their
//!   types into [`PatchQueryInput`] + [`PatchQueryField`]. The derive emits a
//!   minimal hidden marker (`__JOLT_PATCH_QUERY_FIELD_COUNT: usize`) so an
//!   integration test can verify the derive compiled and parsed the field
//!   count without depending on later codegen.
//! - JOLT-RS-111: parse the struct-level `#[patch("users")]` attribute to
//!   extract the target table name.
//! - JOLT-RS-112: detect `Optional<T>` fields, extract inner `T`, mark
//!   field as tri-state.
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
use syn::{Data, DataStruct, DeriveInput, Fields, Ident, Type};

/// Parsed shape of a `#[derive(PatchQuery)]` input.
///
/// JOLT-RS-110 captures the struct identifier and per-field metadata (name +
/// type, verbatim). Later phase26 slices extend [`PatchQueryField`] with
/// optional-detection state (112) and column-name overrides; this slice keeps
/// the shape minimal so 111-117 have a single place to add fields without
/// breaking the parse entry point.
#[derive(Debug)]
pub(crate) struct PatchQueryInput {
    pub(crate) ident: Ident,
    pub(crate) fields: Vec<PatchQueryField>,
}

/// One field on a `#[derive(PatchQuery)]` struct. Captured verbatim from
/// `syn::Field` — the ident is the column name source (until JOLT-RS-113 lands
/// `#[patch(column = "...")]` overrides) and the type is what later slices
/// inspect to spot `Optional<T>` (112) and the inner `T` (113).
///
/// `#[allow(dead_code)]` on `ident` and `ty` is a JOLT-RS-111/112 cross-link:
/// the lib-side `expand_patch_query` only reads `parsed.fields.len()` at 110,
/// so the per-field metadata is consumed solely by the `#[cfg(test)]` parse
/// witnesses today. 112 will inspect `ty` to detect `Optional<T>` and 114+
/// will splice `ident` into the generated `SET <ident> = $N` clause — at
/// which point this allow gets dropped.
#[derive(Debug, Clone)]
pub(crate) struct PatchQueryField {
    #[allow(dead_code)]
    pub(crate) ident: Ident,
    #[allow(dead_code)]
    pub(crate) ty: Type,
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
    match input.data {
        Data::Struct(s) => {
            let fields = parse_struct_fields(&s, &ident)?;
            Ok(PatchQueryInput { ident, fields })
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
                out.push(PatchQueryField {
                    ident: field_ident,
                    ty: field.ty.clone(),
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
/// carrying `__JOLT_PATCH_QUERY_FIELD_COUNT: usize` so the integration test in
/// `jolt-core/tests/patch_query_derive.rs` can witness that parsing observed
/// the right field count. Later slices (111-117) extend the emission with the
/// `#[patch("table")]` attribute, optional-detection state, and the
/// `to_patch_query` method itself.
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
    quote! {
        #[automatically_derived]
        impl #ident {
            #[doc(hidden)]
            pub const __JOLT_PATCH_QUERY_FIELD_COUNT: usize = #field_count;
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
}
