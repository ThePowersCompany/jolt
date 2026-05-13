//! `joltr-types` library: the link-time TypeScript type registry.
//!
//! ## What lives here
//! - [`TsTypeDef`] — one collected type definition (struct interface, simple-enum
//!   string-union, or future free-form union). The `#[derive(TsExport)]` macro
//!   in `joltr-macros` emits one `::joltr_types::inventory::submit!` block per
//!   user-defined type carrying a `TsTypeDef` literal.
//! - [`inventory::collect!(TsTypeDef)`] — the single workspace-wide collection
//!   point. Defining it once here means every binary that links the
//!   `joltr-types` lib observes the same registry.
//! - [`render`] — walks the registry and produces the full `types.d.ts` body
//!   (header + per-entry TypeScript rendering).
//!
//! ## What does NOT live here
//! The `joltr-types` binary in `src/main.rs` is intentionally thin — it
//! resolves the output path (env-overridable for tests) and writes the
//! `render()` string to disk. All shape + rendering decisions are here so
//! tests can call `joltr_types::render()` directly without spawning the
//! binary.
//!
//! ## Inventory + per-binary scope
//! `inventory::submit!` registrations are scoped to a single linked binary.
//! A user crate that derives `TsExport` only contributes to the registry when
//! that crate is in the link of the binary doing the iteration. For the
//! `joltr-types` binary that means the user wires their app crate into
//! `joltr-types`'s dep graph (a future workflow PRD); for `joltr-types`'s own
//! integration test, the test binary is the link-unit and the derive in the
//! test file is visible because the test crate links itself.

use std::fmt::Write as _;

/// Re-exported so the `#[derive(TsExport)]` macro can emit
/// `::joltr_types::inventory::submit!` without forcing every user crate to
/// add `inventory` as a direct dep. Mirrors the `joltr-core` re-export of
/// `inventory` used by the `#[endpoint]` macro.
pub use inventory;

/// What kind of TypeScript declaration this entry produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsKind {
    /// `export interface Name { … }` — rendered from [`TsTypeDef::fields`],
    /// each field carries its own TS type expression.
    Interface,
    /// `export const Name = { Variant: "Variant", … } as const;` followed by
    /// `export type Name = typeof Name[keyof typeof Name];`. Variants are
    /// listed in [`TsTypeDef::fields`] with `ts_type` empty (each variant
    /// renders as a string literal of its own name).
    Enum,
    /// `export type Name = A | B | C;` — each entry in [`TsTypeDef::fields`]
    /// contributes its `ts_type` as one arm of the union. Reserved for the
    /// tagged-union derive (future PRD).
    Union,
}

/// One field on an [`Interface`](TsKind::Interface) type, or one variant on
/// an [`Enum`](TsKind::Enum) / arm on a [`Union`](TsKind::Union).
#[derive(Debug, Clone, Copy)]
pub struct TsField {
    /// Property name (interface) or variant identifier (enum). For unions
    /// this is informational only — the renderer uses [`Self::ts_type`].
    pub name: &'static str,
    /// Rendered TS type expression (e.g. `"string"`, `"number[]"`,
    /// `"string | null"`). For enum variants this is unused.
    pub ts_type: &'static str,
    /// Optional JSDoc body to emit on the line above this field.
    pub docs: Option<&'static str>,
}

/// One TypeScript declaration registered via `inventory::submit!`.
#[derive(Debug)]
pub struct TsTypeDef {
    pub name: &'static str,
    pub kind: TsKind,
    /// Interface fields, enum variants, or union arms.  Empty for an empty
    /// interface (renders as `export interface Name {}`).
    pub fields: &'static [TsField],
    /// Generic parameter names rendered between `<…>` after the type name.
    /// Empty for non-generic types. (Generic-parameter parsing is deferred —
    /// the derive emits `&[]` until a future PRD lights up support.)
    pub generics: &'static [&'static str],
    /// Optional JSDoc body rendered above the declaration.
    pub docs: Option<&'static str>,
}

inventory::collect!(TsTypeDef);

/// Canonical header written at the top of every generated `types.d.ts`.
///
/// Matches the marker used by the legacy Zig generator so editors and
/// tooling that look for this string keep working across the transition.
pub const HEADER: &str = "// === DO NOT MODIFY ===\n\
                          //\n\
                          // Auto-generated type definitions\n\
                          //\n\
                          // === DO NOT MODIFY ===\n\n";

/// Render every registered [`TsTypeDef`] into a single TypeScript document.
///
/// Entries appear in the order `inventory::iter` yields them — link-order
/// stable for a given build. Each entry is followed by a single blank line.
pub fn render() -> String {
    let mut out = String::with_capacity(HEADER.len() + 256);
    out.push_str(HEADER);
    for def in inventory::iter::<TsTypeDef> {
        render_def(&mut out, def);
        out.push('\n');
    }
    out
}

/// Render a single [`TsTypeDef`] into the provided string buffer. Exposed
/// (rather than crate-private) so tests can pin per-kind formatting without
/// going through the full registry walk.
pub fn render_def(out: &mut String, def: &TsTypeDef) {
    if let Some(docs) = def.docs {
        let _ = writeln!(out, "/** {docs} */");
    }
    match def.kind {
        TsKind::Interface => render_interface(out, def),
        TsKind::Enum => render_enum(out, def),
        TsKind::Union => render_union(out, def),
    }
}

