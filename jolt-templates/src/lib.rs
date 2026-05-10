//! Handlebars-based templating for the Jolt framework: registry construction,
//! template loading, and render helpers. Modules land in subsequent PRD items.
//!
//! [`TemplateEngine`] (JOLT-RS-106) wraps a [`handlebars::Handlebars`] registry
//! and is constructed from a filesystem directory of `.hbs` templates via
//! [`TemplateEngine::new`]. The render API (JOLT-RS-107), the custom helper
//! registration (JOLT-RS-108), and the closing test bundle (JOLT-RS-109) build
//! on the registry surface this slice exposes.
//!
//! Architectural decisions pinned here for JOLT-RS-107..109 to build on:
//!
//! 1. **`Handlebars<'static>` registry, owned by the engine.** The lifetime
//!    parameter on [`handlebars::Handlebars`] exists so helpers can borrow
//!    outer state; the Jolt framework does not need that flexibility (helpers
//!    register owned closures or `&'static`-functional helpers via 108), so
//!    pinning the registry's lifetime to `'static` keeps [`TemplateEngine`]'s
//!    public type signature free of lifetime noise. Callers that need to
//!    share the engine across threads wrap it in `Arc<TemplateEngine>` rather
//!    than fighting borrow checker lifetimes.
//!
//! 2. **`new` pre-validates the directory exists.** The upstream
//!    [`handlebars::Handlebars::register_templates_directory`] swallows
//!    `walkdir` errors (`filter_map(|e| e.ok())`) and silently returns `Ok(())`
//!    with zero registered templates when the directory does not exist. That
//!    fail-silent behavior would let a misconfigured `template_dir` produce a
//!    runtime "no such template" error at every render call rather than
//!    surface at startup. [`TemplateEngine::new`] front-loads a
//!    [`std::fs::metadata`] check so a missing directory raises an
//!    [`std::io::ErrorKind::NotFound`] (or a non-directory path raises
//!    [`std::io::ErrorKind::InvalidInput`]) at construction time. Callers
//!    that want the lenient behavior can build a raw
//!    [`handlebars::Handlebars`] themselves; the framework's opinionated
//!    constructor errs on the side of fail-fast.
//!
//! 3. **`.hbs` is the only extension loaded.** The default
//!    [`handlebars::DirectorySourceOptions`] uses `tpl_extension = ".hbs"`,
//!    which matches the framework convention pinned in the original Zig
//!    `mustache.zig` and the PRD-106 verification line ("`.hbs` files loaded
//!    from directory"). The slice does not yet expose an extension-override
//!    knob — every Jolt deployment uses `.hbs` — but the registry built here
//!    is reachable via [`TemplateEngine::registry`] for callers that want to
//!    register additional templates with a different extension after
//!    construction.
//!
//! 4. **Typed error variants ([`TemplateInitError`]).** Mirrors the
//!    [`JwtDecodeError`](../../jolt_utils/jwt/enum.JwtDecodeError.html) (072)
//!    and [`MigrationError`](../../jolt_db/enum.MigrationError.html) (100)
//!    convention: dedicated variants for the two error classes
//!    ([`TemplateInitError::Io`] for the pre-validation step,
//!    [`TemplateInitError::Template`] for the upstream registry call) so the
//!    caller can branch on the failure kind without string-matching. Both
//!    wrap the upstream error via `#[from]`-style conversions and the enum
//!    implements [`std::error::Error`] + [`std::fmt::Display`] so it composes
//!    with the framework's eventual top-level error type.
//!
//! 5. **`new` takes `impl AsRef<Path>`, not `&str`.** Mirrors the
//!    [`std::fs`] convention so callers can pass `&str`, `String`,
//!    `&Path`, `PathBuf`, or `&PathBuf` without an explicit conversion at
//!    the call site. JOLT-RS-094's `read_migration_files` (jolt-db) takes
//!    `&str` because it serializes the path into a `MigrationFile.path`
//!    field whose API is `&str`-typed; this constructor has no such
//!    constraint and benefits from the broader signature.

use std::path::Path;

use handlebars::{DirectorySourceOptions, Handlebars};

