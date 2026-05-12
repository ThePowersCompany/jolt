//! End-to-end integration tests for the `joltr-types` crate.
//!
//! Coverage split:
//! - **Binary subprocess tests** (`binary_*`): spawn `cargo`-built
//!   `joltr-types` and assert on what it writes to disk. The joltr-types
//!   binary's own link unit contains no `#[derive(TsExport)]` sites, so
//!   these tests pin the empty-registry contract (header-only output).
//! - **In-process library tests** (`render_*`): call `joltr_types::render()`
//!   directly from this test binary. Because integration tests link the
//!   joltr-types lib AND any `#[derive(TsExport)]` sites in this file, the
//!   inventory is non-empty here — these tests verify the macro → submit →
//!   render integration path end-to-end.
//!
//! ## JOLTR-RS-176 (PRD #11)
//! Added the binary subprocess tests.
//!
//! ## JOLTR-RS-177 (PRD #12)
//! Added the in-process library tests. They are the canonical surface that
//! exercises the full pipeline introduced in #12: `#[derive(TsExport)]` →
//! `::joltr_types::inventory::submit!` → `joltr_types::render()`.

use std::path::PathBuf;
use std::process::Command;

use joltr_macros::TsExport;

// ── In-process derives. These submit into THIS test binary's inventory; the
//    `joltr-types` subprocess binary's inventory is separate and unaffected. ──

/// Verifies struct → interface rendering with primitive + Vec + Option fields.
#[derive(TsExport)]
#[allow(dead_code)]
struct UserExport {
    id: u32,
    name: String,
    tags: Vec<String>,
    nickname: Option<String>,
}

/// Verifies simple-enum → const-object + union rendering.
#[derive(TsExport)]
#[allow(dead_code)]
enum StatusExport {
    Active,
    Inactive,
}

/// Verifies user-defined path references survive instead of collapsing to any.
#[derive(TsExport)]
#[allow(dead_code)]
struct ProfileExport {
    handle: String,
}

#[derive(TsExport)]
#[allow(dead_code)]
struct AccountExport {
    primary_profile: ProfileExport,
    profiles: Vec<ProfileExport>,
    fallback_profile: Option<ProfileExport>,
}

// ── Binary subprocess test helpers ──

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
fn binary_with_no_local_derives_emits_only_the_header() {
    // The `joltr-types` binary's link unit contains no `#[derive(TsExport)]`
    // sites (this test file's derives are linked into a DIFFERENT binary —
    // this integration-test binary, not the subprocess). So spawning the
    // joltr-types binary still produces a header-only document.
    //
    // When a future PRD wires user app crates into the joltr-types binary's
    // dep graph (so its inventory is non-empty), this test will need updating
    // to match the new expected output.
    let exe = env!("CARGO_BIN_EXE_joltr-types");
    let out = tempfile_in_target("empty-link-unit");

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
        "joltr-types binary's own link unit has no derives — body must be empty, got: {after_header:?}"
    );

    let _ = std::fs::remove_file(&out);
}

// ── In-process library tests ──

#[test]
fn render_includes_struct_interface_from_derive() {
    let out = joltr_types::render();

    assert!(
        out.contains("export interface UserExport {"),
        "render must emit `export interface UserExport`, got:\n{out}"
    );
    assert!(
        out.contains("id: number;"),
        "id: u32 must render as `id: number;`, got:\n{out}"
    );
    assert!(
        out.contains("name: string;"),
        "name: String must render as `name: string;`, got:\n{out}"
    );
    assert!(
        out.contains("tags: string[];"),
        "tags: Vec<String> must render as `tags: string[];`, got:\n{out}"
    );
    assert!(
        out.contains("nickname: string | null;"),
        "nickname: Option<String> must render as `nickname: string | null;`, got:\n{out}"
    );
}

#[test]
fn render_includes_enum_const_object_plus_type() {
    let out = joltr_types::render();

    assert!(
        out.contains("export const StatusExport = {"),
        "simple enum must emit `export const StatusExport = {{`, got:\n{out}"
    );
    assert!(
        out.contains(r#"Active: "Active","#),
        "variant must emit `Active: \"Active\",`, got:\n{out}"
    );
    assert!(
        out.contains(r#"Inactive: "Inactive","#),
        "variant must emit `Inactive: \"Inactive\",`, got:\n{out}"
    );
    assert!(
        out.contains("export type StatusExport = typeof StatusExport[keyof typeof StatusExport];"),
        "enum must emit the companion type alias, got:\n{out}"
    );
}

#[test]
fn render_preserves_user_defined_type_references() {
    let out = joltr_types::render();

    assert!(
        out.contains("export interface AccountExport {"),
        "render must emit `export interface AccountExport`, got:\n{out}"
    );
    assert!(
        out.contains("primary_profile: ProfileExport;"),
        "direct user-defined field must render as ProfileExport, got:\n{out}"
    );
    assert!(
        out.contains("profiles: ProfileExport[];"),
        "Vec<ProfileExport> must render as ProfileExport[], got:\n{out}"
    );
    assert!(
        out.contains("fallback_profile: ProfileExport | null;"),
        "Option<ProfileExport> must render as ProfileExport | null, got:\n{out}"
    );
}

#[test]
fn render_starts_with_canonical_header() {
    let out = joltr_types::render();
    assert!(
        out.starts_with("// === DO NOT MODIFY ==="),
        "render must always lead with the header marker, got: {out:?}"
    );
}
