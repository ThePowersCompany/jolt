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
//!
//! 6. **`render` pre-checks template registration before delegating.** The
//!    upstream [`handlebars::Handlebars::render`] surfaces an unregistered
//!    template as a [`handlebars::RenderError`] whose
//!    [`handlebars::RenderErrorReason`] is `TemplateNotFound`; callers who
//!    want to map missing templates to a 404 (HTTP) or a fallback render
//!    have to either string-match or `match` on the non-exhaustive upstream
//!    enum. The dedicated [`TemplateRenderError::TemplateNotFound`] variant
//!    front-loads that branch — [`TemplateEngine::render`] checks
//!    [`handlebars::Handlebars::has_template`] first and short-circuits
//!    without calling into the upstream renderer when the name is not
//!    registered. The rest of the render-time failure modes (missing
//!    variable in strict mode, helper failure, serde error, etc.) come
//!    back wrapped in the catch-all [`TemplateRenderError::Render`]
//!    variant, which exposes the upstream error via [`std::error::Error`]
//!    so the JOLT-RS-109 closing-test bundle can pin specific failure
//!    classes without re-implementing the wrapper.
//!
//! 7. **Custom helpers are registered in [`TemplateEngine::new`], not opt-in.**
//!    The PRD-108 helper set (`eq`, `ne`, `gt`, `lt`, `json`) is the
//!    framework default — every Jolt deployment gets the same surface so
//!    templates are portable across services. Of those five, `eq`/`ne`/`gt`/
//!    `lt` are already registered by handlebars 6's
//!    [`Handlebars::new`](handlebars::Handlebars::new) builtin setup (see
//!    `handlebars-6.4.0/src/registry.rs` `setup_builtins` lines 180–186), so
//!    the only NEW helper Jolt registers is `json` — a
//!    [`serde_json::to_string`] wrapper for embedding a value as a JSON
//!    literal inside a template (`<script>const data = {{json payload}};
//!    </script>`). The registration runs once at construction time inside
//!    [`TemplateEngine::new`]: pulling it out into an opt-in
//!    `with_default_helpers()` builder method would create a foot-gun where
//!    a template that references `{{json data}}` renders successfully on
//!    one deployment and fails on another with the same `.hbs` source.
//!    Callers that need to register additional custom helpers reach through
//!    [`TemplateEngine::registry_mut`]; callers that need to remove a
//!    default helper can call
//!    [`handlebars::Handlebars::unregister_helper`] through the same
//!    accessor.

use std::path::Path;

use handlebars::{handlebars_helper, DirectorySourceOptions, Handlebars};
use serde::Serialize;

// `json`: serialize the supplied JSON value to its canonical string form
// (`serde_json::to_string`). Intended for embedding server-side data inside
// `<script>` blocks or other contexts where the consumer needs a parseable
// JSON literal rather than handlebars' default string coercion.
//
// On serialization failure (which `serde_json::Value` → `String` shouldn't
// produce in practice, since every `Value` is by construction serializable)
// the helper falls back to the JSON `null` literal so the template still
// renders. The fallback is unreachable in practice; it exists so the helper
// matches the upstream `handlebars_helper!` signature (which requires an
// expression, not a `Result`) without forcing every template author through
// an `unwrap` panic.
handlebars_helper!(json: |v: Json| serde_json::to_string(v).unwrap_or_else(|_| "null".to_string()));

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

/// Failure modes for [`TemplateEngine::render`].
#[derive(Debug)]
pub enum TemplateRenderError {
    /// No template with the given name is registered with the engine.
    /// Carries the unresolved name so callers can surface it (e.g. as a
    /// 404 body) without re-deriving it from the request.
    TemplateNotFound(String),
    /// The upstream renderer rejected the template/data combination
    /// (missing variable in strict mode, helper failure, serde error,
    /// etc.). The wrapped [`handlebars::RenderError`] is preserved so
    /// callers can downcast via [`std::error::Error::source`] to inspect
    /// the upstream [`handlebars::RenderErrorReason`].
    Render(handlebars::RenderError),
}

