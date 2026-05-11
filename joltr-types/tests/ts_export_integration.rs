//! End-to-end integration tests for the `joltr-types` binary scaffold.
//!
//! JOLTR-RS-176 (PRD #11): exercises the `cargo run -p joltr-types` pipeline
//! without depending on `joltr-macros` having a fully-wired `inventory::submit!`
//! site yet (that arrives with JOLTR-RS-177 / PRD #12). Tests redirect the
//! output via `JOLTR_TYPES_OUT` so a developer's workspace-level `types.d.ts`
//! is never overwritten by `cargo test`.

use std::path::PathBuf;
use std::process::Command;

/// Generate a unique tempfile path inside the workspace's `target/` directory.
///
/// Avoids `std::env::temp_dir()` because some CI sandboxes mount `/tmp`
/// read-only or with `noexec`, and keeps the artifact discoverable when a
/// test fails (the file isn't auto-deleted).
fn tempfile_in_target(suffix: &str) -> PathBuf {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("joltr-types crate has a parent (workspace root)")
        .to_path_buf();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    workspace_root
        .join("target")
        .join(format!("joltr-types-test-{nanos}-{suffix}.d.ts"))
}

#[test]
fn binary_writes_header_to_redirected_output() {
    let exe = env!("CARGO_BIN_EXE_joltr-types");
    let out = tempfile_in_target("header");

    let result = Command::new(exe)
        .env("JOLTR_TYPES_OUT", &out)
        .output()
        .expect("joltr-types binary spawns");

    assert!(
        result.status.success(),
        "binary exit non-zero: status={:?} stderr={}",
        result.status,
        String::from_utf8_lossy(&result.stderr)
    );

    let contents = std::fs::read_to_string(&out)
        .unwrap_or_else(|e| panic!("output file unreadable at {}: {e}", out.display()));

    assert!(
        contents.starts_with("// === DO NOT MODIFY ==="),
        "header must start the file, got: {contents:?}"
    );
    assert!(
        contents.contains("Auto-generated type definitions"),
        "header marker must be present, got: {contents:?}"
    );

    let _ = std::fs::remove_file(&out);
}

#[test]
fn binary_emits_one_entry_per_inventory_submission() {
    // JOLTR-RS-176: pre-#12, no `inventory::submit!` sites exist in any
    // workspace crate, so the rendered document is exactly the header.
    // This test pins that invariant — when #12 wires `#[derive(TsExport)]`
    // into the registry, this test will fail loudly and force an update
    // to match the new expected output, surfacing the integration point.
    let exe = env!("CARGO_BIN_EXE_joltr-types");
    let out = tempfile_in_target("empty-registry");

    let result = Command::new(exe)
        .env("JOLTR_TYPES_OUT", &out)
        .output()
        .expect("joltr-types binary spawns");

    assert!(result.status.success(), "binary exit non-zero");

    let contents = std::fs::read_to_string(&out).expect("output readable");
    let after_header = contents
        .strip_prefix("// === DO NOT MODIFY ===\n//\n// Auto-generated type definitions\n//\n// === DO NOT MODIFY ===\n\n")
        .expect("canonical header prefix must match exactly");

    assert!(
        after_header.is_empty(),
        "no inventory entries are registered yet at PRD #11 — body must be empty, got: {after_header:?}"
    );

    let _ = std::fs::remove_file(&out);
}
