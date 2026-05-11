//! `joltr-types` binary: thin glue that walks the [`joltr_types`] inventory
//! and writes the rendered `types.d.ts` to disk.
//!
//! All shape + rendering decisions live in [`joltr_types`] (the sibling lib
//! target in this crate) — this entry point only resolves the output path
//! and calls [`joltr_types::render`].
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
    let contents = joltr_types::render();
    fs::write(&out_path, &contents)?;
    println!(
        "joltr-types: wrote {} bytes to {}",
        contents.len(),
        out_path.display()
    );
    Ok(())
}
