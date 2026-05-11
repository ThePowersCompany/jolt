//! `#[derive(TsExport)]` proc-macro derive — phase39 field parsing + phase40 attribute analysis.
//!
//! Phase39 ladder:
//! - JOLT-RS-165 (this iteration): parse the struct's named fields and their
//!   types into [`TsExportInput`]. The derive emits a hidden marker
//!   (`__JOLT_TS_EXPORT_FIELD_COUNT: usize`) so an integration test can
//!   verify the derive compiled and parsed the field count without depending
//!   on later codegen.
//! - JOLT-RS-166: map Rust types to TypeScript types
//!   (String→string, i32→number, etc.)
//! - JOLT-RS-167: map generics (Option<T>→T|null, Json<T>→T, etc.)
//! - JOLT-RS-168: generate the `export interface StructName { ... }`
//!   TypeScript definition string.
//!
//! Phase40 ladder:
//! - JOLT-RS-169: `#[ts(rename = "newName")]` support on fields.
//! - JOLT-RS-170: `#[ts(flatten)]` field inlining.
//! - JOLT-RS-171: JSDoc comment generation from doc comments.
//! - JOLT-RS-172: closing attribute test bundle.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DataStruct, DeriveInput, Fields, GenericArgument, Ident, PathArguments, Type};

/// Parsed shape of a `#[derive(TsExport)]` input.
///
/// JOLT-RS-165 captures the struct identifier and per-field Rust name + type.
/// Later items add per-field attributes (rename, flatten, doc) and the
/// TypeScript type-mapping engine (166-168).
#[derive(Debug)]
pub(crate) struct TsExportInput {
    pub(crate) ident: Ident,
    pub(crate) fields: Vec<TsExportField>,
}

/// Internal representation of one field on a `#[derive(TsExport)]` struct.
///
/// JOLT-RS-165: carries the Rust-level representation.
/// JOLT-RS-166: carries `ts_type` — the resolved TypeScript type string
///   computed by [`rust_type_to_ts`]. `None` for unsupported/unrecognised
///   Rust types.
/// Later items add:
/// - `ts_name`: overridden TS property name (169: `#[ts(rename = "...")]`)
/// - `flatten`: whether to inline the field's properties (170: `#[ts(flatten)]`)
/// - `doc`: JSDoc string extracted from /// doc comments (171)
#[derive(Debug, Clone)]
pub(crate) struct TsExportField {
    #[allow(dead_code)]
    pub(crate) name: Ident,
    #[allow(dead_code)]
    pub(crate) rust_type: Type,
    pub(crate) ts_type: Option<String>,
}

