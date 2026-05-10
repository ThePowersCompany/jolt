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
//!
//! [`JoltDb`] (JOLT-RS-083) is the runtime handle holding the
//! [`sqlx::PgPool`](https://docs.rs/sqlx/latest/sqlx/struct.PgPool.html) that
//! every downstream phase19/20/21 slice consumes. Construction goes through
//! [`JoltDb::connect`], which builds a
//! [`sqlx::postgres::PgPoolOptions`](https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgPoolOptions.html)
//! from the [`DbConfig`] knobs and `.connect()`s to Postgres. Architectural
//! decisions pinned here for JOLT-RS-084/085 and onward to build on:
//!
//! 5. **`JoltDb` owns the `PgPool` by value, not behind an `Arc`.**
//!    [`sqlx::PgPool`](https://docs.rs/sqlx/latest/sqlx/struct.PgPool.html) is
//!    already a cheap-to-clone handle that internally wraps an `Arc<...>`,
//!    so wrapping it again in `Arc<PgPool>` would be redundant. Callers that
//!    need shared ownership of the `JoltDb` itself can wrap the outer struct
//!    in `Arc<JoltDb>` (the eventual `JoltServer` integration will own one
//!    `Arc<JoltDb>` and clone the handle into request extensions).
//!
//! 6. **`connect` returns `Result<Self, sqlx::Error>` (the raw sqlx error).**
//!    A bespoke error enum would force callers to convert between two error
//!    shapes for trivial reasons (sqlx already produces a rich error with
//!    `Display` + `source()` for chained reporting); the connect call has
//!    exactly one failure mode (sqlx couldn't open the pool), so wrapping it
//!    adds noise. Future query helpers (JOLT-RS-086 onward) will likely
//!    return `sqlx::Error` for the same reason.
//!
//! 7. **Connect runs `lazy_connect` semantics via `PgPoolOptions::connect`,
//!    not `connect_lazy`.** `connect` actually opens at least one TCP
//!    connection before returning, which surfaces auth / unreachable-server
//!    errors at startup rather than on the first query — matching the
//!    "fail-fast on misconfiguration" contract documented for
//!    [`DbConfig`]'s empty-default `database_url`.
//!
//! 8. **`pool()` returns `&PgPool` (a borrow), not `PgPool` (a clone)
//!    (JOLT-RS-084).** Callers that need an owned handle clone the returned
//!    reference themselves (`db.pool().clone()`); callers that only need to
//!    run a query through the pool pass the borrow straight to sqlx (which
//!    accepts `&PgPool` as an executor). Returning a borrow is the
//!    lower-friction default — owners can always upgrade with `.clone()` but
//!    borrowers cannot avoid an unwanted clone.
//!
//! 9. **`health_check()` runs `SELECT 1` and returns `Result<(), sqlx::Error>`
//!    (JOLT-RS-084).** Discards the row payload — the success of the round
//!    trip is the whole signal. The error shape matches decision 6 (raw
//!    `sqlx::Error`), so a caller can pattern-match on the specific failure
//!    (e.g. `Error::PoolTimedOut` vs `Error::Io`) without an enum hop. The
//!    intended use sites are (a) the eventual `JoltServer` readiness probe,
//!    (b) HTTP `/healthz` endpoints, (c) JOLT-RS-085's closing connection
//!    test.

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

/// Runtime handle around a [`sqlx::PgPool`] consumed by every downstream
/// phase19/20/21 slice (JOLT-RS-083). See module docs decisions 5–7 for
/// the ownership shape, error contract, and connect semantics; decisions
/// 8 and 9 cover the read-side API ([`Self::pool`], [`Self::health_check`])
/// added by JOLT-RS-084.
#[derive(Debug, Clone)]
pub struct JoltDb {
    pool: sqlx::PgPool,
}

