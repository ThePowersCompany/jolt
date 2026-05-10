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
//!
//! [`TypedQuery`] (JOLT-RS-086) opens phase20's typed-query helper layer on
//! top of [`JoltDb::pool`]. `db.query_as::<T>(sql)` returns a [`TypedQuery<T>`]
//! that exposes `.bind(...)` for positional parameters and the three terminal
//! fetch verbs (`.fetch_one()`, `.fetch_optional()`, `.fetch_all()`).
//! Architectural decisions pinned here for JOLT-RS-087..089 and onward:
//!
//! 10. **`TypedQuery<T>` owns its SQL, args, and a cloned `PgPool` handle;
//!     no exposed lifetimes.** The struct holds the SQL as an owned `String`
//!     and the bound parameters as an owned [`sqlx::postgres::PgArguments`].
//!     This sidesteps the self-borrowing lifetime trap that the obvious
//!     "wrap [`sqlx::query::QueryAs`] directly" approach hits (sqlx's
//!     `QueryAs<'q, ...>` borrows the SQL `&'q str`, which can't be stored
//!     alongside the `String` it borrows from in the same struct). The
//!     pool field is a [`sqlx::PgPool`] *clone* (cheap — sqlx's pool is an
//!     `Arc` internally) so each `TypedQuery<T>` is fully self-contained
//!     and outlives the originating `JoltDb` borrow. Terminal fetch methods
//!     reconstitute a fresh [`sqlx::query_as_with`] inside their body using
//!     the owned SQL + args, so the borrowed-vs-owned lifetime question
//!     never reaches the caller's signature. The PRD-mandated "params..."
//!     in JOLT-RS-086 is realized via the chainable `.bind(value)` builder;
//!     bound values must be `'static + Send + Encode + Type` so the
//!     `TypedQuery<T>` itself remains `'static + Send` and can cross task
//!     boundaries / be stored in a struct without a lifetime parameter.
//!
//! 11. **`.bind()` panics on encode failure rather than returning `Result`
//!     (JOLT-RS-086).** sqlx 0.8's
//!     [`Arguments::add`](https://docs.rs/sqlx/latest/sqlx/trait.Arguments.html#tymethod.add)
//!     returns `Result<(), BoxDynError>` for the rare case where a value
//!     fails to encode at bind time (e.g. a custom type whose `Encode` impl
//!     refuses some input). Propagating that `Result` would force every
//!     caller into `?` syntax on every `.bind()` even though the common
//!     case (primitives, `String`, `chrono`, `uuid`) cannot fail. The
//!     chainable builder shape (`q.bind(a).bind(b).fetch_one()`) is more
//!     valuable than recovering from a programming error. Pathological
//!     encode failures panic with a descriptive message. The terminal
//!     `.fetch_*` methods *do* return `Result<_, sqlx::Error>` for the
//!     normal runtime failure modes (acquire timeout, row count mismatch,
//!     column type mismatch, etc.).
//!
//! [`JoltDb::transaction`] (JOLT-RS-088) layers the auto-commit / auto-
//! rollback wrapper on top of [`sqlx::PgPool::begin`]. A caller hands in a
//! `FnOnce(&mut Transaction)` whose body returns a `Result<T, sqlx::Error>`;
//! `transaction` opens a tx, runs the closure, commits on `Ok` and rolls
//! back on `Err`. Architectural decisions pinned here for JOLT-RS-089 and
//! onward:
//!
//! 12. **The closure receives `&mut sqlx::Transaction<'static, Postgres>`
//!     directly — sqlx-native, not a JoltDb wrapper (JOLT-RS-088).** A wrapper
//!     would have to re-implement the typed-query helpers (or a tx-aware
//!     [`TypedQuery`] variant) to give callers anything beyond raw sqlx, and
//!     the [`pool`](Self::pool) getter from JOLT-RS-084 already exposes raw
//!     sqlx for the non-tx path. Symmetric design: outside the closure callers
//!     reach for raw sqlx via `db.pool()`; inside they reach for raw sqlx via
//!     the `&mut Transaction`. A future tx-aware `TypedQuery` can be added
//!     without disturbing this contract — it would be a layer on top, not a
//!     replacement.
//!
//! 13. **The closure returns `Pin<Box<dyn Future + Send + 'c>>`, not a bare
//!     `Future` value (JOLT-RS-088).** The future borrows the `&'c mut
//!     Transaction` argument for the duration of its body, which Rust's
//!     stable trait system cannot express as a non-`'static` HRTB on a bare
//!     `impl Future` return without async closures. The `Box::pin(async
//!     move { ... })` pattern at the call site is the established workaround
//!     and matches how sqlx's own examples wire transactions in stable Rust.
//!     When async closures stabilize this signature can be loosened without
//!     breaking callers.
//!
//! 14. **`Ok` commits, `Err` rolls back; if `commit` itself fails the error
//!     surfaces, but if `rollback` itself fails the closure's error wins
//!     (JOLT-RS-088).** Rationale: when the closure returns `Err` the user
//!     already knows the operation failed and cares about *why*; a follow-on
//!     rollback failure (typically connection-level) would mask the real
//!     cause. Dropping a `Transaction` without commit also auto-rolls-back
//!     at the connection level, so a failed explicit rollback is rarely
//!     load-bearing. On the commit path the commit error *is* the reason the
//!     txn didn't take effect, so propagating it directly is correct.
//!
//! [`JoltDb::listen_connection`] (JOLT-RS-090) opens phase21's LISTEN/NOTIFY
//! layer. Returns a dedicated [`sqlx::postgres::PgListener`] — a single
//! Postgres connection allocated outside the regular pool and reserved for
//! `LISTEN <channel>` + notification streaming. JOLT-RS-091 (`listen`) and
//! JOLT-RS-092 (`notify`) build on top of this opener.
//!
//! 15. **Dedicated connection is a `sqlx::postgres::PgListener`, not a
//!     `tokio_postgres::Connection` (JOLT-RS-090).** The PRD's task wording
//!     ("dedicated tokio-postgres connection") describes what kind of
//!     connection LISTEN/NOTIFY needs (a single long-lived TCP connection
//!     dedicated to receiving async notifications, *not* a pool-checked-out
//!     connection that gets returned between calls). It does not mandate
//!     adding `tokio-postgres` as a sibling driver to sqlx. sqlx's
//!     [`PgListener`](https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgListener.html)
//!     is exactly this shape — built on tokio-postgres-style async
//!     mechanics internally but exposed through sqlx's existing trait stack,
//!     producing the same `sqlx::Error` shape as the rest of jolt-db
//!     (decision 6) and the same
//!     [`PgNotification`](https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgNotification.html)
//!     type that JOLT-RS-091's stream will surface. Adding `tokio-postgres`
//!     as a second driver would force the LISTEN/NOTIFY surface to use a
//!     foreign error type and `Notification` struct, double the workspace's
//!     async Postgres dependency footprint, and create a second connection
//!     URL / TLS / SCRAM-auth code path. Using sqlx end-to-end keeps the
//!     whole jolt-db crate on one driver stack.
//!
//! 16. **`listen_connection` allocates a fresh connection via
//!     [`PgListener::connect_with`](https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgListener.html#method.connect_with),
//!     reusing the pool's configured connection options (JOLT-RS-090).** This
//!     gives the listener the same URL / TLS / credentials the pool was built
//!     with (so a deployment configures Postgres exactly once via
//!     [`DbConfig`]) while still allocating a *new* connection outside the
//!     pool — the PRD-mandated "separate from pool" property holds. The
//!     alternative (`PgListener::connect(url_string)`) would require
//!     re-storing the URL on `JoltDb` after `connect` consumed it, which
//!     breaks the existing decision-5 "JoltDb owns just the pool" shape.
//!     `connect_with` sidesteps the storage question entirely by reading the
//!     options off the pool handle directly.
//!
//! 17. **Returns the `PgListener` to the caller by value rather than storing
//!     it on `JoltDb` (JOLT-RS-090).** `PgListener` is `&mut self`-driven for
//!     `listen` / `recv` / `into_stream`, so a single shared `PgListener`
//!     stored on `JoltDb` would force all listeners through one connection
//!     and serialize them behind a `Mutex`. Returning a fresh listener per
//!     call lets each subscriber own its own dedicated connection (matching
//!     the spec's per-channel listen model) and avoids cross-subscriber
//!     interference. JOLT-RS-091's `listen(channel)` will be a convenience
//!     wrapper that calls `listen_connection()` + `listener.listen(channel)`
//!     + `listener.into_stream()` internally.
//!
//! [`JoltDb::listen`] (JOLT-RS-091) builds the high-level streaming verb on
//! top of [`Self::listen_connection`]. Returns a `Stream` of
//! [`sqlx::postgres::PgNotification`] items, each wrapped in
//! `Result<_, sqlx::Error>` because the underlying connection can drop
//! mid-stream and the auto-reconnect machinery surfaces the failure as an
//! item rather than ending the stream silently.
//!
//! 18. **Two error tiers: outer `Result` for setup, per-item `Result` for
//!     mid-stream failures (JOLT-RS-091).** `listen` is `async fn -> Result<
//!     impl Stream<Item = Result<PgNotification, sqlx::Error>> + Unpin,
//!     sqlx::Error>`. The outer `Result` covers the two upfront failure modes
//!     (the [`PgListener::connect_with`] dial inherited from decision 16, and
//!     the `LISTEN <channel>` round trip that subscribes to the channel) —
//!     callers can `?`-propagate these at startup. The inner per-item
//!     `Result` mirrors [`sqlx::postgres::PgListener::into_stream`]'s native
//!     shape verbatim: each delivered notification is `Ok(PgNotification)`,
//!     and a connection drop / reconnect-failure surfaces as `Err(sqlx::
//!     Error)` so the subscriber can decide whether to log, retry, or
//!     abandon the subscription. Squashing the two tiers (e.g. returning a
//!     stream whose first item carries the setup error) would force every
//!     subscriber to write the same "peek the first item to find out if
//!     setup worked" boilerplate. Keeping them separate matches the rest of
//!     the crate's "fail-fast on misconfiguration" stance (decision 7) for
//!     the setup half while preserving the canonical `Stream` shape for the
//!     hot path. The named `Stream` trait is imported from
//!     [`tokio_stream::Stream`] (a re-export of `futures_core::Stream`),
//!     pulled in via the `tokio-stream` workspace dep so the public surface
//!     does not pin callers to any specific futures runtime.
//!
//! 19. **`listen` consumes the channel name as `&str` and propagates it
//!     unmodified to [`PgListener::listen`] (JOLT-RS-091).** No quoting,
//!     escaping, or validation happens at the jolt-db layer — sqlx's
//!     `PgListener::listen` already does the right thing (it issues a
//!     parameterized `LISTEN` via the wire protocol, so SQL-injection
//!     vectors via channel name are sqlx's concern, not ours). Channel
//!     name semantics (case folding, identifier length limits) are
//!     Postgres's concern and would be the same whether or not jolt-db
//!     wrapped this call. Future overloads for `listen_all(channels:
//!     &[&str])` can be added without disturbing this single-channel
//!     contract.
//!
//! 20. **Each `listen` call allocates a fresh dedicated connection
//!     (JOLT-RS-091, inherits from decision 17).** A pair of
//!     `db.listen("ch_a")` and `db.listen("ch_b")` calls produces two
//!     independent streams backed by two independent connections; one
//!     stream's connection drop does not affect the other. Callers who
//!     want multi-channel multiplexing on a single connection should
//!     reach for [`Self::listen_connection`] directly and call
//!     `listener.listen_all(...)` themselves — that primitive remains
//!     available exactly for this case.

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

    /// Build a typed query against this pool for a row type `T`
    /// (JOLT-RS-086).
    ///
    /// Returns a [`TypedQuery<T>`] that the caller chains `.bind(...)` calls
    /// onto for positional parameters (`$1`, `$2`, ...) and finishes with one
    /// of the three terminal verbs: [`TypedQuery::fetch_one`],
    /// [`TypedQuery::fetch_optional`], or [`TypedQuery::fetch_all`]. The
    /// returned `TypedQuery<T>` owns its SQL, args, and a cloned pool handle
    /// (decision 10), so it is `'static + Send` and can be stored or moved
    /// across task boundaries.
    ///
    /// `T` must implement
    /// [`sqlx::FromRow`](https://docs.rs/sqlx/latest/sqlx/trait.FromRow.html)
    /// for [`sqlx::postgres::PgRow`]. The simplest way to satisfy that is
    /// `#[derive(sqlx::FromRow)]` on a struct whose field names + types match
    /// the query's projected columns.
    pub fn query_as<T>(&self, sql: impl Into<String>) -> TypedQuery<T>
    where
        T: for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> + Send + Unpin,
    {
        TypedQuery {
            sql: sql.into(),
            pool: self.pool.clone(),
            args: sqlx::postgres::PgArguments::default(),
            _marker: std::marker::PhantomData,
        }
    }

    /// Run `f` inside a Postgres transaction (JOLT-RS-088).
    ///
    /// Opens a transaction with [`sqlx::PgPool::begin`], hands the closure a
    /// `&mut sqlx::Transaction<'static, Postgres>` (decision 12), and:
    /// - on `Ok(value)`, commits and returns `Ok(value)`;
    /// - on `Err(err)`, rolls back and returns `Err(err)` — the closure's
    ///   error wins even if the rollback itself fails (decision 14).
    ///
    /// The closure returns `Pin<Box<dyn Future + Send + 'c>>` (decision 13);
    /// at the call site this is the standard `|tx| Box::pin(async move {
    /// ... })` pattern. Inside the closure callers run queries against
    /// `&mut **tx` (the `&mut PgConnection` derefs sqlx's
    /// `Transaction → PgConnection` chain).
    ///
    /// # Example
    ///
    /// ```ignore
    /// db.transaction(|tx| {
    ///     Box::pin(async move {
    ///         sqlx::query("INSERT INTO accounts (id) VALUES ($1)")
    ///             .bind(7_i32)
    ///             .execute(&mut **tx)
    ///             .await?;
    ///         Ok(())
    ///     })
    /// })
    /// .await?;
    /// ```
    pub async fn transaction<F, T>(&self, f: F) -> Result<T, sqlx::Error>
    where
        F: for<'c> FnOnce(
            &'c mut sqlx::Transaction<'static, sqlx::Postgres>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<T, sqlx::Error>> + Send + 'c>,
        >,
    {
        let mut tx = self.pool.begin().await?;
        match f(&mut tx).await {
            Ok(value) => {
                tx.commit().await?;
                Ok(value)
            }
            Err(err) => {
                // Best-effort rollback. The closure's error is the real
                // user-facing cause (decision 14); a rollback failure here
                // is typically a downstream connection issue and dropping
                // the transaction also auto-rolls-back at the sqlx layer.
                let _ = tx.rollback().await;
                Err(err)
            }
        }
    }

    /// Open a dedicated [`sqlx::postgres::PgListener`] connection for
    /// LISTEN/NOTIFY (JOLT-RS-090).
    ///
    /// Allocates a fresh Postgres connection outside the pool, using the
    /// pool's existing connection options (URL, TLS, credentials) via
    /// [`PgListener::connect_with`]. The returned listener is the caller's
    /// to own — call `.listen(channel)` to subscribe, then drive notifications
    /// with `.recv()`, `.try_recv()`, or `.into_stream()`.
    ///
    /// Each call allocates a brand-new connection so concurrent subscribers
    /// do not contend on a shared `PgListener` (decision 17). The pool
    /// continues to serve regular pooled queries unaffected.
    ///
    /// Errors mirror [`PgListener::connect_with`]: returns the raw
    /// [`sqlx::Error`] from the connection attempt (decision 6).
    ///
    /// [`PgListener::connect_with`]: https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgListener.html#method.connect_with
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut listener = db.listen_connection().await?;
    /// listener.listen("orders").await?;
    /// while let Some(note) = listener.try_recv().await? {
    ///     println!("got {} on {}", note.payload(), note.channel());
    /// }
    /// ```
    pub async fn listen_connection(&self) -> Result<sqlx::postgres::PgListener, sqlx::Error> {
        sqlx::postgres::PgListener::connect_with(&self.pool).await
    }

    /// Subscribe to a Postgres `LISTEN` channel and stream notifications
    /// (JOLT-RS-091).
    ///
    /// Composes [`Self::listen_connection`] + [`PgListener::listen`] +
    /// [`PgListener::into_stream`]: opens a fresh dedicated connection,
    /// issues `LISTEN <channel>`, and surfaces the resulting notification
    /// stream. See module docs decisions 18–20 for the two-tier error
    /// shape, the unmodified channel-name propagation, and the
    /// per-subscriber-connection isolation.
    ///
    /// The outer `Result` reports setup failures (the listener-connection
    /// dial or the `LISTEN` round trip itself). Each item on the returned
    /// stream is itself a `Result`: `Ok(PgNotification)` for a delivered
    /// notification, `Err(sqlx::Error)` if the auto-reconnecting backing
    /// connection finally surfaces a failure.
    ///
    /// [`PgListener::listen`]: https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgListener.html#method.listen
    /// [`PgListener::into_stream`]: https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgListener.html#method.into_stream
    ///
    /// # Example
    ///
    /// ```ignore
    /// use tokio_stream::StreamExt;
    /// let mut stream = db.listen("orders").await?;
    /// while let Some(item) = stream.next().await {
    ///     let note = item?;
    ///     println!("got {} on {}", note.payload(), note.channel());
    /// }
    /// ```
    pub async fn listen(
        &self,
        channel: &str,
    ) -> Result<
        impl tokio_stream::Stream<Item = Result<sqlx::postgres::PgNotification, sqlx::Error>>
            + Unpin,
        sqlx::Error,
    > {
        let mut listener = self.listen_connection().await?;
        listener.listen(channel).await?;
        Ok(listener.into_stream())
    }
}

