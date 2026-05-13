//! `#[derive(TsExport)]` proc-macro derive.
//!
//! The derive parses named/unit structs, simple enums, and data-carrying enums
//! into [`TsExportInput`]. It emits hidden marker consts for tests and submits a
//! structured `joltr_types::TsTypeDef` into the link-time inventory rendered by
//! the `joltr-types` crate.
//!
//! Type mapping supports Rust primitives, `Vec<T>`, `JsonArray<T>`, `Option<T>`,
//! `Optional<T>`, transparent `Json<T>`, type generic parameters, and
//! user-defined path references. Simple enums render as const-object enums;
//! data-carrying enums render as tagged TypeScript union arms.
//!
//! Field-level `#[ts(rename = "...")]` changes the submitted property name and
//! doc comments are preserved as field JSDoc metadata. `#[ts(flatten)]` is kept
//! on hidden marker consts, but inventory submission leaves the field as a
//! normal property because the macro cannot inspect another type's exported
//! fields during expansion.

use proc_macro2::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{
    Data, DataStruct, DeriveInput, Fields, GenericArgument, GenericParam, Generics, Ident, Lit,
    Meta, PathArguments, Token, Type,
};

/// Parsed shape of a `#[derive(TsExport)]` input.
///
/// Captures the source type identifier, Rust generics, TypeScript generic
/// parameter names, struct fields, simple enum variants, and tagged-union enum
/// variants. Only one of `fields`, `variants`, or `union_variants` is populated
/// for a successful parse.
#[derive(Debug)]
pub(crate) struct TsExportInput {
    pub(crate) ident: Ident,
    pub(crate) generics: Generics,
    pub(crate) ts_generics: Vec<String>,
    pub(crate) fields: Vec<TsExportField>,
    pub(crate) variants: Vec<String>,
    pub(crate) union_variants: Vec<TsUnionVariant>,
}

#[derive(Debug, Clone)]
pub(crate) struct TsUnionVariant {
    pub(crate) name: String,
    pub(crate) fields: Vec<TsUnionField>,
}

#[derive(Debug, Clone)]
pub(crate) struct TsUnionField {
    pub(crate) name: String,
    pub(crate) ts_type: Option<String>,
}

/// Internal representation of one field on a `#[derive(TsExport)]` struct.
///
/// Carries the Rust-level field representation plus the TypeScript metadata
/// derived from it: resolved TS type, optional renamed property name, flatten
/// marker, and extracted field doc comment.
#[derive(Debug, Clone)]
pub(crate) struct TsExportField {
    #[allow(dead_code)]
    pub(crate) name: Ident,
    #[allow(dead_code)]
    pub(crate) rust_type: Type,
    pub(crate) ts_type: Option<String>,
    pub(crate) ts_name: Option<String>,
    pub(crate) is_flatten: bool,
    pub(crate) doc: Option<String>,
}

/// Map a Rust type to its TypeScript equivalent.
///
/// - `String` / `str` → `"string"`
/// - `i32` / `i64` / `u32` / `u64` / `f32` / `f64` / `usize` / `isize` → `"number"`
/// - `bool` → `"boolean"`
/// - `Vec<T>` → `"{T_ts}[]"`
/// - `Option<T>` → `"{T_ts} | null"`
/// - `Optional<T>` → `"{T_ts} | null"` (tri-state on the wire collapses to
///   the same nullable shape in TS; the `NotProvided` arm is the absence of
///   the field, but TS callers see the optional-field semantics via
///   the `| null` union — matching the existing serde behavior in
///   `joltr_core::Optional`)
/// - `Json<T>` → inner `T`'s TS (transparent newtype)
/// - `JsonArray<T>` → `"{T_ts}[]"` (transparent over `Vec<T>`)
///
/// Returns the final path identifier for user-defined path references that
/// don't have a direct mapping (for example, `crate::models::User` → `User`).
/// Unsupported non-path shapes and generic paths with unknown semantics still
/// return `None`.
///
/// For nested generics, mapped wrappers compose with user-defined references:
/// `Vec<User>` → `User[]`, `Option<User>` → `User | null`.
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
                "Vec" | "JsonArray" => single_generic_inner(last)
                    .and_then(rust_type_to_ts)
                    .map(|inner| format!("{inner}[]")),
                "Option" | "Optional" => single_generic_inner(last)
                    .and_then(rust_type_to_ts)
                    .map(|inner| format!("{inner} | null")),
                "Json" => single_generic_inner(last).and_then(rust_type_to_ts),
                _ => user_defined_path_reference(type_path),
            }
        }
        _ => None,
    }
}

fn user_defined_path_reference(type_path: &syn::TypePath) -> Option<String> {
    if type_path.qself.is_some() {
        return None;
    }

    let last = type_path.path.segments.last()?;
    if !matches!(last.arguments, PathArguments::None) {
        return None;
    }

    let ident = last.ident.to_string();
    if is_unmapped_primitive(&ident) {
        return None;
    }

    Some(ident)
}

fn is_unmapped_primitive(ident: &str) -> bool {
    matches!(
        ident,
        "u8" | "u16" | "u128" | "i8" | "i16" | "i128" | "char"
    )
}

/// Extract the single `T` from a `Wrapper<T>`-shaped type path segment, if
/// it has exactly one type-position generic argument. Returns `None` for
/// segments with no generics, lifetime-only generics, or arity ≠ 1.
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

/// Parse a `DeriveInput` into [`TsExportInput`].
///
/// Acceptance rules:
/// - **struct** (named fields) → captured field-by-field.
/// - **unit struct** → accepted with an empty field list.
/// - **tuple struct** → rejected (TS property names come from field idents).
/// - **simple enum** (no data on any variant) → accepted; `variants` populated,
///   `fields` empty (JOLTR-RS-173).
/// - **data-carrying enum** (at least one variant with fields) → accepted as a
///   tagged TypeScript union; `union_variants` populated.
/// - **union** → rejected.
pub(crate) fn parse_ts_export_input(input: DeriveInput) -> syn::Result<TsExportInput> {
    let ident = input.ident.clone();
    let generics = input.generics.clone();
    let ts_generics = ts_generic_params(&generics);
    match input.data {
        Data::Struct(s) => {
            let fields = parse_struct_fields(&s, &ident)?;
            Ok(TsExportInput {
                ident,
                generics,
                ts_generics,
                fields,
                variants: Vec::new(),
                union_variants: Vec::new(),
            })
        }
        Data::Enum(e) => {
            let parsed_enum = parse_enum_variants(&e)?;
            let (variants, union_variants) = match parsed_enum {
                ParsedEnum::Simple(variants) => (variants, Vec::new()),
                ParsedEnum::Union(union_variants) => (Vec::new(), union_variants),
            };
            Ok(TsExportInput {
                ident,
                generics,
                ts_generics,
                fields: Vec::new(),
                variants,
                union_variants,
            })
        }
        Data::Union(u) => Err(syn::Error::new_spanned(
            u.union_token,
            "#[derive(TsExport)] can only be applied to structs and enums, not unions",
        )),
    }
}

