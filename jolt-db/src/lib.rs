//! jolt-db: Postgres connection pool, query helpers, and migration support
//! for the Jolt framework.
//!
//! [`DbConfig`] (JOLT-RS-082) is the per-deployment configuration record the
//! upcoming `JoltDb::connect` (JOLT-RS-083) consumes to build a
//! [`sqlx::PgPool`](https://docs.rs/sqlx/latest/sqlx/struct.PgPool.html).
//! Fields mirror the three `PgPoolOptions` knobs the connect call will set:
//! the database URL, the pool's connection ceiling, and the per-acquire
//! timeout.
//!
//! Architectural decisions pinned here for JOLT-RS-083..085 to build on:
//!
//! 1. **Plain `pub` fields, no getters/setters.** Mirrors
//!    [`CorsConfig`](../../jolt_core/server/struct.CorsConfig.html) (055) and
//!    [`JwtConfig`](../../jolt_utils/jwt/struct.JwtConfig.html) (072): callers
//!    construct a config by struct literal or by [`Self::new`] +
//!    field-by-field mutation, and the connect call consumes a `&DbConfig`
//!    rather than threading individual arguments. This keeps the surface
//!    stable as later phases extend the config with additional pool knobs
//!    (`idle_timeout`, `min_connections`, etc.) without bumping the connect
//!    signature.
//!
//! 2. **`Default` is hand-written, not derived.** The PRD-mandated defaults
//!    for `max_connections` (10) and `acquire_timeout_secs` (30) are NOT the
//!    types' built-in zero defaults. A derived `Default` would produce
//!    `max_connections = 0` (no usable connections) and
//!    `acquire_timeout_secs = 0` (instant-fail acquires) — both nonsensical
//!    operationally. The manual impl pins the documented defaults so a caller
//!    who does `DbConfig::default()` (or `DbConfig::new(url)`, which delegates
//!    to `Default` for the other fields) gets an operationally usable pool.
//!
//! 3. **`database_url` defaults to an empty string.** It has no operationally
//!    sensible default — every deployment supplies its own URL — but a
//!    `Default` impl is still useful for the "fill the rest from defaults"
//!    construction pattern. The empty-string default is intentionally
//!    nonfunctional; passing it to `connect` will surface as a connect-time
//!    error from sqlx rather than a silent connection to a wrong server.
//!    [`Self::new`] is the canonical caller-facing constructor for this
//!    reason — it accepts the URL up front and defers only the optional
//!    knobs to `Default`.
//!
//! 4. **`acquire_timeout_secs` is `u64`, not `Duration`.** The connect call
//!    will convert via `Duration::from_secs(config.acquire_timeout_secs)`.
//!    Carrying seconds-as-`u64` (instead of `Duration` directly) keeps the
//!    field trivially `Copy`/`Debug`/`serde`-serializable for the eventual
//!    config-from-env / config-from-file paths, and seconds are a coarse
//!    enough unit for pool acquire that sub-second precision is not useful.

/// Per-deployment Postgres pool configuration consumed by the upcoming
/// `JoltDb::connect` (JOLT-RS-083) to build a
/// [`sqlx::PgPool`](https://docs.rs/sqlx/latest/sqlx/struct.PgPool.html).
///
/// See module docs for the architectural contract. The short version:
/// plain `pub` fields, hand-written [`Default`] so the documented
/// `max_connections` / `acquire_timeout_secs` defaults are pinned, and
/// [`Self::new`] for the canonical "URL up front, knobs from defaults"
/// construction shape.
#[derive(Debug, Clone)]
pub struct DbConfig {
    /// Postgres connection URL (e.g. `postgres://user:pass@host:5432/db`).
    /// No operationally sensible default — every deployment supplies its
    /// own — so [`Default`] uses an empty string; pass it to `connect` and
    /// sqlx will surface the misconfiguration at connect time.
    pub database_url: String,
    /// Maximum number of connections the pool will open. Default: `10`.
    /// Maps to
    /// [`PgPoolOptions::max_connections`](https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgPoolOptions.html#method.max_connections).
    pub max_connections: u32,
    /// Per-acquire timeout in seconds. Default: `30`. Converted to a
    /// [`std::time::Duration`] by `JoltDb::connect` before being handed to
    /// [`PgPoolOptions::acquire_timeout`](https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgPoolOptions.html#method.acquire_timeout).
    pub acquire_timeout_secs: u64,
}