/// Map a Rust type to its TypeScript equivalent.
///
/// JOLT-RS-166: maps common Rust types to TypeScript type strings.
///
/// - `String` / `str` → `"string"`
/// - `i32` / `i64` / `u32` / `u64` / `f32` / `f64` / `usize` / `isize` → `"number"`
/// - `bool` → `"boolean"`
/// - `Vec<T>` → `"{T_ts}[]"` (recursive mapping of the inner type)
///
/// Returns `None` for types that don't have a direct mapping (user-defined
/// structs, enums, tuples, references, etc.). Those types will either be
/// resolved by a future cross-crate lookup phase or left as opaque
/// references.
///
/// Matching is by identity alone — no path resolution. The compiler
/// type-checks that the user's types are what they claim to be; the
/// derive operates on syntax.
pub(crate) fn rust_type_to_ts(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(type_path) => {
            let segments = &type_path.path.segments;
            let last = segments.last()?;
            let ident_str = last.ident.to_string();

            match ident_str.as_str() {
                "String" | "str" => Some("string".into()),
                "i32" | "i64" | "u32" | "u64" | "f32" | "f64" | "usize" | "isize" => {
                    Some("number".into())
                }
                "bool" => Some("boolean".into()),
                "Vec" => {
                    if let PathArguments::AngleBracketed(args) = &last.arguments {
                        if args.args.len() == 1 {
                            if let GenericArgument::Type(inner) = &args.args[0] {
                                if let Some(inner_ts) = rust_type_to_ts(inner) {
                                    return Some(format!("{inner_ts}[]"));
                                }
                            }
                        }
                    }
                    None
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Parse a `DeriveInput` into [`TsExportInput`].
///
/// Acceptance rules (mirror [`crate::patch_query::parse_patch_query_input`]'s
/// decision set from JOLT-RS-110):
/// - Must be a `struct`. Enums and unions are rejected with a span pointing
///   at the offending keyword.
/// - Named-fields struct → captured field-by-field.
/// - Unit struct → accepted with an empty field list.
/// - Tuple struct → rejected (TS property names come from field idents; a
///   positional struct has no names to export).
pub(crate) fn parse_ts_export_input(
    input: DeriveInput,
) -> syn::Result<TsExportInput> {
    let ident = input.ident.clone();
    match input.data {
        Data::Struct(s) => {
            let fields = parse_struct_fields(&s, &ident)?;
            Ok(TsExportInput { ident, fields })
        }
        Data::Enum(e) => Err(syn::Error::new_spanned(
            e.enum_token,
            "#[derive(TsExport)] can only be applied to structs, not enums",
        )),
        Data::Union(u) => Err(syn::Error::new_spanned(
            u.union_token,
            "#[derive(TsExport)] can only be applied to structs, not unions",
        )),
    }
}

fn parse_struct_fields(
    data: &DataStruct,
    owner: &Ident,
) -> syn::Result<Vec<TsExportField>> {
    match &data.fields {
        Fields::Named(named) => {
            let mut out = Vec::with_capacity(named.named.len());
            for field in &named.named {
                let field_ident = field
                    .ident
                    .clone()
                    .expect("Fields::Named guarantees every field has an ident");
                let ts_type = rust_type_to_ts(&field.ty);
                out.push(TsExportField {
                    name: field_ident,
                    rust_type: field.ty.clone(),
                    ts_type,
                });
            }
            Ok(out)
        }
        Fields::Unit => Ok(Vec::new()),
        Fields::Unnamed(_) => Err(syn::Error::new_spanned(
            owner,
            "#[derive(TsExport)] requires named fields (tuple structs aren't supported; \
             TS property names come from field idents)",
        )),
    }
}

/// Top-level driver for `#[derive(TsExport)]`.
///
/// Parses via [`parse_ts_export_input`] and emits:
/// 1. Hidden marker `__JOLT_TS_EXPORT_FIELD_COUNT: usize` (JOLT-RS-165) so an
///    integration test can observe the derive ran and parsed the correct field
///    count.
/// 2. Hidden marker `__JOLT_TS_EXPORT_MAPPED_FIELD_COUNT: usize` (JOLT-RS-166)
///    for the number of fields with a recognised TypeScript type mapping.
/// 3. One hidden marker `__JOLT_TS_EXPORT_type_<N>` per field (JOLT-RS-166)
///    emitting an `Option<&'static str>` — `Some("ts_type")` for mapped fields,
///    `None` for unrecognised types.
///
/// On parse failure the emission is a single `compile_error!` token — no
/// partial codegen. Mirrors [`crate::auto_middleware::expand_auto_middleware`]'s
/// contract from JOLT-RS-046.
pub(crate) fn expand_ts_export(input: DeriveInput) -> TokenStream {
    let parsed = match parse_ts_export_input(input) {
        Ok(p) => p,
        Err(err) => return err.to_compile_error(),
    };
    let ident = &parsed.ident;
    let field_count = parsed.fields.len();
    let mapped_count = parsed.fields.iter().filter(|f| f.ts_type.is_some()).count();

    let mut field_type_markers = Vec::with_capacity(parsed.fields.len());
    for (i, f) in parsed.fields.iter().enumerate() {
        let const_name = quote::format_ident!("__JOLT_TS_EXPORT_type_{i}");
        match &f.ts_type {
            Some(ts) => {
                field_type_markers.push(quote! {
                    #[doc(hidden)]
                    pub const #const_name: ::core::option::Option<&'static str> = ::core::option::Option::Some(#ts);
                });
            }
            None => {
                field_type_markers.push(quote! {
                    #[doc(hidden)]
                    pub const #const_name: ::core::option::Option<&'static str> = ::core::option::Option::None;
                });
            }
        }
    }

    quote! {
        #[automatically_derived]
        impl #ident {
            #[doc(hidden)]
            pub const __JOLT_TS_EXPORT_FIELD_COUNT: usize = #field_count;

            #[doc(hidden)]
            pub const __JOLT_TS_EXPORT_MAPPED_FIELD_COUNT: usize = #mapped_count;

            #(#field_type_markers)*
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
        let input = parse_derive("struct EmptyExport;");
        let parsed = parse_ts_export_input(input).expect("unit struct parses");
        assert_eq!(parsed.ident, "EmptyExport");
        assert!(parsed.fields.is_empty(), "unit struct has zero fields");
    }

    #[test]
    fn parses_struct_with_named_fields() {
        let input = parse_derive(
            r#"
            struct UserExport {
                name: String,
                age: u32,
                email: Option<String>,
            }
            "#,
        );
        let parsed = parse_ts_export_input(input).expect("named-field struct parses");
        assert_eq!(parsed.ident, "UserExport");
        assert_eq!(parsed.fields.len(), 3);
        let names: Vec<String> = parsed.fields.iter().map(|f| f.name.to_string()).collect();
        assert_eq!(names, vec!["name", "age", "email"]);
    }

    #[test]
    fn preserves_field_types() {
        let input = parse_derive(
            r#"
            struct TypeExport {
                title: String,
                count: i32,
                active: bool,
            }
            "#,
        );
        let parsed = parse_ts_export_input(input).expect("parses");
        assert_eq!(parsed.fields.len(), 3);

        let ty0 = &parsed.fields[0].rust_type;
        let type0 = quote! { #ty0 }.to_string();
        assert_eq!(type0, "String", "field 0 type must be String");

        let ty1 = &parsed.fields[1].rust_type;
        let type1 = quote! { #ty1 }.to_string();
        assert_eq!(type1, "i32", "field 1 type must be i32");

        let ty2 = &parsed.fields[2].rust_type;
        let type2 = quote! { #ty2 }.to_string();
        assert_eq!(type2, "bool", "field 2 type must be bool");
    }

    #[test]
    fn handles_generic_field_types() {
        let input = parse_derive(
            r#"
            struct GenericExport {
                tags: Vec<String>,
                meta: Option<serde_json::Value>,
            }
            "#,
        );
        let parsed = parse_ts_export_input(input).expect("parses");
        assert_eq!(parsed.fields.len(), 2);
        assert_eq!(parsed.fields[0].name, "tags");
        assert_eq!(parsed.fields[1].name, "meta");

        let ty0 = &parsed.fields[0].rust_type;
        let type0 = quote! { #ty0 }.to_string();
        assert!(type0.contains("Vec") && type0.contains("String"), "tags type must contain Vec and String");

        let ty1 = &parsed.fields[1].rust_type;
        let type1 = quote! { #ty1 }.to_string();
        assert!(type1.contains("Option") || type1.contains("Value"), "meta type must contain Option/Value");
    }

    #[test]
    fn rejects_enum() {
        let input = parse_derive("enum Bad { A, B }");
        let err = parse_ts_export_input(input).expect_err("enum must be rejected");
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
        let err = parse_ts_export_input(input).expect_err("union must be rejected");
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
        let input = parse_derive("struct TupleExport(String, u32);");
        let err = parse_ts_export_input(input).expect_err("tuple struct must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("named fields"),
            "diagnostic must mention named fields, got: {msg}"
        );
    }

    #[test]
    fn expand_emits_field_count_marker_const() {
        let input = parse_derive(
            r#"
            struct UserExport {
                name: String,
                age: u32,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLT_TS_EXPORT_FIELD_COUNT"),
            "emission must declare the marker const, got: {out}"
        );
        assert!(
            out.contains("2usize") || out.contains("2 usize") || out.contains(": usize = 2"),
            "marker const must carry the parsed field count (2), got: {out}"
        );
    }

    #[test]
    fn expand_emits_compile_error_on_enum() {
        let input = parse_derive("enum Bad { A, B }");
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("compile_error"),
            "parse failure must emit compile_error!, got: {out}"
        );
        assert!(
            !out.contains("__JOLT_TS_EXPORT_FIELD_COUNT"),
            "no partial codegen on parse failure, got: {out}"
        );
    }

    // ── JOLT-RS-166: type mapping tests ──

    fn parse_type(src: &str) -> Type {
        syn::parse_str::<Type>(src).expect("test input parses as Type")
    }

    #[test]
    fn string_maps_to_string() {
        let ty = parse_type("String");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("string"));
    }

    #[test]
    fn i32_maps_to_number() {
        let ty = parse_type("i32");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("number"));
    }

    #[test]
    fn all_number_types_map_to_number() {
        for rt in ["i32", "i64", "u32", "u64", "f32", "f64", "usize", "isize"] {
            let ty = parse_type(rt);
            assert_eq!(
                rust_type_to_ts(&ty).as_deref(),
                Some("number"),
                "{rt} must map to number"
            );
        }
    }

    #[test]
    fn bool_maps_to_boolean() {
        let ty = parse_type("bool");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("boolean"));
    }

    #[test]
    fn vec_string_maps_to_string_array() {
        let ty = parse_type("Vec<String>");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("string[]"));
    }

    #[test]
    fn vec_i32_maps_to_number_array() {
        let ty = parse_type("Vec<i32>");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("number[]"));
    }

    #[test]
    fn vec_vec_i32_maps_to_nested_array() {
        let ty = parse_type("Vec<Vec<i32>>");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("number[][]"));
    }

    #[test]
    fn unknown_type_returns_none() {
        for rt in ["MyStruct", "DateTime<Utc>", "u8"] {
            let ty = parse_type(rt);
            assert!(
                rust_type_to_ts(&ty).is_none(),
                "{rt} must return None (not a mapped type)"
            );
        }
    }

    #[test]
    fn reference_type_returns_none() {
        let ty = parse_type("&str");
        assert!(rust_type_to_ts(&ty).is_none());
    }

    #[test]
    fn expand_emits_type_markers_for_mapped_fields() {
        let input = parse_derive(
            r#"
            struct TypeExport {
                title: String,
                count: i32,
                active: bool,
                unknown: SomeUserType,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(out.contains("__JOLT_TS_EXPORT_FIELD_COUNT"), "must emit field count");
        assert!(out.contains("__JOLT_TS_EXPORT_MAPPED_FIELD_COUNT"), "must emit mapped count");

        assert!(
            out.contains(r#"__JOLT_TS_EXPORT_type_0 :"#) && out.contains(r#"Some ("string")"#),
            "field 0 (title: String) must map to Some(\"string\"), got: {out}"
        );
        assert!(
            out.contains(r#"__JOLT_TS_EXPORT_type_1 :"#) && out.contains(r#"Some ("number")"#),
            "field 1 (count: i32) must map to Some(\"number\"), got: {out}"
        );
        assert!(
            out.contains(r#"__JOLT_TS_EXPORT_type_2 :"#) && out.contains(r#"Some ("boolean")"#),
            "field 2 (active: bool) must map to Some(\"boolean\"), got: {out}"
        );
        assert!(
            out.contains(r#"__JOLT_TS_EXPORT_type_3 :"#) && out.contains("None"),
            "field 3 (unknown: SomeUserType) must be None, got: {out}"
        );
    }

    #[test]
    fn expand_emits_correct_mapped_count() {
        let input = parse_derive(
            r#"
            struct HalfMapped {
                name: String,
                count: i32,
                blob: UnknownBlob,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        // mapped: 2 (String + i32), unmapped: 1 (UnknownBlob)
        assert!(
            out.contains("__JOLT_TS_EXPORT_MAPPED_FIELD_COUNT : usize = 2"),
            "mapped count must be 2, got: {out}"
        );
    }

    #[test]
    fn expand_marks_all_fields_unmapped_for_unknown_types() {
        let input = parse_derive(
            r#"
            struct AllUnknown {
                a: MyA,
                b: MyB,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLT_TS_EXPORT_MAPPED_FIELD_COUNT : usize = 0"),
            "all fields unmapped so mapped count must be 0, got: {out}"
        );
        // Both field markers should be None
    }

    #[test]
    fn expand_handles_vec_types_in_markers() {
        let input = parse_derive(
            r#"
            struct VecExport {
                tags: Vec<String>,
                counts: Vec<i32>,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains(r#"Some ("string[]")"#),
            "Vec<String> must map to Some(\"string[]\"), got: {out}"
        );
        assert!(
            out.contains(r#"Some ("number[]")"#),
            "Vec<i32> must map to Some(\"number[]\"), got: {out}"
        );
        assert!(
            out.contains("__JOLT_TS_EXPORT_MAPPED_FIELD_COUNT : usize = 2"),
            "both vec fields are mapped, got: {out}"
        );
    }
}