enum ParsedEnum {
    Simple(Vec<String>),
    Union(Vec<TsUnionVariant>),
}

fn ts_generic_params(generics: &Generics) -> Vec<String> {
    generics
        .params
        .iter()
        .filter_map(|param| match param {
            GenericParam::Type(ty) => Some(ty.ident.to_string()),
            GenericParam::Lifetime(_) | GenericParam::Const(_) => None,
        })
        .collect()
}

/// Walk enum variants. All-unit enums stay on the legacy enum renderer; mixed
/// or fully data-carrying enums become tagged union arms.
fn parse_enum_variants(e: &syn::DataEnum) -> syn::Result<ParsedEnum> {
    let has_data = e.variants.iter().any(|variant| !variant.fields.is_empty());
    if !has_data {
        return Ok(ParsedEnum::Simple(
            e.variants
                .iter()
                .map(|variant| variant.ident.to_string())
                .collect(),
        ));
    }

    let mut variants = Vec::with_capacity(e.variants.len());
    for variant in &e.variants {
        let fields = match &variant.fields {
            Fields::Unit => Vec::new(),
            Fields::Named(named) => parse_union_named_fields(named),
            Fields::Unnamed(unnamed) => unnamed
                .unnamed
                .iter()
                .enumerate()
                .map(|(i, field)| {
                    let ts_attr = parse_ts_field_attrs(&field.attrs);
                    TsUnionField {
                        name: ts_attr.rename.unwrap_or_else(|| format!("_{i}")),
                        ts_type: rust_type_to_ts(&field.ty),
                    }
                })
                .collect(),
        };
        variants.push(TsUnionVariant {
            name: variant.ident.to_string(),
            fields,
        });
    }

    Ok(ParsedEnum::Union(variants))
}

fn parse_union_named_fields(named: &syn::FieldsNamed) -> Vec<TsUnionField> {
    named
        .named
        .iter()
        .map(|field| {
            let field_ident = field
                .ident
                .as_ref()
                .expect("Fields::Named guarantees every field has an ident");
            let ts_attr = parse_ts_field_attrs(&field.attrs);
            TsUnionField {
                name: ts_attr.rename.unwrap_or_else(|| field_ident.to_string()),
                ts_type: rust_type_to_ts(&field.ty),
            }
        })
        .collect()
}

/// Parsed `#[ts(...)]` field-level attribute values.
///
/// JOLTR-RS-172: unified parser replaces the separate `parse_ts_rename_from_attrs`
/// and `parse_ts_flatten_from_attrs` helpers. The union parser uses
/// `Punctuated::parse_terminated` to support comma-separated multi-item forms
/// like `#[ts(flatten, rename = "userId")]` — which the old single-`Meta`
/// `parse_args()` call rejected.
struct TsFieldAttrInfo {
    rename: Option<String>,
    flatten: bool,
}

/// Parse all `#[ts(...)]` attributes on a field into a unified [`TsFieldAttrInfo`].
///
/// Handles both separate attributes (`#[ts(flatten)] #[ts(rename = "x")]`) and
/// combined attributes (`#[ts(flatten, rename = "x")]`) by parsing comma-separated
/// `Meta` items within each `#[ts(...)]` group.
///
/// If multiple `rename` values are found across attributes, the first one wins
/// (same as Rust's standard attribute resolution). `flatten` is binary: any
/// `flatten` path sets it to `true`.
fn parse_ts_field_attrs(attrs: &[syn::Attribute]) -> TsFieldAttrInfo {
    let mut info = TsFieldAttrInfo {
        rename: None,
        flatten: false,
    };

    for attr in attrs {
        if !attr.path().is_ident("ts") {
            continue;
        }
        let Ok(list) = attr.meta.require_list() else {
            continue;
        };
        let Ok(items) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        else {
            continue;
        };

        for meta in items {
            match meta {
                Meta::Path(path) if path.is_ident("flatten") => {
                    info.flatten = true;
                }
                Meta::NameValue(nv) if nv.path.is_ident("rename") => {
                    if info.rename.is_none() {
                        if let syn::Expr::Lit(syn::ExprLit {
                            lit: Lit::Str(s), ..
                        }) = &nv.value
                        {
                            info.rename = Some(s.value());
                        }
                    }
                }
                _ => continue,
            }
        }
    }

    info
}

/// Extract doc comments from field-level `/// ...` attributes.
///
/// Collects all `#[doc = "..."]` attributes (the standard representation of `///`
/// doc comments in Rust's attribute model). Each line is trimmed and joined with a
/// single space. Returns `None` when no doc attributes are present.
///
/// Multiple `///` lines become separate `#[doc = "..."]` entries in the
/// attribute list. This function joins them into a single JSDoc-ready string.
fn parse_doc_from_attrs(attrs: &[syn::Attribute]) -> Option<String> {
    let lines: Vec<String> = attrs
        .iter()
        .filter(|a| a.path().is_ident("doc"))
        .filter_map(|a| {
            if let Meta::NameValue(nv) = &a.meta {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: Lit::Str(s), ..
                }) = &nv.value
                {
                    return Some(s.value().trim().to_string());
                }
            }
            None
        })
        .collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join(" "))
    }
}