impl DbConfig {
    /// Build a config from a URL with the spec-mandated defaults applied to
    /// the other fields (`max_connections = 10`, `acquire_timeout_secs = 30`).
    /// `database_url` accepts any `Into<String>` so the canonical
    /// `DbConfig::new("postgres://...")` form just works with both `&str`
    /// and `String`.
    pub fn new(database_url: impl Into<String>) -> Self {
        Self {
            database_url: database_url.into(),
            ..Self::default()
        }
    }
}

impl Default for DbConfig {
    /// `database_url = ""`, `max_connections = 10`,
    /// `acquire_timeout_secs = 30`. See module docs decisions 2 and 3.
    fn default() -> Self {
        Self {
            database_url: String::new(),
            max_connections: 10,
            acquire_timeout_secs: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DbConfig;

    #[test]
    fn default_pins_max_connections_to_10() {
        // PRD-mandated default surfaces verbatim. A derived Default would
        // give `0`; this asserts the hand-written impl is what's in effect.
        assert_eq!(DbConfig::default().max_connections, 10);
    }

    #[test]
    fn default_pins_acquire_timeout_to_30_seconds() {
        assert_eq!(DbConfig::default().acquire_timeout_secs, 30);
    }

    #[test]
    fn default_url_is_empty_string() {
        // Documented in module docs decision 3: empty by default; callers
        // override via `new` or struct-literal mutation. Pinned so a later
        // refactor that tries to derive `Default` (which would also produce
        // empty here, but with the wrong numeric defaults) gets caught by
        // the sibling tests above.
        assert!(DbConfig::default().database_url.is_empty());
    }

    #[test]
    fn new_carries_url_through_with_str() {
        let cfg = DbConfig::new("postgres://localhost/db");
        assert_eq!(cfg.database_url, "postgres://localhost/db");
    }

    #[test]
    fn new_carries_url_through_with_owned_string() {
        // `Into<String>` overload accepts owned strings without an extra
        // `.to_string()` at the call site. Pinned to catch a regression
        // that narrows the parameter type to `&str`.
        let url: String = String::from("postgres://localhost/db");
        let cfg = DbConfig::new(url);
        assert_eq!(cfg.database_url, "postgres://localhost/db");
    }

    #[test]
    fn new_applies_default_knobs() {
        // The two non-URL knobs come from Default, so `new` users get the
        // PRD-mandated defaults without a second call.
        let cfg = DbConfig::new("postgres://localhost/db");
        assert_eq!(cfg.max_connections, 10);
        assert_eq!(cfg.acquire_timeout_secs, 30);
    }

    #[test]
    fn struct_literal_construction_compiles_with_all_fields_pub() {
        // Pins the `pub` field contract (decision 1). A regression that
        // makes any field private would fail this test to compile.
        let cfg = DbConfig {
            database_url: String::from("postgres://localhost/db"),
            max_connections: 25,
            acquire_timeout_secs: 5,
        };
        assert_eq!(cfg.max_connections, 25);
        assert_eq!(cfg.acquire_timeout_secs, 5);
    }

    #[test]
    fn debug_is_implemented() {
        // Confirms the derive landed — the connect call (JOLT-RS-083) will
        // want to log the config on startup at least once.
        let cfg = DbConfig::new("postgres://localhost/db");
        let rendered = format!("{cfg:?}");
        assert!(rendered.contains("DbConfig"));
        assert!(rendered.contains("postgres://localhost/db"));
    }

    #[test]
    fn clone_is_implemented() {
        // Connect-call (JOLT-RS-083) may want to keep an owned clone of the
        // config alongside the pool; pinned so the derive doesn't get
        // dropped.
        let cfg = DbConfig::new("postgres://localhost/db");
        let copy = cfg.clone();
        assert_eq!(copy.database_url, cfg.database_url);
        assert_eq!(copy.max_connections, cfg.max_connections);
        assert_eq!(copy.acquire_timeout_secs, cfg.acquire_timeout_secs);
    }
}