impl JoltDb {
    /// Build a pool from `config` and return the owning [`JoltDb`].
    ///
    /// Maps the three [`DbConfig`] knobs onto
    /// [`sqlx::postgres::PgPoolOptions`]:
    /// - `max_connections` → [`PgPoolOptions::max_connections`].
    /// - `acquire_timeout_secs` → [`PgPoolOptions::acquire_timeout`] via
    ///   [`std::time::Duration::from_secs`].
    /// - `database_url` → the URL handed to [`PgPoolOptions::connect`].
    ///
    /// `connect` opens at least one TCP connection before returning so
    /// auth / unreachable-server errors surface at startup rather than at
    /// first-query time (decision 7).
    ///
    /// [`PgPoolOptions::max_connections`]: https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgPoolOptions.html#method.max_connections
    /// [`PgPoolOptions::acquire_timeout`]: https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgPoolOptions.html#method.acquire_timeout
    /// [`PgPoolOptions::connect`]: https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgPoolOptions.html#method.connect
    pub async fn connect(config: &DbConfig) -> Result<Self, sqlx::Error> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(config.max_connections)
            .acquire_timeout(std::time::Duration::from_secs(
                config.acquire_timeout_secs,
            ))
            .connect(&config.database_url)
            .await?;
        Ok(Self { pool })
    }

    /// Borrow the underlying [`sqlx::PgPool`] (JOLT-RS-084, decision 8).
    ///
    /// Callers run queries by passing the returned borrow straight to sqlx
    /// (which accepts `&PgPool` as an executor). For shared owned access
    /// (e.g. handing the pool to a spawned task), call `.clone()` on the
    /// borrow — [`sqlx::PgPool`] is itself an `Arc`-wrapped cheap-clone
    /// handle.
    pub fn pool(&self) -> &sqlx::PgPool {
        &self.pool
    }

    /// Round-trip a `SELECT 1` through the pool to verify it is alive
    /// (JOLT-RS-084, decision 9).
    ///
    /// Returns `Ok(())` on a successful round trip, or the raw
    /// [`sqlx::Error`] on any failure (acquire timeout, connection drop,
    /// authentication failure, etc.). The row payload is discarded — the
    /// success of the round trip is the whole signal.
    ///
    /// Intended use sites: `JoltServer` readiness probes, HTTP `/healthz`
    /// endpoints, and JOLT-RS-085's closing connection test.
    pub async fn health_check(&self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{DbConfig, JoltDb};

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

    // ---- JOLT-RS-083: JoltDb::connect ----

    /// Unreachable-host URL produces an `Err` from `connect` rather than
    /// hanging or panicking. Pins decision 7 ("fail-fast on
    /// misconfiguration"): the connect call performs at least one real TCP
    /// dial before returning so the error surfaces at startup.
    ///
    /// Uses `127.0.0.1:1` (port 1 reserved + not listening) plus a 1-second
    /// acquire timeout so the test fails fast in CI sandboxes that have
    /// no Postgres available. The assertion only checks `is_err()` because
    /// the exact `sqlx::Error` variant (`Io` vs `PoolTimedOut`) depends on
    /// platform-specific TCP refusal timing.
    #[tokio::test]
    async fn connect_returns_err_on_unreachable_server() {
        let cfg = DbConfig {
            database_url: "postgres://nouser:nopw@127.0.0.1:1/nodb".into(),
            max_connections: 1,
            acquire_timeout_secs: 1,
        };
        let result = JoltDb::connect(&cfg).await;
        assert!(
            result.is_err(),
            "expected Err from connect to unreachable server, got Ok",
        );
    }

    /// Bogus URL scheme (`not-a-real-url`) trips sqlx's URL parser before
    /// any TCP dial happens. Confirms `connect` propagates the parse error
    /// as `sqlx::Error` rather than panicking.
    #[tokio::test]
    async fn connect_returns_err_on_malformed_url() {
        let cfg = DbConfig::new("not-a-real-url");
        let result = JoltDb::connect(&cfg).await;
        assert!(
            result.is_err(),
            "expected Err from connect with malformed URL, got Ok",
        );
    }

    /// Success-path test gated on the `JOLT_TEST_DATABASE_URL` env var.
    ///
    /// Without the env var set the test passes trivially so the default
    /// `cargo test -p jolt-db` flow does not require a running Postgres.
    /// With the env var set (e.g. `JOLT_TEST_DATABASE_URL=postgres://...
    /// cargo test -p jolt-db`) the test exercises the PRD-mandated
    /// "JoltDb::connect() returns Ok" verification.
    #[tokio::test]
    async fn connect_returns_ok_when_test_db_available() {
        let Ok(url) = std::env::var("JOLT_TEST_DATABASE_URL") else {
            // No test DB configured — skip. The error-path tests above
            // exercise the rest of the connect logic.
            return;
        };
        let cfg = DbConfig::new(url);
        let db = JoltDb::connect(&cfg)
            .await
            .expect("expected Ok from JoltDb::connect against JOLT_TEST_DATABASE_URL");
        // Pool is reachable: a trivial SELECT 1 should round-trip.
        let one: (i32,) = sqlx::query_as("SELECT 1")
            .fetch_one(db.pool())
            .await
            .expect("SELECT 1 against the connected pool failed");
        assert_eq!(one.0, 1);
    }

    // ---- JOLT-RS-084: JoltDb::pool + JoltDb::health_check ----

    /// `pool()` returns a borrow of the underlying [`sqlx::PgPool`] (decision
    /// 8). Compile-pins that the signature is `&PgPool` (a borrow) rather
    /// than `PgPool` (a clone) — the explicit `&sqlx::PgPool` binding will
    /// fail to typecheck if the getter ever changes to return an owned
    /// value.
    ///
    /// Uses the unreachable-server fixture from the connect error-path tests
    /// because the slice only needs an owned `JoltDb` to exercise the getter
    /// shape, not a live pool. The connect itself is expected to fail; the
    /// test path that actually inspects a `pool()` borrow lives in the
    /// env-gated `health_check_returns_ok_*` test below.
    #[test]
    fn pool_signature_is_borrow_not_clone() {
        // Pure compile-time pin: the binding annotation forces the return
        // type to be `&PgPool`. No runtime body needed.
        fn _pin(db: &JoltDb) -> &sqlx::PgPool {
            db.pool()
        }
    }

    /// Health-check success path gated on `JOLT_TEST_DATABASE_URL` (same
    /// convention as `connect_returns_ok_when_test_db_available`). Pins
    /// decision 9: a successful `SELECT 1` round trip resolves to `Ok(())`.
    #[tokio::test]
    async fn health_check_returns_ok_when_test_db_available() {
        let Ok(url) = std::env::var("JOLT_TEST_DATABASE_URL") else {
            return;
        };
        let cfg = DbConfig::new(url);
        let db = JoltDb::connect(&cfg)
            .await
            .expect("expected Ok from JoltDb::connect against JOLT_TEST_DATABASE_URL");
        db.health_check()
            .await
            .expect("expected Ok from JoltDb::health_check on live pool");
    }

    /// Health-check failure path: a pool whose configured server is
    /// unreachable surfaces an `Err` rather than hanging or panicking. Uses
    /// the same `127.0.0.1:1` + 1-second-acquire-timeout fixture as the
    /// connect error-path tests, with `connect_lazy_with` so the pool is
    /// constructed without an upfront TCP dial — the `SELECT 1` inside
    /// `health_check` is what tries (and fails) to acquire a connection.
    ///
    /// This is the only path in jolt-db that uses `connect_lazy_with`; it
    /// exists exclusively to give the health-check failure path a `JoltDb`
    /// to call `health_check()` on without requiring a live Postgres. The
    /// production constructor remains the eager [`JoltDb::connect`] from
    /// JOLT-RS-083.
    #[tokio::test]
    async fn health_check_returns_err_on_unreachable_server() {
        let pool_options = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(1));
        let pool = pool_options
            .connect_lazy("postgres://nouser:nopw@127.0.0.1:1/nodb")
            .expect("connect_lazy should accept a well-formed URL even if unreachable");
        let db = JoltDb { pool };
        let result = db.health_check().await;
        assert!(
            result.is_err(),
            "expected Err from health_check against unreachable server, got Ok",
        );
    }
}