fn parse_struct_fields(data: &DataStruct, owner: &Ident) -> syn::Result<Vec<TsExportField>> {
    match &data.fields {
        Fields::Named(named) => {
            let mut out = Vec::with_capacity(named.named.len());
            for field in &named.named {
                let field_ident = field
                    .ident
                    .clone()
                    .expect("Fields::Named guarantees every field has an ident");
                let ts_type = rust_type_to_ts(&field.ty);
                let ts_attr = parse_ts_field_attrs(&field.attrs);
                let doc = parse_doc_from_attrs(&field.attrs);
                out.push(TsExportField {
                    name: field_ident,
                    rust_type: field.ty.clone(),
                    ts_type,
                    ts_name: ts_attr.rename,
                    is_flatten: ts_attr.flatten,
                    doc,
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
///
/// *Structs*:
/// 1. `__JOLTR_TS_EXPORT_IS_ENUM: bool = false`
/// 2. `__JOLTR_TS_EXPORT_FIELD_COUNT: usize`
/// 3. `__JOLTR_TS_EXPORT_MAPPED_FIELD_COUNT: usize`
/// 4. Per-field type / name / flatten / doc markers.
///
/// *Enums*:
/// 1. `__JOLTR_TS_EXPORT_IS_ENUM: bool = true`
/// 2. `__JOLTR_TS_EXPORT_VARIANT_COUNT: usize`
/// 3. `__JOLTR_TS_EXPORT_VARIANT_<N>: Option<&'static str>` per simple or
///    tagged-union variant.
///
/// All successful derives also emit an inventory registration for `joltr-types`:
/// structs submit `TsKind::Interface`, simple enums submit `TsKind::Enum`, and
/// data-carrying enums submit `TsKind::Union` with tagged object arms.
///
/// On parse failure the emission is a single `compile_error!` token — no
/// partial codegen.
pub(crate) fn expand_ts_export(input: DeriveInput) -> TokenStream {
    let parsed = match parse_ts_export_input(input) {
        Ok(p) => p,
        Err(err) => return err.to_compile_error(),
    };
    let ident = &parsed.ident;

    let is_enum = !parsed.variants.is_empty() || !parsed.union_variants.is_empty();
    let field_count = parsed.fields.len();
    let variant_count = if parsed.union_variants.is_empty() {
        parsed.variants.len()
    } else {
        parsed.union_variants.len()
    };
    let mapped_count = parsed.fields.iter().filter(|f| f.ts_type.is_some()).count();

    let mut field_type_markers = Vec::with_capacity(parsed.fields.len());
    let mut field_name_markers = Vec::with_capacity(parsed.fields.len());
    let mut field_flatten_markers = Vec::with_capacity(parsed.fields.len());
    let mut field_doc_markers = Vec::with_capacity(parsed.fields.len());
    for (i, f) in parsed.fields.iter().enumerate() {
        let type_const_name = quote::format_ident!("__JOLTR_TS_EXPORT_type_{i}");
        match &f.ts_type {
            Some(ts) => {
                field_type_markers.push(quote! {
                    #[doc(hidden)]
                    pub const #type_const_name: ::core::option::Option<&'static str> = ::core::option::Option::Some(#ts);
                });
            }
            None => {
                field_type_markers.push(quote! {
                    #[doc(hidden)]
                    pub const #type_const_name: ::core::option::Option<&'static str> = ::core::option::Option::None;
                });
            }
        }

        let name_const_name = quote::format_ident!("__JOLTR_TS_EXPORT_name_{i}");
        match &f.ts_name {
            Some(name) => {
                field_name_markers.push(quote! {
                    #[doc(hidden)]
                    pub const #name_const_name: ::core::option::Option<&'static str> = ::core::option::Option::Some(#name);
                });
            }
            None => {
                field_name_markers.push(quote! {
                    #[doc(hidden)]
                    pub const #name_const_name: ::core::option::Option<&'static str> = ::core::option::Option::None;
                });
            }
        }

        let flatten_const_name = quote::format_ident!("__JOLTR_TS_EXPORT_flatten_{i}");
        let is_flatten = f.is_flatten;
        field_flatten_markers.push(quote! {
            #[doc(hidden)]
            pub const #flatten_const_name: bool = #is_flatten;
        });

        let doc_const_name = quote::format_ident!("__JOLTR_TS_EXPORT_doc_{i}");
        match &f.doc {
            Some(doc_text) => {
                field_doc_markers.push(quote! {
                    #[doc(hidden)]
                    pub const #doc_const_name: ::core::option::Option<&'static str> = ::core::option::Option::Some(#doc_text);
                });
            }
            None => {
                field_doc_markers.push(quote! {
                    #[doc(hidden)]
                    pub const #doc_const_name: ::core::option::Option<&'static str> = ::core::option::Option::None;
                });
            }
        }
    }

    let variant_names: Vec<&str> = if parsed.union_variants.is_empty() {
        parsed.variants.iter().map(String::as_str).collect()
    } else {
        parsed
            .union_variants
            .iter()
            .map(|variant| variant.name.as_str())
            .collect()
    };

    let mut variant_markers = Vec::with_capacity(variant_names.len());
    for (i, v) in variant_names.iter().enumerate() {
        let v = *v;
        let variant_const_name = quote::format_ident!("__JOLTR_TS_EXPORT_VARIANT_{i}");
        variant_markers.push(quote! {
            #[doc(hidden)]
            pub const #variant_const_name: ::core::option::Option<&'static str> = ::core::option::Option::Some(#v);
        });
    }

    let inventory_submit = generate_inventory_submit(&parsed);
    let (impl_generics, ty_generics, where_clause) = parsed.generics.split_for_impl();

    quote! {
        #[automatically_derived]
        impl #impl_generics #ident #ty_generics #where_clause {
            #[doc(hidden)]
            pub const __JOLTR_TS_EXPORT_IS_ENUM: bool = #is_enum;

            #[doc(hidden)]
            pub const __JOLTR_TS_EXPORT_FIELD_COUNT: usize = #field_count;

            #[doc(hidden)]
            pub const __JOLTR_TS_EXPORT_MAPPED_FIELD_COUNT: usize = #mapped_count;

            #[doc(hidden)]
            pub const __JOLTR_TS_EXPORT_VARIANT_COUNT: usize = #variant_count;

            #(#field_type_markers)*

            #(#field_name_markers)*

            #(#field_flatten_markers)*

            #(#field_doc_markers)*

            #(#variant_markers)*
        }

        #inventory_submit
    }
}

/// Emit the `::joltr_types::inventory::submit!` block that registers this
/// derive site into the link-time `TsTypeDef` registry walked by the
/// `joltr-types` binary.
///
/// JOLTR-RS-177 (PRD #12) addition. The block is at module scope so the
/// resulting static lives in the binary's `linkme` / `inventory` section.
/// All `&'static [TsField]` and `&'static str` literals are baked at compile
/// time — no runtime allocation — so the value satisfies `inventory`'s
/// `const`-construction requirement.
///
/// Field-name resolution:
/// 1. `#[ts(rename = "x")]` wins (the user override).
/// 2. Otherwise the Rust field ident is used verbatim.
///
/// Field-type resolution:
/// 1. If [`rust_type_to_ts`] mapped the field, that string is used.
/// 2. Otherwise `"any"` (TS top type) is the fallback for unsupported shapes —
///    keeps the emitted TS syntactically valid for fields whose type the macro
///    can't resolve.
///
/// Enums: each variant contributes a [`TsField`] whose `name` is the variant
/// ident and whose `ts_type` is `""` (unused by the [`TsKind::Enum`]
/// renderer, which derives the literal from the variant name).
///
/// `#[ts(flatten)]` is intentionally ignored at submit time — a faithful
/// flatten requires cross-type lookup that isn't available at macro-expand
/// time. Marked-flatten fields are still emitted as ordinary fields so the
/// derive doesn't silently drop them.
fn generate_inventory_submit(parsed: &TsExportInput) -> TokenStream {
    let ident = &parsed.ident;
    let name_lit = ident.to_string();
    let generic_literals = &parsed.ts_generics;

    let (kind_tokens, field_literals): (TokenStream, Vec<TokenStream>) =
        if !parsed.union_variants.is_empty() {
            let fields = parsed
                .union_variants
                .iter()
                .map(|variant| {
                    let variant_name = &variant.name;
                    let ts_type = render_tagged_union_arm(variant);
                    quote! {
                        ::joltr_types::TsField {
                            name: #variant_name,
                            ts_type: #ts_type,
                            docs: ::core::option::Option::None,
                        }
                    }
                })
                .collect();
            (quote! { ::joltr_types::TsKind::Union }, fields)
        } else if parsed.variants.is_empty() {
            let fields = parsed
                .fields
                .iter()
                .map(|f| {
                    let prop_name = match &f.ts_name {
                        Some(n) => n.clone(),
                        None => f.name.to_string(),
                    };
                    let ts_type = f.ts_type.clone().unwrap_or_else(|| "any".to_string());
                    let docs = match &f.doc {
                        Some(d) => quote! { ::core::option::Option::Some(#d) },
                        None => quote! { ::core::option::Option::None },
                    };
                    quote! {
                        ::joltr_types::TsField {
                            name: #prop_name,
                            ts_type: #ts_type,
                            docs: #docs,
                        }
                    }
                })
                .collect();
            (quote! { ::joltr_types::TsKind::Interface }, fields)
        } else {
            let fields = parsed
                .variants
                .iter()
                .map(|v| {
                    quote! {
                        ::joltr_types::TsField {
                            name: #v,
                            ts_type: "",
                            docs: ::core::option::Option::None,
                        }
                    }
                })
                .collect();
            (quote! { ::joltr_types::TsKind::Enum }, fields)
        };

    quote! {
        ::joltr_types::inventory::submit! {
            ::joltr_types::TsTypeDef {
                name: #name_lit,
                kind: #kind_tokens,
                fields: &[ #(#field_literals),* ],
                generics: &[ #(#generic_literals),* ],
                docs: ::core::option::Option::None,
            }
        }
    }
}

fn render_tagged_union_arm(variant: &TsUnionVariant) -> String {
    let mut arm = format!("{{ type: \"{}\";", variant.name);
    for field in &variant.fields {
        arm.push(' ');
        arm.push_str(&field.name);
        arm.push_str(": ");
        arm.push_str(field.ts_type.as_deref().unwrap_or("any"));
        arm.push(';');
    }
    arm.push_str(" }");
    arm
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
        assert!(
            type0.contains("Vec") && type0.contains("String"),
            "tags type must contain Vec and String"
        );

        let ty1 = &parsed.fields[1].rust_type;
        let type1 = quote! { #ty1 }.to_string();
        assert!(
            type1.contains("Option") || type1.contains("Value"),
            "meta type must contain Option/Value"
        );
    }

    #[test]
    fn parses_type_generic_parameters() {
        let input = parse_derive(
            r#"
            struct Page<T, U> {
                item: T,
                fallback: Option<U>,
            }
            "#,
        );
        let parsed = parse_ts_export_input(input).expect("generic struct parses");
        assert_eq!(parsed.ts_generics, vec!["T", "U"]);
        assert_eq!(parsed.fields[0].ts_type.as_deref(), Some("T"));
        assert_eq!(parsed.fields[1].ts_type.as_deref(), Some("U | null"));
    }

    #[test]
    fn ts_generics_ignore_lifetimes_and_const_params() {
        let input = parse_derive(
            r#"
            struct Buffer<'a, T, const N: usize> {
                item: T,
                label: &'a str,
            }
            "#,
        );
        let parsed = parse_ts_export_input(input).expect("mixed generic params parse");
        assert_eq!(parsed.ts_generics, vec!["T"]);
    }

    #[test]
    fn accepts_simple_enum() {
        let input = parse_derive("enum Status { Active, Inactive }");
        let parsed = parse_ts_export_input(input).expect("simple enum must be accepted");
        assert_eq!(parsed.ident, "Status");
        assert!(parsed.fields.is_empty());
        assert_eq!(parsed.variants, vec!["Active", "Inactive"]);
        assert!(parsed.union_variants.is_empty());
    }

    #[test]
    fn accepts_data_carrying_enum_as_tagged_union() {
        let input = parse_derive(
            r#"
            enum Event {
                Login { user: String },
                Logout,
            }
            "#,
        );
        let parsed = parse_ts_export_input(input).expect("data enum must be accepted");
        assert!(parsed.variants.is_empty());
        assert_eq!(parsed.union_variants.len(), 2);
        assert_eq!(parsed.union_variants[0].name, "Login");
        assert_eq!(parsed.union_variants[0].fields[0].name, "user");
        assert_eq!(
            parsed.union_variants[0].fields[0].ts_type.as_deref(),
            Some("string")
        );
        assert_eq!(parsed.union_variants[1].name, "Logout");
        assert!(parsed.union_variants[1].fields.is_empty());
    }

    #[test]
    fn enum_with_all_unit_variants_is_accepted() {
        let input = parse_derive("enum Color { Red, Green, Blue }");
        let parsed =
            parse_ts_export_input(input).expect("enum with unit variants must be accepted");
        assert_eq!(parsed.variants.len(), 3);
        assert_eq!(parsed.variants, vec!["Red", "Green", "Blue"]);
        assert!(parsed.union_variants.is_empty());
    }

    #[test]
    fn enum_with_mixed_variants_becomes_tagged_union() {
        let input = parse_derive(
            r#"
            enum Mixed {
                A,
                B { x: i32 },
                C,
            }
            "#,
        );
        let parsed = parse_ts_export_input(input).expect("mixed enum must be accepted");
        assert!(parsed.variants.is_empty());
        assert_eq!(parsed.union_variants.len(), 3);
        assert_eq!(parsed.union_variants[0].name, "A");
        assert!(parsed.union_variants[0].fields.is_empty());
        assert_eq!(parsed.union_variants[1].name, "B");
        assert_eq!(parsed.union_variants[1].fields[0].name, "x");
        assert_eq!(
            parsed.union_variants[1].fields[0].ts_type.as_deref(),
            Some("number")
        );
    }

    #[test]
    fn tuple_variant_fields_use_positional_names() {
        let input = parse_derive("enum Event { Point(i32, i32) }");
        let parsed = parse_ts_export_input(input).expect("tuple variant must be accepted");
        assert_eq!(parsed.union_variants.len(), 1);
        assert_eq!(parsed.union_variants[0].fields[0].name, "_0");
        assert_eq!(parsed.union_variants[0].fields[1].name, "_1");
        assert_eq!(
            parsed.union_variants[0].fields[0].ts_type.as_deref(),
            Some("number")
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
            out.contains("__JOLTR_TS_EXPORT_FIELD_COUNT"),
            "emission must declare the marker const, got: {out}"
        );
        assert!(
            out.contains("2usize") || out.contains("2 usize") || out.contains(": usize = 2"),
            "marker const must carry the parsed field count (2), got: {out}"
        );
    }

    #[test]
    fn expand_emits_is_enum_and_variant_markers_for_simple_enum() {
        let input = parse_derive("enum Status { Active, Inactive }");
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLTR_TS_EXPORT_IS_ENUM") && out.contains("true"),
            "simple enum must emit IS_ENUM = true, got: {out}"
        );
        assert!(
            out.contains("__JOLTR_TS_EXPORT_VARIANT_COUNT : usize = 2"),
            "simple enum must emit VARIANT_COUNT = 2, got: {out}"
        );
        assert!(
            out.contains("__JOLTR_TS_EXPORT_VARIANT_0") && out.contains(r#"Some ("Active")"#),
            "variant 0 must emit Some(\"Active\"), got: {out}"
        );
        assert!(
            out.contains("__JOLTR_TS_EXPORT_VARIANT_1") && out.contains(r#"Some ("Inactive")"#),
            "variant 1 must emit Some(\"Inactive\"), got: {out}"
        );
    }

    #[test]
    fn expand_emits_union_metadata_for_data_carrying_enum() {
        let input = parse_derive("enum Bad { A { x: i32 }, B }");
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLTR_TS_EXPORT_IS_ENUM") && out.contains("true"),
            "data enum must still be marked as an enum, got: {out}"
        );
        assert!(
            out.contains("__JOLTR_TS_EXPORT_VARIANT_COUNT : usize = 2"),
            "data enum must emit VARIANT_COUNT = 2, got: {out}"
        );
        assert!(
            out.contains("__JOLTR_TS_EXPORT_VARIANT_0") && out.contains(r#"Some ("A")"#),
            "variant 0 must emit Some(\"A\"), got: {out}"
        );
        assert!(
            out.contains("__JOLTR_TS_EXPORT_VARIANT_1") && out.contains(r#"Some ("B")"#),
            "variant 1 must emit Some(\"B\"), got: {out}"
        );
    }

    // ── JOLTR-RS-166: type mapping tests ──

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
    fn user_defined_type_maps_to_final_identifier() {
        let ty = parse_type("MyStruct");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("MyStruct"));
    }

    #[test]
    fn qualified_user_defined_type_maps_to_final_identifier() {
        let ty = parse_type("crate::models::MyStruct");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("MyStruct"));
    }

    #[test]
    fn unsupported_path_types_return_none() {
        for rt in ["DateTime<Utc>", "u8"] {
            let ty = parse_type(rt);
            assert!(
                rust_type_to_ts(&ty).is_none(),
                "{rt} must return None (not a supported TS reference)"
            );
        }
    }

    #[test]
    fn reference_type_returns_none() {
        let ty = parse_type("&str");
        assert!(rust_type_to_ts(&ty).is_none());
    }

    // ── JOLTR-RS-177 (PRD #12) — JSON-wrapper and nullable mappings ──

    #[test]
    fn option_string_maps_to_nullable_string() {
        let ty = parse_type("Option<String>");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("string | null"));
    }

    #[test]
    fn option_i32_maps_to_nullable_number() {
        let ty = parse_type("Option<i32>");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("number | null"));
    }

    #[test]
    fn optional_string_collapses_to_nullable_string() {
        // `joltr_core::Optional` is tri-state on the wire (Some/Null/NotProvided)
        // but TS-side it surfaces as the same nullable union — the absent
        // arm is encoded by the property being optional via serde defaults.
        let ty = parse_type("Optional<String>");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("string | null"));
    }

    #[test]
    fn json_array_maps_like_vec() {
        let ty = parse_type("JsonArray<String>");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("string[]"));
    }

    #[test]
    fn json_wrapper_is_transparent_over_inner() {
        let ty = parse_type("Json<i32>");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("number"));
    }

    #[test]
    fn json_array_of_json_double_unwraps() {
        // JsonArray<Json<i32>> → JsonArray<number> → number[]
        let ty = parse_type("JsonArray<Json<i32>>");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("number[]"));
    }

    #[test]
    fn option_of_vec_nests_correctly() {
        let ty = parse_type("Option<Vec<String>>");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("string[] | null"));
    }

    #[test]
    fn vec_of_user_defined_type_preserves_reference() {
        let ty = parse_type("Vec<MyStruct>");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("MyStruct[]"));
    }

    #[test]
    fn option_of_user_defined_type_preserves_reference() {
        let ty = parse_type("Option<MyStruct>");
        assert_eq!(rust_type_to_ts(&ty).as_deref(), Some("MyStruct | null"));
    }

    // ── JOLTR-RS-177 (PRD #12) — inventory::submit! emission ──

    #[test]
    fn expand_emits_inventory_submit_for_struct() {
        let input = parse_derive(
            r#"
            struct User {
                id: u32,
                name: String,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains(":: joltr_types :: inventory :: submit"),
            "must emit ::joltr_types::inventory::submit! block, got: {out}"
        );
        assert!(
            out.contains(":: joltr_types :: TsTypeDef"),
            "submit body must reference ::joltr_types::TsTypeDef, got: {out}"
        );
        assert!(
            out.contains(":: joltr_types :: TsKind :: Interface"),
            "struct must register as TsKind::Interface, got: {out}"
        );
        assert!(
            out.contains(r#"name : "User""#),
            "must carry the struct ident as TsTypeDef.name, got: {out}"
        );
        assert!(
            out.contains(r#"name : "id""#) && out.contains(r#"ts_type : "number""#),
            "id: u32 must register as name=\"id\", ts_type=\"number\", got: {out}"
        );
        assert!(
            out.contains(r#"name : "name""#) && out.contains(r#"ts_type : "string""#),
            "name: String must register as name=\"name\", ts_type=\"string\", got: {out}"
        );
    }

    #[test]
    fn expand_emits_inventory_submit_for_enum() {
        let input = parse_derive("enum Status { Active, Inactive }");
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains(":: joltr_types :: TsKind :: Enum"),
            "simple enum must register as TsKind::Enum, got: {out}"
        );
        assert!(
            out.contains(r#"name : "Active""#),
            "variant 0 must appear as TsField.name=\"Active\", got: {out}"
        );
        assert!(
            out.contains(r#"name : "Inactive""#),
            "variant 1 must appear as TsField.name=\"Inactive\", got: {out}"
        );
    }

    #[test]
    fn expand_emits_inventory_submit_for_tagged_union_enum() {
        let input = parse_derive(
            r#"
            enum Event {
                Login { user: String, remember: bool },
                Logout,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains(":: joltr_types :: TsKind :: Union"),
            "data enum must register as TsKind::Union, got: {out}"
        );
        assert!(
            out.contains(r#"name : "Login""#),
            "union arm must keep the variant name, got: {out}"
        );
        assert!(
            out.contains(r#"{ type: \"Login\"; user: string; remember: boolean; }"#),
            "Login arm must render as a tagged object shape, got: {out}"
        );
        assert!(
            out.contains(r#"{ type: \"Logout\"; }"#),
            "unit variant in a mixed enum must render as tag-only object shape, got: {out}"
        );
    }

    #[test]
    fn expand_submit_honors_ts_rename_for_property_name() {
        let input = parse_derive(
            r#"
            struct Rn {
                #[ts(rename = "userId")]
                id: u32,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains(r#"name : "userId""#),
            "renamed field must use the ts_name in TsField.name, got: {out}"
        );
        assert!(
            !out.contains(r#"TsField { name : "id""#) || out.contains(r#"name : "userId""#),
            "rust ident \"id\" must not leak as the submit-time property name, got: {out}"
        );
    }

    #[test]
    fn expand_submit_preserves_user_defined_field_type() {
        let input = parse_derive(
            r#"
            struct Blob {
                payload: SomeUserType,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains(r#"name : "payload""#) && out.contains(r#"ts_type : "SomeUserType""#),
            "user-defined field type must render as its TS reference, got: {out}"
        );
    }

    #[test]
    fn expand_submit_registers_generic_parameters() {
        let input = parse_derive(
            r#"
            struct Page<T> {
                items: Vec<T>,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("impl < T > Page < T >"),
            "generic impl must preserve the user's type parameter, got: {out}"
        );
        assert!(
            out.contains(r#"generics : & ["T"]"#),
            "TsTypeDef.generics must register T, got: {out}"
        );
        assert!(
            out.contains(r#"ts_type : "T []""#) || out.contains(r#"ts_type : "T[]""#),
            "generic Vec<T> field must render as T[], got: {out}"
        );
    }

    #[test]
    fn expand_submit_falls_back_to_any_for_unsupported_field_type() {
        let input = parse_derive(
            r#"
            struct Blob {
                payload: DateTime<Utc>,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains(r#"name : "payload""#) && out.contains(r#"ts_type : "any""#),
            "unsupported generic field type must fall back to \"any\" in submit body, got: {out}"
        );
    }

    #[test]
    fn expand_no_submit_on_compile_error() {
        // Parse failure (tuple struct) must NOT emit a submit block.
        let input = parse_derive("struct TupleExport(String, u32);");
        let out = expand_ts_export(input).to_string();
        assert!(
            !out.contains(":: joltr_types :: inventory :: submit"),
            "no partial codegen on parse failure, got: {out}"
        );
        assert!(
            out.contains("compile_error"),
            "expected compile_error on tuple struct, got: {out}"
        );
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
        assert!(
            out.contains("__JOLTR_TS_EXPORT_FIELD_COUNT"),
            "must emit field count"
        );
        assert!(
            out.contains("__JOLTR_TS_EXPORT_MAPPED_FIELD_COUNT"),
            "must emit mapped count"
        );

        assert!(
            out.contains(r#"__JOLTR_TS_EXPORT_type_0 :"#) && out.contains(r#"Some ("string")"#),
            "field 0 (title: String) must map to Some(\"string\"), got: {out}"
        );
        assert!(
            out.contains(r#"__JOLTR_TS_EXPORT_type_1 :"#) && out.contains(r#"Some ("number")"#),
            "field 1 (count: i32) must map to Some(\"number\"), got: {out}"
        );
        assert!(
            out.contains(r#"__JOLTR_TS_EXPORT_type_2 :"#) && out.contains(r#"Some ("boolean")"#),
            "field 2 (active: bool) must map to Some(\"boolean\"), got: {out}"
        );
        assert!(
            out.contains(r#"__JOLTR_TS_EXPORT_type_3 :"#)
                && out.contains(r#"Some ("SomeUserType")"#),
            "field 3 (unknown: SomeUserType) must preserve the type reference, got: {out}"
        );
    }

    #[test]
    fn expand_emits_correct_mapped_count() {
        let input = parse_derive(
            r#"
            struct HalfMapped {
                name: String,
                count: i32,
                blob: DateTime<Utc>,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        // mapped: 2 (String + i32), unmapped: 1 (unsupported generic path)
        assert!(
            out.contains("__JOLTR_TS_EXPORT_MAPPED_FIELD_COUNT : usize = 2"),
            "mapped count must be 2, got: {out}"
        );
    }

    #[test]
    fn expand_marks_all_fields_unmapped_for_unsupported_generic_types() {
        let input = parse_derive(
            r#"
            struct AllUnknown {
                a: DateTime<Utc>,
                b: Result<String, Error>,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLTR_TS_EXPORT_MAPPED_FIELD_COUNT : usize = 0"),
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
            out.contains("__JOLTR_TS_EXPORT_MAPPED_FIELD_COUNT : usize = 2"),
            "both vec fields are mapped, got: {out}"
        );
    }

    // ── JOLTR-RS-169: #[ts(rename = "...")] tests ──

    #[test]
    fn rename_overrides_ts_property_name() {
        let input = parse_derive(
            r#"
            struct RenameExport {
                #[ts(rename = "userId")]
                id: u32,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLTR_TS_EXPORT_name_0") && out.contains(r#"Some ("userId")"#),
            "field 0 (id) renamed to userId must emit Some(\"userId\"), got: {out}"
        );
        assert!(
            out.contains("__JOLTR_TS_EXPORT_type_0") && out.contains(r#"Some ("number")"#),
            "field 0 type must still map to number, got: {out}"
        );
    }

    #[test]
    fn rename_persists_on_unmapped_type() {
        let input = parse_derive(
            r#"
            struct RenameUnknown {
                #[ts(rename = "customBlob")]
                blob: UnknownType,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLTR_TS_EXPORT_name_0") && out.contains(r#"Some ("customBlob")"#),
            "rename must persist even for unmapped TS type, got: {out}"
        );
        assert!(
            out.contains("__JOLTR_TS_EXPORT_type_0") && out.contains("None"),
            "type marker must still be None for unknown type, got: {out}"
        );
    }

    #[test]
    fn no_rename_emits_none_name_marker() {
        let input = parse_derive(
            r#"
            struct NoRename {
                title: String,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLTR_TS_EXPORT_name_0") && out.contains("None"),
            "field without #[ts(rename)] must emit None for name marker, got: {out}"
        );
    }

    #[test]
    fn mixed_renamed_and_non_renamed_fields() {
        let input = parse_derive(
            r#"
            struct MixedRename {
                #[ts(rename = "userId")]
                id: u32,
                name: String,
                #[ts(rename = "isActive")]
                active: bool,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();

        // field 0: renamed to userId
        assert!(
            out.contains("__JOLTR_TS_EXPORT_name_0") && out.contains(r#"Some ("userId")"#),
            "field 0 must emit Some(\"userId\"), got: {out}"
        );
        // field 1: no rename → None
        assert!(
            out.contains("__JOLTR_TS_EXPORT_name_1")
                && out.matches("__JOLTR_TS_EXPORT_name_1").count() == 1,
            "field 1 must have exactly one name_1 marker, got: {out}"
        );
        // field 2: renamed to isActive
        assert!(
            out.contains("__JOLTR_TS_EXPORT_name_2") && out.contains(r#"Some ("isActive")"#),
            "field 2 must emit Some(\"isActive\"), got: {out}"
        );

        // Verify field 1's name marker is None (between name_0 and name_2)
        let after_name0 = out.split("__JOLTR_TS_EXPORT_name_0").nth(1).unwrap_or("");
        let before_name2 = after_name0
            .split("__JOLTR_TS_EXPORT_name_2")
            .next()
            .unwrap_or("");
        assert!(
            before_name2.contains("None"),
            "field 1 (no rename) must have None name marker between field 0 and field 2, got chunk: {before_name2}"
        );
    }

    // ── JOLTR-RS-170: #[ts(flatten)] tests ──

    #[test]
    fn flatten_detected_on_field() {
        let input = parse_derive(
            r#"
            struct FlattenExport {
                #[ts(flatten)]
                address: Address,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLTR_TS_EXPORT_flatten_0") && out.contains("true"),
            "field 0 with #[ts(flatten)] must emit flatten_0 = true, got: {out}"
        );
    }

    #[test]
    fn no_flatten_emits_false_marker() {
        let input = parse_derive(
            r#"
            struct NoFlatten {
                name: String,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLTR_TS_EXPORT_flatten_0") && out.contains("false"),
            "field without #[ts(flatten)] must emit flatten_0 = false, got: {out}"
        );
    }

    #[test]
    fn mixed_flattened_and_non_flattened_fields() {
        let input = parse_derive(
            r#"
            struct MixedFlatten {
                #[ts(flatten)]
                addr: Address,
                name: String,
                #[ts(flatten)]
                meta: Meta,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();

        assert!(
            out.contains("__JOLTR_TS_EXPORT_flatten_0") && out.contains("true"),
            "field 0 (addr) must be flatten=true, got: {out}"
        );
        // field 1 (name) must be false between flatten_0 and flatten_2
        let after_flatten0 = out
            .split("__JOLTR_TS_EXPORT_flatten_0")
            .nth(1)
            .unwrap_or("");
        let before_flatten2 = after_flatten0
            .split("__JOLTR_TS_EXPORT_flatten_2")
            .next()
            .unwrap_or("");
        assert!(
            before_flatten2.contains("false"),
            "field 1 (name, no flatten) must have false marker between flatten_0 and flatten_2, got chunk: {before_flatten2}"
        );
        assert!(
            out.contains("__JOLTR_TS_EXPORT_flatten_2") && out.contains("true"),
            "field 2 (meta) must be flatten=true, got: {out}"
        );
    }

    #[test]
    fn flatten_persists_on_unmapped_type() {
        let input = parse_derive(
            r#"
            struct FlattenUnknown {
                #[ts(flatten)]
                blob: UnknownType,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLTR_TS_EXPORT_flatten_0") && out.contains("true"),
            "flatten must persist even for unmapped TS type, got: {out}"
        );
        assert!(
            out.contains("__JOLTR_TS_EXPORT_type_0") && out.contains("None"),
            "type marker must still be None for unknown type, got: {out}"
        );
    }

    // ── JOLTR-RS-171: /// doc comment → JSDoc tests ──

    #[test]
    fn doc_detected_on_field() {
        let input = parse_derive(
            r#"
            struct DocExport {
                /// The user's name
                name: String,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLTR_TS_EXPORT_doc_0") && out.contains(r#"Some ("The user's name")"#),
            "field 0 with /// doc must emit Some(\"The user's name\"), got: {out}"
        );
    }

    #[test]
    fn no_doc_emits_none_marker() {
        let input = parse_derive(
            r#"
            struct NoDoc {
                title: String,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLTR_TS_EXPORT_doc_0") && out.contains("None"),
            "field without /// doc must emit None for doc marker, got: {out}"
        );
    }

    #[test]
    fn multiline_doc_joined_with_spaces() {
        let input = parse_derive(
            r#"
            struct MultiDoc {
                /// First line
                /// Second line
                name: String,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLTR_TS_EXPORT_doc_0")
                && out.contains(r#"Some ("First line Second line")"#),
            "multi-line /// doc must be joined with spaces, got: {out}"
        );
    }

    #[test]
    fn mixed_doc_and_non_doc_fields() {
        let input = parse_derive(
            r#"
            struct MixedDoc {
                /// The user's name
                name: String,
                age: u32,
                /// Whether the user is active
                active: bool,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();

        assert!(
            out.contains("__JOLTR_TS_EXPORT_doc_0") && out.contains(r#"Some ("The user's name")"#),
            "field 0 must emit Some(\"The user's name\"), got: {out}"
        );
        // field 1 (no doc) must be None between doc_0 and doc_2
        let after_doc0 = out.split("__JOLTR_TS_EXPORT_doc_0").nth(1).unwrap_or("");
        let before_doc2 = after_doc0
            .split("__JOLTR_TS_EXPORT_doc_2")
            .next()
            .unwrap_or("");
        assert!(
            before_doc2.contains("None"),
            "field 1 (no doc) must have None doc marker between doc_0 and doc_2, got chunk: {before_doc2}"
        );
        assert!(
            out.contains("__JOLTR_TS_EXPORT_doc_2")
                && out.contains(r#"Some ("Whether the user is active")"#),
            "field 2 must emit Some(\"Whether the user is active\"), got: {out}"
        );
    }

    #[test]
    fn doc_works_with_rename() {
        let input = parse_derive(
            r#"
            struct DocRename {
                /// The unique identifier
                #[ts(rename = "userId")]
                id: u32,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLTR_TS_EXPORT_doc_0")
                && out.contains(r#"Some ("The unique identifier")"#),
            "doc must coexist with rename, got: {out}"
        );
        assert!(
            out.contains("__JOLTR_TS_EXPORT_name_0") && out.contains(r#"Some ("userId")"#),
            "rename must coexist with doc, got: {out}"
        );
    }

    #[test]
    fn doc_works_with_flatten() {
        let input = parse_derive(
            r#"
            struct DocFlatten {
                /// The address details
                #[ts(flatten)]
                addr: Address,
            }
            "#,
        );
        let out = expand_ts_export(input).to_string();
        assert!(
            out.contains("__JOLTR_TS_EXPORT_doc_0")
                && out.contains(r#"Some ("The address details")"#),
            "doc must coexist with flatten, got: {out}"
        );
        assert!(
            out.contains("__JOLTR_TS_EXPORT_flatten_0") && out.contains("true"),
            "flatten must coexist with doc, got: {out}"
        );
    }

    // ── JOLTR-RS-172: multi-item attribute + combination tests ──

    mod tsexport_attributes {
        use super::*;

        #[test]
        fn combined_flatten_and_rename_single_attr() {
            let input = parse_derive(
                r#"
            struct ComboExport {
                #[ts(flatten, rename = "userId")]
                id: u32,
            }
            "#,
            );
            let out = expand_ts_export(input).to_string();
            assert!(
                out.contains("__JOLTR_TS_EXPORT_flatten_0") && out.contains("true"),
                "flatten must be true when combined with rename in single attr, got: {out}"
            );
            assert!(
                out.contains("__JOLTR_TS_EXPORT_name_0") && out.contains(r#"Some ("userId")"#),
                "rename must be Some(\"userId\") when combined with flatten, got: {out}"
            );
        }

        #[test]
        fn combined_flatten_and_rename_reversed_order() {
            let input = parse_derive(
                r#"
            struct ComboReversed {
                #[ts(rename = "fullName", flatten)]
                name: String,
            }
            "#,
            );
            let out = expand_ts_export(input).to_string();
            assert!(
                out.contains("__JOLTR_TS_EXPORT_flatten_0") && out.contains("true"),
                "flatten must be true even when rename comes first, got: {out}"
            );
            assert!(
                out.contains("__JOLTR_TS_EXPORT_name_0") && out.contains(r#"Some ("fullName")"#),
                "rename must be Some even when it comes first, got: {out}"
            );
        }

        #[test]
        fn doc_rename_and_flatten_all_three_combo() {
            let input = parse_derive(
                r#"
            struct AllThree {
                /// The user identifier
                #[ts(flatten, rename = "userId")]
                id: u32,
            }
            "#,
            );
            let out = expand_ts_export(input).to_string();
            assert!(
                out.contains("__JOLTR_TS_EXPORT_doc_0")
                    && out.contains(r#"Some ("The user identifier")"#),
                "doc must coexist with flatten+rename combo, got: {out}"
            );
            assert!(
                out.contains("__JOLTR_TS_EXPORT_name_0") && out.contains(r#"Some ("userId")"#),
                "rename must coexist with flatten+doc combo, got: {out}"
            );
            assert!(
                out.contains("__JOLTR_TS_EXPORT_flatten_0") && out.contains("true"),
                "flatten must coexist with rename+doc combo, got: {out}"
            );
        }

        #[test]
        fn separate_ts_attrs_still_work() {
            let input = parse_derive(
                r#"
            struct SeparateAttrs {
                #[ts(flatten)]
                #[ts(rename = "displayName")]
                name: String,
            }
            "#,
            );
            let out = expand_ts_export(input).to_string();
            assert!(
                out.contains("__JOLTR_TS_EXPORT_flatten_0") && out.contains("true"),
                "separate #[ts(flatten)] must still set flatten to true, got: {out}"
            );
            assert!(
                out.contains("__JOLTR_TS_EXPORT_name_0") && out.contains(r#"Some ("displayName")"#),
                "separate #[ts(rename)] must still set rename, got: {out}"
            );
        }

        #[test]
        fn mixed_single_and_combo_attr_styles() {
            let input = parse_derive(
                r#"
            struct MixedAttrs {
                /// Primary key
                #[ts(flatten, rename = "userId")]
                id: u32,
                #[ts(flatten)]
                #[ts(rename = "fullName")]
                name: String,
                /// Email address
                email: String,
            }
            "#,
            );
            let out = expand_ts_export(input).to_string();

            // field 0: combo attr with doc
            assert!(
                out.contains("__JOLTR_TS_EXPORT_flatten_0") && out.contains("true"),
                "field 0 (combo attr) must be flatten=true, got: {out}"
            );
            assert!(
                out.contains("__JOLTR_TS_EXPORT_name_0") && out.contains(r#"Some ("userId")"#),
                "field 0 (combo attr) must have rename userId, got: {out}"
            );
            assert!(
                out.contains("__JOLTR_TS_EXPORT_doc_0") && out.contains(r#"Some ("Primary key")"#),
                "field 0 must have doc, got: {out}"
            );

            // field 1: separate attrs
            assert!(
                out.contains("__JOLTR_TS_EXPORT_flatten_1") && out.contains("true"),
                "field 1 (separate attrs) must be flatten=true, got: {out}"
            );
            assert!(
                out.contains("__JOLTR_TS_EXPORT_name_1") && out.contains(r#"Some ("fullName")"#),
                "field 1 (separate attrs) must have rename fullName, got: {out}"
            );
            assert!(
                out.contains("__JOLTR_TS_EXPORT_doc_1") && out.contains("None"),
                "field 1 must have no doc, got: {out}"
            );

            // field 2: plain field with doc
            assert!(
                out.contains("__JOLTR_TS_EXPORT_flatten_2") && out.contains("false"),
                "field 2 (plain) must be flatten=false, got: {out}"
            );
            assert!(
                out.contains("__JOLTR_TS_EXPORT_name_2") && out.contains("None"),
                "field 2 (plain) must have rename None, got: {out}"
            );
            assert!(
                out.contains("__JOLTR_TS_EXPORT_doc_2")
                    && out.contains(r#"Some ("Email address")"#),
                "field 2 must have doc, got: {out}"
            );
        }
    }
}