fn write_generics(out: &mut String, generics: &[&'static str]) {
    if generics.is_empty() {
        return;
    }
    out.push('<');
    for (i, g) in generics.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(g);
    }
    out.push('>');
}

fn render_interface(out: &mut String, def: &TsTypeDef) {
    let _ = write!(out, "export interface {}", def.name);
    write_generics(out, def.generics);
    if def.fields.is_empty() {
        out.push_str(" {}\n");
        return;
    }
    out.push_str(" {\n");
    for field in def.fields {
        if let Some(d) = field.docs {
            let _ = writeln!(out, "  /** {d} */");
        }
        let _ = writeln!(out, "  {}: {};", field.name, field.ts_type);
    }
    out.push_str("}\n");
}

fn render_enum(out: &mut String, def: &TsTypeDef) {
    let _ = writeln!(out, "export const {} = {{", def.name);
    for v in def.fields {
        let _ = writeln!(out, "  {}: \"{}\",", v.name, v.name);
    }
    out.push_str("} as const;\n");
    let _ = writeln!(
        out,
        "export type {0} = typeof {0}[keyof typeof {0}];",
        def.name
    );
}

fn render_union(out: &mut String, def: &TsTypeDef) {
    let _ = write!(out, "export type {}", def.name);
    write_generics(out, def.generics);
    out.push_str(" = ");
    if def.fields.is_empty() {
        out.push_str("never;\n");
        return;
    }
    for (i, arm) in def.fields.iter().enumerate() {
        if i > 0 {
            out.push_str(" | ");
        }
        out.push_str(arm.ts_type);
    }
    out.push_str(";\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_interface_with_two_fields() {
        let def = TsTypeDef {
            name: "User",
            kind: TsKind::Interface,
            fields: &[
                TsField {
                    name: "id",
                    ts_type: "number",
                    docs: None,
                },
                TsField {
                    name: "name",
                    ts_type: "string",
                    docs: None,
                },
            ],
            generics: &[],
            docs: None,
        };
        let mut out = String::new();
        render_def(&mut out, &def);
        assert_eq!(
            out,
            "export interface User {\n  id: number;\n  name: string;\n}\n"
        );
    }

    #[test]
    fn render_interface_emits_jsdoc_for_docs() {
        let def = TsTypeDef {
            name: "Token",
            kind: TsKind::Interface,
            fields: &[TsField {
                name: "value",
                ts_type: "string",
                docs: Some("Opaque bearer token"),
            }],
            generics: &[],
            docs: Some("Authentication payload"),
        };
        let mut out = String::new();
        render_def(&mut out, &def);
        assert!(out.contains("/** Authentication payload */"));
        assert!(out.contains("/** Opaque bearer token */"));
        assert!(out.contains("value: string;"));
    }

    #[test]
    fn render_empty_interface_uses_inline_braces() {
        let def = TsTypeDef {
            name: "Empty",
            kind: TsKind::Interface,
            fields: &[],
            generics: &[],
            docs: None,
        };
        let mut out = String::new();
        render_def(&mut out, &def);
        assert_eq!(out, "export interface Empty {}\n");
    }

    #[test]
    fn render_enum_uses_const_object_plus_type_pattern() {
        let def = TsTypeDef {
            name: "Status",
            kind: TsKind::Enum,
            fields: &[
                TsField {
                    name: "Active",
                    ts_type: "",
                    docs: None,
                },
                TsField {
                    name: "Inactive",
                    ts_type: "",
                    docs: None,
                },
            ],
            generics: &[],
            docs: None,
        };
        let mut out = String::new();
        render_def(&mut out, &def);
        assert_eq!(
            out,
            "export const Status = {\n  Active: \"Active\",\n  Inactive: \"Inactive\",\n} as const;\nexport type Status = typeof Status[keyof typeof Status];\n"
        );
    }

    #[test]
    fn render_union_joins_arms_with_pipes() {
        let def = TsTypeDef {
            name: "Id",
            kind: TsKind::Union,
            fields: &[
                TsField {
                    name: "_0",
                    ts_type: "string",
                    docs: None,
                },
                TsField {
                    name: "_1",
                    ts_type: "number",
                    docs: None,
                },
            ],
            generics: &[],
            docs: None,
        };
        let mut out = String::new();
        render_def(&mut out, &def);
        assert_eq!(out, "export type Id = string | number;\n");
    }

    #[test]
    fn render_interface_with_generics_emits_angle_brackets() {
        let def = TsTypeDef {
            name: "Wrapper",
            kind: TsKind::Interface,
            fields: &[TsField {
                name: "inner",
                ts_type: "T",
                docs: None,
            }],
            generics: &["T"],
            docs: None,
        };
        let mut out = String::new();
        render_def(&mut out, &def);
        assert_eq!(out, "export interface Wrapper<T> {\n  inner: T;\n}\n");
    }

    #[test]
    fn render_full_document_starts_with_canonical_header() {
        // Smoke: even with no submissions, the rendered output begins with
        // the legacy-compatible header marker.
        let out = render();
        assert!(out.starts_with("// === DO NOT MODIFY ==="));
        assert!(out.contains("Auto-generated type definitions"));
    }
}