impl std::fmt::Display for TemplateRenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateRenderError::TemplateNotFound(name) => {
                write!(f, "template not found: {name}")
            }
            TemplateRenderError::Render(e) => write!(f, "template render error: {e}"),
        }
    }
}

impl std::error::Error for TemplateRenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TemplateRenderError::TemplateNotFound(_) => None,
            TemplateRenderError::Render(e) => Some(e),
        }
    }
}

impl From<handlebars::RenderError> for TemplateRenderError {
    fn from(e: handlebars::RenderError) -> Self {
        TemplateRenderError::Render(e)
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
        register_default_helpers(&mut registry);
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

    /// Render a registered template against the supplied serializable data
    /// and return the produced string.
    ///
    /// Returns [`TemplateRenderError::TemplateNotFound`] if `template` is
    /// not registered (per decision 6, this branch short-circuits before
    /// the upstream renderer is called) and
    /// [`TemplateRenderError::Render`] for any other render-time failure
    /// surfaced by [`handlebars::Handlebars::render`] (missing variable in
    /// strict mode, helper failure, serde error, etc.).
    pub fn render<T: Serialize>(
        &self,
        template: &str,
        data: &T,
    ) -> Result<String, TemplateRenderError> {
        if !self.registry.has_template(template) {
            return Err(TemplateRenderError::TemplateNotFound(template.to_string()));
        }
        Ok(self.registry.render(template, data)?)
    }
}

// Register Jolt's framework-default helper set on a [`Handlebars`] registry
// (decision 7). Currently a single helper (`json`) — the comparison helpers
// (`eq`, `ne`, `gt`, `lt`) are provided by handlebars 6's own
// [`Handlebars::new`] builtin setup and do not need re-registering. Pulled
// out as a free function (not a method on [`TemplateEngine`]) so the same
// helper set can be applied to a bare [`Handlebars`] registry constructed
// outside the framework path (e.g. by integration tests that bypass
// [`TemplateEngine::new`]).
fn register_default_helpers(registry: &mut Handlebars<'static>) {
    registry.register_helper("json", Box::new(json));
}

#[cfg(test)]
mod tests {
    use super::{TemplateEngine, TemplateInitError, TemplateRenderError};

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

    /// Compile-time pin: `TemplateEngine::render` resolves to
    /// `(&self, &str, &T: Serialize) -> Result<String, TemplateRenderError>`.
    /// Exercises three concrete data types (struct via `serde_json::Value`,
    /// owned `String` map, and a unit `()` for templates with no
    /// substitutions) so a regression that narrowed the data parameter
    /// (e.g. to `&serde_json::Value`) breaks the build.
    #[test]
    fn render_signature_pins() {
        fn _pin_value(
            engine: &TemplateEngine,
            data: &serde_json::Value,
        ) -> Result<String, TemplateRenderError> {
            engine.render("page", data)
        }
        fn _pin_unit(engine: &TemplateEngine) -> Result<String, TemplateRenderError> {
            engine.render("page", &())
        }
        fn _pin_owned(
            engine: &TemplateEngine,
            name: String,
        ) -> Result<String, TemplateRenderError> {
            #[derive(serde::Serialize)]
            struct Owned {
                name: String,
            }
            engine.render("page", &Owned { name })
        }
    }

    /// PRD-mandated verification for JOLT-RS-107: render `hello.hbs` (which
    /// registers as `hello`) with `{ "name": "World" }` and assert the
    /// output is exactly `"Hello World"`. This is the reason the slice
    /// exists — a regression that broke variable substitution or template
    /// lookup would fail this test.
    #[test]
    fn render_returns_template_output_with_substituted_variables() {
        let dir = TestDir::new("render-prd");
        dir.write_file("hello.hbs", "Hello {{name}}");
        let engine = TemplateEngine::new(&dir.path).expect("engine constructs");

        let out = engine
            .render("hello", &serde_json::json!({ "name": "World" }))
            .expect("render succeeds");

        assert_eq!(out, "Hello World");
    }