/// Typed-query builder returned by [`JoltDb::query_as`] (JOLT-RS-086).
///
/// Owns its SQL, bound arguments, and a cloned [`sqlx::PgPool`] handle so it
/// has no exposed lifetimes (decision 10). Chain `.bind(value)` for each
/// positional parameter (`$1`, `$2`, ...) and finish with one of the three
/// terminal fetch methods.
///
/// The phantom marker uses `fn() -> T` so `TypedQuery<T>` is `Send + Sync`
/// regardless of whether `T` is — the query never holds a `T` value, only
/// the type-level promise that the row will deserialize into one.
pub struct TypedQuery<T> {
    sql: String,
    pool: sqlx::PgPool,
    args: sqlx::postgres::PgArguments,
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<T> TypedQuery<T>
where
    T: for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> + Send + Unpin,
{
    /// Bind one positional parameter to this query (JOLT-RS-086, decision 11).
    ///
    /// Parameters are bound in the order called: the first `.bind(...)`
    /// becomes `$1`, the second `$2`, and so on. Returns `self` so calls
    /// chain (`q.bind(a).bind(b).fetch_one()`).
    ///
    /// Bound values are required to be `'static + Send` so the resulting
    /// `TypedQuery<T>` remains `'static + Send`. To bind a borrowed value
    /// (e.g. `&str`), call `.to_owned()` at the call site.
    ///
    /// # Panics
    ///
    /// Panics if sqlx's
    /// [`Arguments::add`](https://docs.rs/sqlx/latest/sqlx/trait.Arguments.html#tymethod.add)
    /// rejects the value (a custom `Encode` impl refused to encode it).
    /// Primitive types, `String`, `chrono`, and `uuid` cannot trigger this.
    /// See decision 11 for why this is a panic rather than a `Result`.
    pub fn bind<V>(mut self, value: V) -> Self
    where
        V: 'static + Send + sqlx::Type<sqlx::Postgres> + sqlx::Encode<'static, sqlx::Postgres>,
    {
        use sqlx::Arguments;
        self.args
            .add(value)
            .expect("TypedQuery::bind: sqlx Arguments::add rejected the value (see decision 11)");
        self
    }

    /// Execute the query and return exactly one row, deserialized as `T`.
    ///
    /// Returns `Err(sqlx::Error::RowNotFound)` if zero rows match;
    /// returns `Err` with sqlx's "more than one row" diagnostic if more
    /// than one row matches. For "zero or one" semantics use
    /// [`Self::fetch_optional`].
    pub async fn fetch_one(self) -> Result<T, sqlx::Error> {
        sqlx::query_as_with::<sqlx::Postgres, T, _>(&self.sql, self.args)
            .fetch_one(&self.pool)
            .await
    }

    /// Execute the query and return the matching row as `Some(T)`, or
    /// `None` if no row matched. If more than one row matches, returns an
    /// `Err`.
    pub async fn fetch_optional(self) -> Result<Option<T>, sqlx::Error> {
        sqlx::query_as_with::<sqlx::Postgres, T, _>(&self.sql, self.args)
            .fetch_optional(&self.pool)
            .await
    }

    /// Execute the query and return every matching row as a `Vec<T>`.
    /// Returns an empty `Vec` (not an error) if no rows matched.
    pub async fn fetch_all(self) -> Result<Vec<T>, sqlx::Error> {
        sqlx::query_as_with::<sqlx::Postgres, T, _>(&self.sql, self.args)
            .fetch_all(&self.pool)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::{DbConfig, JoltDb, TypedQuery};

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

    // ---- JOLT-RS-086: JoltDb::query_as + TypedQuery<T> ----

    /// Compile-time pin: `JoltDb::query_as::<T>(sql)` returns
    /// `TypedQuery<T>` (decision 10). A regression that wraps the return
    /// type in a `Result<...>`, a `Pin<Box<dyn Future<...>>>`, or anything
    /// other than `TypedQuery<T>` fails this typecheck without ever running.
    #[test]
    fn query_as_signature_returns_typed_query() {
        // Type-pin tests below only reference `Row` as a generic argument,
        // so the compiler can't see through sqlx's `FromRow` derive to know
        // the struct is constructed. `#[allow(dead_code)]` on the struct
        // suppresses the noise without hiding real dead code elsewhere.
        #[derive(sqlx::FromRow)]
        #[allow(dead_code)]
        struct Row {
            id: i32,
        }
        fn _pin(db: &JoltDb) -> TypedQuery<Row> {
            db.query_as::<Row>("SELECT 1 AS id")
        }
    }

    /// Compile-time pin: the three terminal verbs return the shapes
    /// documented on `TypedQuery<T>` and consume `self` (decision 10).
    /// `fetch_one -> Result<T>`, `fetch_optional -> Result<Option<T>>`,
    /// `fetch_all -> Result<Vec<T>>`, every error a raw `sqlx::Error`.
    #[test]
    fn terminal_verbs_return_documented_shapes() {
        #[derive(sqlx::FromRow)]
        #[allow(dead_code)]
        struct Row {
            id: i32,
        }
        async fn _pin_one(q: TypedQuery<Row>) -> Result<Row, sqlx::Error> {
            q.fetch_one().await
        }
        async fn _pin_optional(q: TypedQuery<Row>) -> Result<Option<Row>, sqlx::Error> {
            q.fetch_optional().await
        }
        async fn _pin_all(q: TypedQuery<Row>) -> Result<Vec<Row>, sqlx::Error> {
            q.fetch_all().await
        }
    }

    /// Compile-time pin: `.bind(value)` returns `Self` (chainable) so a
    /// caller can write `q.bind(a).bind(b).fetch_one()` (decision 11). A
    /// regression that switches `.bind` to `Result<Self>` would force every
    /// caller into `?` and break the chain.
    #[test]
    fn bind_returns_self_for_chaining() {
        #[derive(sqlx::FromRow)]
        #[allow(dead_code)]
        struct Row {
            v: i32,
        }
        fn _pin(db: &JoltDb) -> TypedQuery<Row> {
            db.query_as::<Row>("SELECT $1::int4 AS v").bind(7_i32)
        }
    }

    /// `TypedQuery<T>` is `'static + Send` so callers can store it or move
    /// it across `tokio::spawn` boundaries (decision 10). The owned-SQL +
    /// owned-args + cloned-pool design exists specifically to satisfy this;
    /// regressing into a borrowed-SQL design would fail this pin.
    #[test]
    fn typed_query_is_static_send() {
        #[derive(sqlx::FromRow)]
        #[allow(dead_code)]
        struct Row {
            id: i32,
        }
        fn _assert_static_send<T: 'static + Send>(_: &T) {}
        fn _pin(db: &JoltDb) {
            let q: TypedQuery<Row> = db.query_as::<Row>("SELECT 1 AS id");
            _assert_static_send(&q);
        }
    }

    /// PRD-mandated success-path verification for JOLT-RS-086:
    /// `db.query_as::<TestRow>("SELECT 1 AS id").fetch_one()` returns
    /// `TestRow { id: 1 }`. Gated on `JOLT_TEST_DATABASE_URL` so the
    /// default `cargo test -p jolt-db` flow does not require a running
    /// Postgres; with the env var set the test exercises the full pipeline.
    #[tokio::test]
    async fn query_as_fetch_one_returns_test_row() {
        let Ok(url) = std::env::var("JOLT_TEST_DATABASE_URL") else {
            return;
        };

        #[derive(sqlx::FromRow)]
        struct TestRow {
            id: i32,
        }

        let cfg = DbConfig::new(url);
        let db = JoltDb::connect(&cfg)
            .await
            .expect("connect against JOLT_TEST_DATABASE_URL");
        let row: TestRow = db
            .query_as::<TestRow>("SELECT 1 AS id")
            .fetch_one()
            .await
            .expect("fetch_one of SELECT 1 AS id");
        assert_eq!(row.id, 1);
    }

    /// `fetch_optional` returns `Ok(None)` for a query that matches zero
    /// rows. Env-gated (same convention) — pins the documented "zero or one"
    /// semantics of the middle terminal verb.
    #[tokio::test]
    async fn query_as_fetch_optional_returns_none_for_empty() {
        let Ok(url) = std::env::var("JOLT_TEST_DATABASE_URL") else {
            return;
        };

        #[derive(sqlx::FromRow)]
        struct Row {
            #[allow(dead_code)]
            id: i32,
        }

        let cfg = DbConfig::new(url);
        let db = JoltDb::connect(&cfg).await.expect("connect");
        let result: Option<Row> = db
            .query_as::<Row>("SELECT 1 AS id WHERE FALSE")
            .fetch_optional()
            .await
            .expect("fetch_optional of empty result set");
        assert!(result.is_none(), "expected None for WHERE FALSE, got Some");
    }

    /// `fetch_all` returns the matching rows as a `Vec<T>` (multi-row case,
    /// not just one). Env-gated (same convention) — pins the documented
    /// "every match" semantics and shows the `UNION ALL` shape working.
    #[tokio::test]
    async fn query_as_fetch_all_returns_all_rows() {
        let Ok(url) = std::env::var("JOLT_TEST_DATABASE_URL") else {
            return;
        };

        #[derive(sqlx::FromRow)]
        struct Row {
            id: i32,
        }

        let cfg = DbConfig::new(url);
        let db = JoltDb::connect(&cfg).await.expect("connect");
        let rows: Vec<Row> = db
            .query_as::<Row>("SELECT 1 AS id UNION ALL SELECT 2 UNION ALL SELECT 3")
            .fetch_all()
            .await
            .expect("fetch_all of three-row UNION");
        let ids: Vec<i32> = rows.into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    /// `.bind($1)` actually threads the parameter through to the DB.
    /// Env-gated (same convention) — pins the `(sql, params...)` half of the
    /// JOLT-RS-086 description that the parameterless `SELECT 1 AS id` test
    /// doesn't exercise.
    #[tokio::test]
    async fn query_as_with_bind_round_trips_parameter() {
        let Ok(url) = std::env::var("JOLT_TEST_DATABASE_URL") else {
            return;
        };

        #[derive(sqlx::FromRow)]
        struct Row {
            v: i32,
        }

        let cfg = DbConfig::new(url);
        let db = JoltDb::connect(&cfg).await.expect("connect");
        let row: Row = db
            .query_as::<Row>("SELECT $1::int4 AS v")
            .bind(42_i32)
            .fetch_one()
            .await
            .expect("fetch_one of SELECT $1::int4 AS v with bind(42)");
        assert_eq!(row.v, 42);
    }

    // ---- JOLT-RS-088: JoltDb::transaction ----

    /// Compile-time pin: `db.transaction(|tx| Box::pin(async move { ... }))`
    /// typechecks against the documented signature (decisions 12–13). The
    /// `_pin` fn never runs; it exists so a regression that changes the
    /// closure parameter type away from `&mut Transaction<'static, Postgres>`
    /// or the return type away from `Pin<Box<dyn Future ...>>` fails the
    /// build.
    #[test]
    fn transaction_signature_accepts_box_pin_closure() {
        async fn _pin(db: &JoltDb) -> Result<i32, sqlx::Error> {
            db.transaction(|_tx| Box::pin(async move { Ok::<_, sqlx::Error>(42_i32) }))
                .await
        }
    }

    /// Commit path: closure returns `Ok` → transaction commits → the
    /// inserted row is visible after `transaction` returns. Env-gated.
    /// Uses a unique table name (`_jolt_tx_commit_test`) and aggressive
    /// `DROP TABLE IF EXISTS` setup/teardown so the test is self-contained
    /// and parallel-safe against the rollback test below (different table).
    #[tokio::test]
    async fn transaction_commits_on_ok_when_test_db_available() {
        let Ok(url) = std::env::var("JOLT_TEST_DATABASE_URL") else {
            return;
        };
        let cfg = DbConfig::new(url);
        let db = JoltDb::connect(&cfg).await.expect("connect");

        sqlx::query("DROP TABLE IF EXISTS _jolt_tx_commit_test")
            .execute(db.pool())
            .await
            .expect("drop table (setup)");
        sqlx::query("CREATE TABLE _jolt_tx_commit_test (id INT)")
            .execute(db.pool())
            .await
            .expect("create table");

        let result = db
            .transaction(|tx| {
                Box::pin(async move {
                    sqlx::query("INSERT INTO _jolt_tx_commit_test (id) VALUES (1)")
                        .execute(&mut **tx)
                        .await?;
                    Ok::<_, sqlx::Error>(())
                })
            })
            .await;
        assert!(result.is_ok(), "expected Ok from commit-path transaction");

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _jolt_tx_commit_test")
            .fetch_one(db.pool())
            .await
            .expect("count after commit");
        assert_eq!(
            count.0, 1,
            "expected committed insert to be visible after transaction returned Ok",
        );

        sqlx::query("DROP TABLE _jolt_tx_commit_test")
            .execute(db.pool())
            .await
            .expect("drop table (teardown)");
    }

    /// Rollback path: closure returns `Err` → transaction rolls back → the
    /// attempted insert is *not* visible. The closure's error propagates
    /// out (decision 14). Env-gated; uses a separate table from the commit
    /// test for parallel safety.
    #[tokio::test]
    async fn transaction_rolls_back_on_err_when_test_db_available() {
        let Ok(url) = std::env::var("JOLT_TEST_DATABASE_URL") else {
            return;
        };
        let cfg = DbConfig::new(url);
        let db = JoltDb::connect(&cfg).await.expect("connect");

        sqlx::query("DROP TABLE IF EXISTS _jolt_tx_rollback_test")
            .execute(db.pool())
            .await
            .expect("drop table (setup)");
        sqlx::query("CREATE TABLE _jolt_tx_rollback_test (id INT)")
            .execute(db.pool())
            .await
            .expect("create table");

        let result = db
            .transaction(|tx| {
                Box::pin(async move {
                    sqlx::query("INSERT INTO _jolt_tx_rollback_test (id) VALUES (2)")
                        .execute(&mut **tx)
                        .await?;
                    // User-defined sentinel error — RowNotFound is the
                    // simplest sqlx::Error variant to construct in-place.
                    Err::<(), _>(sqlx::Error::RowNotFound)
                })
            })
            .await;
        assert!(
            matches!(result, Err(sqlx::Error::RowNotFound)),
            "expected closure's Err to surface unchanged, got {result:?}",
        );

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _jolt_tx_rollback_test")
            .fetch_one(db.pool())
            .await
            .expect("count after rollback");
        assert_eq!(
            count.0, 0,
            "expected rolled-back insert to be invisible after transaction returned Err",
        );

        sqlx::query("DROP TABLE _jolt_tx_rollback_test")
            .execute(db.pool())
            .await
            .expect("drop table (teardown)");
    }

    // ---- JOLT-RS-090: JoltDb::listen_connection ----

    /// Compile-time pin: `db.listen_connection()` resolves to
    /// `Result<sqlx::postgres::PgListener, sqlx::Error>` (decisions 15–17).
    /// The explicit return annotation forces the typecheck — a regression
    /// that wraps the listener in a foreign type (e.g. `tokio_postgres::
    /// Connection`) or changes the error shape would break this build pin
    /// without ever needing a live Postgres.
    #[test]
    fn listen_connection_signature_returns_pg_listener() {
        async fn _pin(db: &JoltDb) -> Result<sqlx::postgres::PgListener, sqlx::Error> {
            db.listen_connection().await
        }
    }

    /// PRD-mandated success-path verification for JOLT-RS-090: "Dedicated
    /// connection opens without error." Env-gated on `JOLT_TEST_DATABASE_URL`
    /// (same convention as 083/084/086/088); without a live Postgres the
    /// test skips trivially so the default `cargo test -p jolt-db` flow
    /// stays runnable.
    ///
    /// Also pins decision 17 by opening two listeners back-to-back: each
    /// call yields its own connection, neither blocks the other. The pool's
    /// regular `health_check` is exercised in between to confirm the pool
    /// path is unaffected by the listener allocations.
    #[tokio::test]
    async fn listen_connection_opens_dedicated_connection_when_test_db_available() {
        let Ok(url) = std::env::var("JOLT_TEST_DATABASE_URL") else {
            return;
        };
        let cfg = DbConfig::new(url);
        let db = JoltDb::connect(&cfg).await.expect("connect");

        let _listener_a = db
            .listen_connection()
            .await
            .expect("first listen_connection should open without error");

        // Pool is unaffected by the listener allocation.
        db.health_check()
            .await
            .expect("pool still healthy after listen_connection");

        let _listener_b = db
            .listen_connection()
            .await
            .expect("second listen_connection should also open without error");
    }

    // ---- JOLT-RS-091: JoltDb::listen ----

    /// Compile-time pin: `db.listen(channel)` resolves to
    /// `Result<impl Stream<Item = Result<PgNotification, sqlx::Error>> +
    /// Unpin, sqlx::Error>` (decision 18). The explicit return annotation
    /// forces the typecheck — a regression that drops the outer `Result`,
    /// changes the per-item shape, swaps in a foreign `Stream` trait, or
    /// removes `Unpin` would break this pin without ever needing a live
    /// Postgres. The `_assert_stream` helper additionally asserts the
    /// returned value satisfies the `Stream<Item = ...>` bound at the
    /// trait level (catches a regression that returns `impl Future`,
    /// `Vec<_>`, or any other type that happens to typecheck as a return
    /// value but breaks the streaming contract).
    #[test]
    fn listen_signature_yields_stream() {
        fn _assert_stream<S: tokio_stream::Stream<Item = Result<sqlx::postgres::PgNotification, sqlx::Error>> + Unpin>(
            _: &S,
        ) {
        }
        async fn _pin(db: &JoltDb) -> Result<(), sqlx::Error> {
            let stream = db.listen("test_ch").await?;
            _assert_stream(&stream);
            Ok(())
        }
    }

    /// PRD-mandated verification for JOLT-RS-091: "listen("test_ch") yields
    /// a Stream." Env-gated on `JOLT_TEST_DATABASE_URL` (same convention as
    /// 083/084/086/088/090): without a live Postgres the test skips
    /// trivially so the default `cargo test -p jolt-db` flow stays runnable.
    ///
    /// With the env var set: calls `listen("_jolt_listen_smoke_ch")` and
    /// asserts the outer `Result` is `Ok` (setup succeeded — the dedicated
    /// connection opened and the `LISTEN` round trip completed). The
    /// returned stream itself is dropped without driving it, which (a)
    /// keeps this slice scoped to the JOLT-RS-091 verification (the
    /// LISTEN/NOTIFY end-to-end notification round-trip is JOLT-RS-093's
    /// closing test) and (b) pins decision 20: a `listen` whose backing
    /// connection is allocated outside the pool drops cleanly without
    /// affecting subsequent pool queries.
    #[tokio::test]
    async fn listen_yields_stream_when_test_db_available() {
        let Ok(url) = std::env::var("JOLT_TEST_DATABASE_URL") else {
            return;
        };
        let cfg = DbConfig::new(url);
        let db = JoltDb::connect(&cfg).await.expect("connect");

        let stream = db
            .listen("_jolt_listen_smoke_ch")
            .await
            .expect("listen on a fresh channel should succeed");

        // Drop the stream explicitly to make the lifecycle test intent
        // visible — the goal is "listen() returns Ok with a Stream", not
        // "we consumed any items from it". JOLT-RS-093 will exercise the
        // notification delivery path.
        drop(stream);

        // Decision 20: the listener uses its own connection, so the pool
        // remains healthy after the listen+drop cycle. Catches a
        // regression that accidentally checks out a pool connection (e.g.
        // by switching `PgListener::connect_with` to a pool-acquire shape).
        db.health_check()
            .await
            .expect("pool still healthy after listen() + drop");
    }
}