/// Failure modes for [`TemplateEngine::new`].
#[derive(Debug)]
pub enum TemplateInitError {
    /// The supplied path could not be opened as a directory.
    Io(std::io::Error),
    /// The registry walked the directory but rejected one of the template
    /// files (syntax error, walkdir error, etc.).
    Template(handlebars::TemplateError),
}

impl std::fmt::Display for TemplateInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateInitError::Io(e) => write!(f, "template directory io error: {e}"),
            TemplateInitError::Template(e) => write!(f, "template registration error: {e}"),
        }
    }
}

impl std::error::Error for TemplateInitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TemplateInitError::Io(e) => Some(e),
            TemplateInitError::Template(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for TemplateInitError {
    fn from(e: std::io::Error) -> Self {
        TemplateInitError::Io(e)
    }
}

impl From<handlebars::TemplateError> for TemplateInitError {
    fn from(e: handlebars::TemplateError) -> Self {
        TemplateInitError::Template(e)
    }
}

/// Owns a [`handlebars::Handlebars`] registry preloaded from a `.hbs`
/// template directory. See the module-level docs for the architectural
/// decisions pinned by this type.
pub struct TemplateEngine {
    registry: Handlebars<'static>,
}

impl TemplateEngine {
    /// Build a new engine from a directory of `.hbs` templates.
    ///
    /// Template names are the file path relative to `template_dir` with the
    /// `.hbs` extension stripped and forward-slash separators (e.g.
    /// `views/users/list.hbs` registers as `views/users/list`).
    ///
    /// Returns [`TemplateInitError::Io`] if `template_dir` does not exist or
    /// is not a directory, and [`TemplateInitError::Template`] if any `.hbs`
    /// file under the directory fails to parse.
    pub fn new(template_dir: impl AsRef<Path>) -> Result<Self, TemplateInitError> {
        let dir = template_dir.as_ref();
        let metadata = std::fs::metadata(dir)?;
        if !metadata.is_dir() {
            return Err(TemplateInitError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("template path is not a directory: {}", dir.display()),
            )));
        }
        let mut registry = Handlebars::new();
        registry.register_templates_directory(dir, DirectorySourceOptions::default())?;
        Ok(Self { registry })
    }

    /// Borrow the underlying [`handlebars::Handlebars`] registry. Exposed so
    /// callers can register additional templates, helpers, or partials on top
    /// of the directory-loaded set; the render API (JOLT-RS-107) and helper
    /// registration (JOLT-RS-108) consume this same registry internally.
    pub fn registry(&self) -> &Handlebars<'static> {
        &self.registry
    }

    /// Mutable companion to [`Self::registry`]; needed for the helper
    /// registration that lands in JOLT-RS-108.
    pub fn registry_mut(&mut self) -> &mut Handlebars<'static> {
        &mut self.registry
    }

    /// `true` if a template with the given canonical name is registered.
    pub fn has_template(&self, name: &str) -> bool {
        self.registry.has_template(name)
    }

    /// Canonical names of every template currently registered. Order is not
    /// guaranteed (the underlying storage is a `HashMap`).
    pub fn template_names(&self) -> Vec<String> {
        self.registry.get_templates().keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{TemplateEngine, TemplateInitError};

    /// Self-cleaning temp directory used by the [`TemplateEngine::new`]
    /// tests. Each instance lives in `std::env::temp_dir()` under a
    /// PID + process-local atomic-counter name so concurrent test threads
    /// don't collide, and the directory is removed on `Drop`. Same shape as
    /// the `TestDir` fixture used by `jolt-db`'s `read_migration_files`
    /// tests (JOLT-RS-094) — keeps the convention consistent across crates.
    struct TestDir {
        path: std::path::PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "jolt-templates-106-{}-{}-{}",
                std::process::id(),
                label,
                n,
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir(&path).expect("create test dir");
            Self { path }
        }

        fn write_file(&self, rel: &str, content: &str) {
            let full = self.path.join(rel);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("create parent dir");
            }
            std::fs::write(full, content).expect("write file");
        }

        fn missing_child(&self, name: &str) -> std::path::PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// Compile-time pin: `TemplateEngine::new` resolves to
    /// `Result<Self, TemplateInitError>` and accepts `impl AsRef<Path>`
    /// (decisions 4 and 5). A regression that narrows the parameter to
    /// `&str` or swaps the return type to `Result<Self, std::io::Error>`
    /// breaks this build pin without ever needing a real directory.
    #[test]
    fn new_signature_pins() {
        fn _pin(dir: &std::path::Path) -> Result<TemplateEngine, TemplateInitError> {
            TemplateEngine::new(dir)
        }
        fn _pin_str(dir: &str) -> Result<TemplateEngine, TemplateInitError> {
            TemplateEngine::new(dir)
        }
        fn _pin_owned(dir: std::path::PathBuf) -> Result<TemplateEngine, TemplateInitError> {
            TemplateEngine::new(dir)
        }
    }

    /// PRD-mandated verification for JOLT-RS-106: handlebars registry created
    /// AND `.hbs` files loaded from the directory. Two flat templates land
    /// under canonical names matching their stems.
    #[test]
    fn new_loads_hbs_files_from_directory() {
        let dir = TestDir::new("flat");
        dir.write_file("hello.hbs", "Hello {{name}}");
        dir.write_file("greeting.hbs", "Hi {{name}}!");

        let engine = TemplateEngine::new(&dir.path).expect("engine constructs");

        assert!(engine.has_template("hello"), "hello.hbs registers as 'hello'");
        assert!(
            engine.has_template("greeting"),
            "greeting.hbs registers as 'greeting'"
        );
        let mut names = engine.template_names();
        names.sort();
        assert_eq!(names, vec!["greeting".to_string(), "hello".to_string()]);
    }

    /// Nested templates register with their relative path (forward-slash
    /// joined, extension stripped). Pins the recursive-walk contract the
    /// upstream `register_templates_directory` provides — a regression that
    /// switched to a flat-walk implementation would put `list.hbs` at the
    /// top level under name `list` instead of `users/list`.
    #[test]
    fn new_loads_nested_templates_with_path_prefixed_names() {
        let dir = TestDir::new("nested");
        dir.write_file("users/list.hbs", "{{#each users}}{{name}}{{/each}}");
        dir.write_file("users/profile.hbs", "{{user.name}}");
        dir.write_file("home.hbs", "home");

        let engine = TemplateEngine::new(&dir.path).expect("engine constructs");

        assert!(engine.has_template("users/list"));
        assert!(engine.has_template("users/profile"));
        assert!(engine.has_template("home"));
        assert_eq!(engine.template_names().len(), 3);
    }

    /// Non-`.hbs` files in the directory are ignored (decision 3). README
    /// and `.txt` fixtures are common in template directories and would
    /// otherwise either silently register under a weird name or trip a
    /// parse error if handlebars treated them as templates.
    #[test]
    fn new_ignores_non_hbs_files() {
        let dir = TestDir::new("mixed");
        dir.write_file("page.hbs", "page");
        dir.write_file("README.md", "# docs");
        dir.write_file("notes.txt", "scratch");

        let engine = TemplateEngine::new(&dir.path).expect("engine constructs");

        assert!(engine.has_template("page"));
        assert!(!engine.has_template("README"));
        assert!(!engine.has_template("notes"));
        assert_eq!(engine.template_names(), vec!["page".to_string()]);
    }

    /// Empty directory constructs cleanly with zero registered templates.
    /// Pins that the constructor does NOT require at least one template —
    /// a fresh-deployment scenario where the operator drops files in over
    /// time should not require pre-seeding the directory.
    #[test]
    fn new_on_empty_directory_yields_zero_templates() {
        let dir = TestDir::new("empty");

        let engine = TemplateEngine::new(&dir.path).expect("engine constructs");

        assert!(engine.template_names().is_empty());
    }

    /// Pins decision 2: a missing directory fails at construction time
    /// (NotFound) rather than silently producing a zero-template registry.
    /// Without the pre-validation step the upstream walkdir filter swallows
    /// the io error and `Ok(())` would propagate here.
    #[test]
    fn new_errors_when_directory_does_not_exist() {
        let dir = TestDir::new("missing");
        let absent = dir.missing_child("does-not-exist");
        assert!(!absent.exists(), "test fixture: path must not exist");

        let result = TemplateEngine::new(&absent);

        match result {
            Err(TemplateInitError::Io(e)) => {
                assert_eq!(
                    e.kind(),
                    std::io::ErrorKind::NotFound,
                    "expected NotFound for absent dir, got {e:?}",
                );
            }
            Err(other) => panic!("expected Io(NotFound), got {other:?}"),
            Ok(_) => panic!("expected error for missing dir, got Ok"),
        }
    }

    /// Pins decision 2's non-directory branch: a file at the path surfaces
    /// as `InvalidInput` rather than going through the walkdir loop (which
    /// would also fail, but with a less-actionable error).
    #[test]
    fn new_errors_when_path_is_a_file() {
        let dir = TestDir::new("file-path");
        let file = dir.path.join("a-file.hbs");
        std::fs::write(&file, "Hello {{name}}").expect("write fixture");

        let result = TemplateEngine::new(&file);

        match result {
            Err(TemplateInitError::Io(e)) => {
                assert_eq!(
                    e.kind(),
                    std::io::ErrorKind::InvalidInput,
                    "expected InvalidInput for file path, got {e:?}",
                );
            }
            Err(other) => panic!("expected Io(InvalidInput), got {other:?}"),
            Ok(_) => panic!("expected error for file path, got Ok"),
        }
    }

    /// A `.hbs` file with a syntactically invalid template surfaces as
    /// [`TemplateInitError::Template`] (decision 4). Pins that the upstream
    /// [`handlebars::TemplateError`] is preserved through the conversion
    /// rather than collapsed into an opaque string — the caller can branch
    /// on the variant.
    #[test]
    fn new_errors_on_invalid_template_syntax() {
        let dir = TestDir::new("bad-syntax");
        dir.write_file("good.hbs", "Hello {{name}}");
        // Unclosed block — the parser will reject this with InvalidSyntax /
        // MismatchingClosedHelper, whichever the upstream tokenizer surfaces.
        dir.write_file("bad.hbs", "{{#if cond}} oops");

        let result = TemplateEngine::new(&dir.path);

        match result {
            Err(TemplateInitError::Template(_)) => {}
            Err(other) => panic!("expected Template(...) error, got {other:?}"),
            Ok(_) => panic!("expected error for bad template, got Ok"),
        }
    }

    /// Pins decision 1: `registry()` returns a borrow with `'static`
    /// lifetime parameter. The compile-time annotation here would fail if a
    /// regression changed [`TemplateEngine`] to hold `Handlebars<'a>` for
    /// some shorter lifetime.
    #[test]
    fn registry_accessor_returns_static_lifetime_handlebars() {
        let dir = TestDir::new("registry-accessor");
        dir.write_file("page.hbs", "page");
        let engine = TemplateEngine::new(&dir.path).expect("engine constructs");

        let r: &handlebars::Handlebars<'static> = engine.registry();
        assert!(r.has_template("page"));
    }

    /// `registry_mut()` exposes the registry for downstream slices (the
    /// helper registration in JOLT-RS-108 will register helpers through this
    /// accessor). Pins the mut accessor so the codegen slice doesn't need
    /// to bypass the engine.
    #[test]
    fn registry_mut_accessor_allows_post_construction_template_registration() {
        let dir = TestDir::new("registry-mut");
        let mut engine = TemplateEngine::new(&dir.path).expect("engine constructs");
        engine
            .registry_mut()
            .register_template_string("dynamic", "x={{x}}")
            .expect("register dynamic template");

        assert!(engine.has_template("dynamic"));
    }

    /// Exercises the [`TemplateInitError`] `Display` + `source` impls
    /// without depending on the upstream error type's internals. Pins the
    /// `impl Error` requirement (decision 4): both variants surface their
    /// inner source so a framework-wide error chain (`anyhow`, `eyre`,
    /// `Box<dyn Error>`) walks through them.
    #[test]
    fn template_init_error_io_variant_exposes_source() {
        use std::error::Error;
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
        let err = TemplateInitError::from(io);
        assert!(format!("{err}").contains("nope"));
        assert!(err.source().is_some(), "Io variant should expose source");
    }
}