    /// Pins decision 6: rendering an unregistered name returns
    /// [`TemplateRenderError::TemplateNotFound`] with the requested name
    /// preserved verbatim, NOT a wrapped upstream error. Lets HTTP layers
    /// branch on the variant without inspecting the upstream
    /// [`handlebars::RenderErrorReason`].
    #[test]
    fn render_returns_template_not_found_for_unregistered_name() {
        let dir = TestDir::new("render-missing");
        dir.write_file("home.hbs", "home");
        let engine = TemplateEngine::new(&dir.path).expect("engine constructs");

        let result = engine.render("absent", &serde_json::json!({}));

        match result {
            Err(TemplateRenderError::TemplateNotFound(name)) => assert_eq!(name, "absent"),
            Err(other) => panic!("expected TemplateNotFound, got {other:?}"),
            Ok(_) => panic!("expected error for unregistered template, got Ok"),
        }
    }

    /// Nested templates are looked up by their canonical
    /// forward-slash-joined name (matching the registry contract pinned in
    /// `new_loads_nested_templates_with_path_prefixed_names`). Pins that
    /// the render method does not strip, normalize, or otherwise rewrite
    /// the supplied name before the registry lookup.
    #[test]
    fn render_resolves_nested_template_name_verbatim() {
        let dir = TestDir::new("render-nested");
        dir.write_file("users/profile.hbs", "user={{user}}");
        let engine = TemplateEngine::new(&dir.path).expect("engine constructs");

        let out = engine
            .render("users/profile", &serde_json::json!({ "user": "alice" }))
            .expect("render succeeds");

        assert_eq!(out, "user=alice");
    }

    /// Default handlebars configuration is non-strict, so a missing
    /// variable in the data renders as an empty string rather than an
    /// error. Pins this behavior so callers can't accidentally rely on
    /// strict-mode semantics — the JOLT-RS-109 closing-test bundle and
    /// any future strict-mode toggle would have to update this test.
    #[test]
    fn render_lenient_for_missing_variable_in_default_mode() {
        let dir = TestDir::new("render-lenient");
        dir.write_file("greet.hbs", "Hello {{name}}!");
        let engine = TemplateEngine::new(&dir.path).expect("engine constructs");

        let out = engine
            .render("greet", &serde_json::json!({}))
            .expect("render succeeds (non-strict default)");

        assert_eq!(out, "Hello !");
    }

    /// Renders a template registered AFTER construction via
    /// [`TemplateEngine::registry_mut`]. Pins that `render` reads from the
    /// same registry the post-construction registration writes to (a
    /// regression that cloned the registry into a separate render-time
    /// registry would silently fail this test).
    #[test]
    fn render_resolves_template_registered_after_construction() {
        let dir = TestDir::new("render-post-register");
        let mut engine = TemplateEngine::new(&dir.path).expect("engine constructs");
        engine
            .registry_mut()
            .register_template_string("dynamic", "x={{x}}")
            .expect("register dynamic template");

        let out = engine
            .render("dynamic", &serde_json::json!({ "x": 42 }))
            .expect("render succeeds");

        assert_eq!(out, "x=42");
    }

