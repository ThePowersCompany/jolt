//! `joltr-types` binary: collects every `#[derive(TsExport)]` registration
//! from the `inventory` link-time registry and writes the rendered TypeScript
//! declarations to `types.d.ts` at the workspace root.
//!
//! ## PRD ladder
//! - **JOLTR-RS-176 (PRD #11, this iteration)** — scaffold the binary crate,
//!   wire up the `inventory::collect!` collection point, render a canonical
//!   header + per-entry body, and write the output file. The placeholder
//!   `TsTypeDef` is intentionally minimal (`name + body` strings) so the
//!   pipeline can be exercised end-to-end before #12 lands the richer model.
//! - **JOLTR-RS-177 (PRD #12)** — move `TsTypeDef` into `joltr-types/src/lib.rs`,
//!   replace the placeholder fields with `name + fields + generics + docs +
//!   kind: Interface|Enum|Union`, implement structured rendering (interface,
//!   union, enum const-object + type), and update the `#[derive(TsExport)]`
//!   proc-macro in `joltr-macros` to `inventory::submit!` an instance per
//!   user-defined type.
//!
//! ## Output path resolution
//! 1. If the `JOLTR_TYPES_OUT` environment variable is set, write to that
//!    path verbatim. Used by integration tests to redirect output into a
//!    temp file so the developer-visible `types.d.ts` is never clobbered
//!    by a `cargo test` run.
//! 2. Otherwise, write to `<workspace_root>/types.d.ts`. The workspace root
//!    is derived from `CARGO_MANIFEST_DIR` (set at compile time to the
//!    `joltr-types` crate directory) by taking its parent.

use std::error::Error;
use std::fs;
use std::path::PathBuf;

/// One TypeScript declaration collected at link time.
///
/// JOLTR-RS-176: deliberately minimal — `name` is informational, `body`
/// carries the entire rendered TS for that type. JOLTR-RS-177 replaces
/// this with a structured `kind + fields + generics + docs` shape and
/// renders inside the binary instead of pre-rendering at the submit site.
#[derive(Debug)]
pub struct TsTypeDef {
    pub name: &'static str,
    pub body: &'static str,
}

inventory::collect!(TsTypeDef);

/// Canonical header written at the top of every generated `types.d.ts`.
///
/// Matches the marker used by the legacy Zig generator so editors and
/// tooling that look for this string keep working across the transition.
const HEADER: &str = "// === DO NOT MODIFY ===\n\
                      //\n\
                      // Auto-generated type definitions\n\
                      //\n\
                      // === DO NOT MODIFY ===\n\n";

/// Render every registered `TsTypeDef` into a single TypeScript document.
///
/// Order: insertion order of `inventory::submit!` calls (link-time stable
/// for a given workspace layout — `cargo` keeps object-file ordering
/// deterministic per build). Entries are separated by a single newline.
pub fn render() -> String {
    let mut out = String::with_capacity(HEADER.len() + 256);
    out.push_str(HEADER);
    for def in inventory::iter::<TsTypeDef> {
        out.push_str(def.body);
        out.push('\n');
    }
    out
}

/// Resolve the output path for the rendered `types.d.ts`.
///
/// Checks `JOLTR_TYPES_OUT` first so tests can sandbox the write; falls
/// back to `<workspace_root>/types.d.ts` derived from this crate's
/// `CARGO_MANIFEST_DIR`.
fn resolve_out_path() -> PathBuf {
    if let Ok(p) = std::env::var("JOLTR_TYPES_OUT") {
        return PathBuf::from(p);
    }
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = PathBuf::from(manifest_dir)
        .parent()
        .unwrap_or_else(|| panic!("CARGO_MANIFEST_DIR ({manifest_dir}) has no parent"))
        .to_path_buf();
    workspace_root.join("types.d.ts")
}

fn main() -> Result<(), Box<dyn Error>> {
    let out_path = resolve_out_path();
    let contents = render();
    fs::write(&out_path, &contents)?;
    println!(
        "joltr-types: wrote {} bytes to {}",
        contents.len(),
        out_path.display()
    );
    Ok(())
}