    /// Pins the [`TemplateRenderError`] `Display` + `source` contract,
    /// mirroring `template_init_error_io_variant_exposes_source` for the
    /// init-side enum. The TemplateNotFound variant is sourceless (it
    /// originates inside this crate, not from an upstream cause); the
    /// Render variant exposes the wrapped [`handlebars::RenderError`] so
    /// `anyhow`/`eyre`/`Box<dyn Error>` walks through the chain.
    #[test]
    fn template_render_error_variants_expose_correct_source() {
        use std::error::Error;

        let nf = TemplateRenderError::TemplateNotFound("missing".to_string());
        assert!(format!("{nf}").contains("missing"));
        assert!(
            nf.source().is_none(),
            "TemplateNotFound has no upstream source"
        );

        let upstream: handlebars::RenderError =
            handlebars::RenderErrorReason::TemplateNotFound("x".to_string()).into();
        let wrapped = TemplateRenderError::from(upstream);
        assert!(format!("{wrapped}").contains("template render error"));
        assert!(
            wrapped.source().is_some(),
            "Render variant exposes upstream source"
        );
    }

    /// PRD-mandated verification for JOLT-RS-108: `{{#if (eq status "active")}}`
    /// renders the inner block when `status` is `"active"`. Pins that the
    /// `eq` helper is wired up to a [`TemplateEngine`] constructed via
    /// [`TemplateEngine::new`] (decision 7) without the caller having to
    /// opt in. A regression that stripped `eq` from the default helper set
    /// (or that built the registry via a path that bypasses handlebars'
    /// `setup_builtins`) would fail this test with either a
    /// `HelperNotFound` render error or an empty-string render.
    #[test]
    fn eq_helper_renders_block_when_value_matches_in_if_subexpression() {
        let dir = TestDir::new("eq-helper");
        dir.write_file(
            "status.hbs",
            r#"{{#if (eq status "active")}}active{{/if}}"#,
        );
        let engine = TemplateEngine::new(&dir.path).expect("engine constructs");

        let out = engine
            .render("status", &serde_json::json!({ "status": "active" }))
            .expect("render succeeds");

        assert_eq!(out, "active");
    }

    /// Companion to the PRD-verification test: the `else` branch fires when
    /// the comparison is false. Pins both halves of the `eq` contract so a
    /// regression that hard-wired `eq` to always return `true` (or always
    /// `false`) is caught.
    #[test]
    fn eq_helper_renders_else_block_when_value_does_not_match() {
        let dir = TestDir::new("eq-helper-else");
        dir.write_file(
            "status.hbs",
            r#"{{#if (eq status "active")}}yes{{else}}no{{/if}}"#,
        );
        let engine = TemplateEngine::new(&dir.path).expect("engine constructs");

        let out = engine
            .render("status", &serde_json::json!({ "status": "inactive" }))
            .expect("render succeeds");

        assert_eq!(out, "no");
    }

    /// `ne`, `gt`, and `lt` (the other three comparison helpers named in the
    /// PRD-108 step) are likewise available without explicit registration.
    /// Pins decision 7's claim that the handlebars-6 builtin set covers all
    /// four comparison operators — if a future handlebars upgrade dropped
    /// any of them, this test would fail and force the framework to either
    /// hold the dependency at 6 or re-register the dropped helper.
    #[test]
    fn ne_gt_lt_helpers_available_in_default_engine() {
        let dir = TestDir::new("comparison-helpers");
        dir.write_file(
            "ne.hbs",
            r#"{{#if (ne a b)}}different{{else}}same{{/if}}"#,
        );
        dir.write_file("gt.hbs", r#"{{#if (gt n 10)}}big{{else}}small{{/if}}"#);
        dir.write_file("lt.hbs", r#"{{#if (lt n 10)}}small{{else}}big{{/if}}"#);
        let engine = TemplateEngine::new(&dir.path).expect("engine constructs");

        assert_eq!(
            engine
                .render("ne", &serde_json::json!({ "a": 1, "b": 2 }))
                .expect("ne renders"),
            "different",
        );
        assert_eq!(
            engine
                .render("gt", &serde_json::json!({ "n": 42 }))
                .expect("gt renders"),
            "big",
        );
        assert_eq!(
            engine
                .render("lt", &serde_json::json!({ "n": 3 }))
                .expect("lt renders"),
            "small",
        );
    }

    /// The `json` helper (the only NEW helper added by JOLT-RS-108 — the
    /// comparison helpers come from handlebars' builtin set) serializes a
    /// value via [`serde_json::to_string`]. Pins that an object renders as
    /// a JSON object literal suitable for embedding inside a `<script>` tag
    /// or other JSON-consuming context. Default handlebars HTML escaping
    /// would otherwise turn `"` into `&quot;`, so the test uses the triple-
    /// brace `{{{json data}}}` form to bypass HTML escaping (the standard
    /// idiom for `<script>` blocks).
    #[test]
    fn json_helper_serializes_object_to_json_literal() {
        let dir = TestDir::new("json-helper-object");
        dir.write_file("payload.hbs", r#"{{{json data}}}"#);
        let engine = TemplateEngine::new(&dir.path).expect("engine constructs");

        let out = engine
            .render(
                "payload",
                &serde_json::json!({ "data": { "name": "World", "count": 3 } }),
            )
            .expect("render succeeds");

        // serde_json's object key order matches insertion order for
        // `serde_json::Map<String, Value>` with the default preserve_order
        // off, but key ordering is BTreeMap-alphabetical when the value
        // is constructed via the `json!` macro. Assert against the
        // alphabetical form.
        assert_eq!(out, r#"{"count":3,"name":"World"}"#);
    }

    /// `json` on a string produces a quoted JSON string (i.e. wraps the
    /// value in `"` characters and escapes interior quotes). Pins that the
    /// helper does NOT short-circuit on the string case to the bare string
    /// — a regression that special-cased strings would silently produce
    /// unquoted output that fails to parse as JSON in the consuming
    /// `<script>` block.
    #[test]
    fn json_helper_quotes_string_value() {
        let dir = TestDir::new("json-helper-string");
        dir.write_file("name.hbs", r#"const n = {{{json name}}};"#);
        let engine = TemplateEngine::new(&dir.path).expect("engine constructs");

        let out = engine
            .render("name", &serde_json::json!({ "name": "Alice" }))
            .expect("render succeeds");

        assert_eq!(out, r#"const n = "Alice";"#);
    }

    /// `json` round-trips numeric, boolean, and null literals to their JSON
    /// forms. Pins that the helper preserves the underlying JSON type
    /// rather than coercing everything to a string representation —
    /// `{{json count}}` for `count: 3` must produce `3`, not `"3"`.
    #[test]
    fn json_helper_preserves_scalar_types() {
        let dir = TestDir::new("json-helper-scalars");
        dir.write_file(
            "values.hbs",
            "n={{{json n}}}|b={{{json b}}}|x={{{json x}}}",
        );
        let engine = TemplateEngine::new(&dir.path).expect("engine constructs");

        let out = engine
            .render(
                "values",
                &serde_json::json!({ "n": 3, "b": true, "x": null }),
            )
            .expect("render succeeds");

        assert_eq!(out, "n=3|b=true|x=null");
    }

    /// Helper registration is visible on templates registered AFTER
    /// construction (e.g. via [`TemplateEngine::registry_mut`]). Pins that
    /// the helper lookup happens at render time against the engine's
    /// shared registry, not at template-compile time against a snapshot —
    /// a regression that captured the helper set into each template's
    /// compiled AST would silently fail this test.
    #[test]
    fn default_helpers_visible_to_templates_registered_after_construction() {
        let dir = TestDir::new("post-register-helpers");
        let mut engine = TemplateEngine::new(&dir.path).expect("engine constructs");
        engine
            .registry_mut()
            .register_template_string("dynamic", r#"{{{json payload}}}"#)
            .expect("register dynamic template");

        let out = engine
            .render("dynamic", &serde_json::json!({ "payload": [1, 2, 3] }))
            .expect("render succeeds");

        assert_eq!(out, "[1,2,3]");
    }
}
