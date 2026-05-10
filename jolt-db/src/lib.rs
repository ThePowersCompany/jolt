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
//!
//! [`JoltDb::notify`] (JOLT-RS-092) is the write side of the LISTEN/NOTIFY
//! pair. Issues a `SELECT pg_notify($1, $2)` through the regular pool so
//! the channel name and payload are both bound parameters rather than
//! interpolated into the SQL text. Symmetric with [`Self::listen`] in error
//! shape (raw [`sqlx::Error`], decision 6) but asymmetric in connection
//! source — `notify` does not need a dedicated connection and uses any
//! pool-checked-out connection because `NOTIFY` is a one-shot write that
//! commits and returns.
//!
//! 21. **`notify` issues `SELECT pg_notify($1, $2)`, not `NOTIFY <ch>,
//!     '<payload>'` (JOLT-RS-092).** Postgres's bare `NOTIFY` statement
//!     does not accept bound parameters for either the channel name (it
//!     requires an identifier literal in the SQL text) or the payload (it
//!     requires a string literal). Building the SQL via `format!` would
//!     either need an allowlist of channel names or a hand-rolled identifier
//!     quoter, both of which re-implement work the
//!     [`pg_notify`](https://www.postgresql.org/docs/current/functions-info.html#FUNCTIONS-INFO-NOTIFY)
//!     function already does correctly. `pg_notify(text, text)` accepts
//!     both arguments as bound parameters, which lets jolt-db hand untrusted
//!     channel/payload strings straight through sqlx's existing parameter-
//!     encoding pipeline with zero injection surface. The semantic effect
//!     is identical to a `NOTIFY` statement.
//!
//! 22. **`notify` runs through the regular pool, not the listener
//!     connection (JOLT-RS-092).** `NOTIFY` is a fire-and-forget write —
//!     the server queues the notification for delivery to LISTEN-ing
//!     subscribers, the producing connection's role ends at commit. Using
//!     the pool means the producer is just another query consumer
//!     contending for pool slots, with no special connection lifecycle to
//!     manage. The dedicated `PgListener` connection (decisions 16–17)
//!     exists to *receive* notifications, which is the half that requires
//!     a long-lived connection holding the LISTEN subscription open.
//!
//! 23. **Returns `Result<(), sqlx::Error>`; the row payload from
//!     `pg_notify` is discarded (JOLT-RS-092).** `pg_notify` returns
//!     `void` (formally a single-row, zero-column result), so there is
//!     nothing meaningful to surface to the caller — success of the round
//!     trip is the whole signal, mirroring [`Self::health_check`]'s shape
//!     (decision 9). The `()` return keeps `notify` ergonomic for
//!     fire-and-forget call sites: `db.notify("orders", &id).await?;`.
//!
//! [`read_migration_files`] (JOLT-RS-094) opens phase22 — the migration
//! file discovery half of the migration pipeline. Returns a list of
//! [`MigrationFile`] records read from a single directory, sorted by
//! filename. JOLT-RS-095 will add a SHA-256 checksum helper; JOLT-RS-096
//! will extend [`MigrationFile`] with the resulting `checksum: String`
//! field; JOLT-RS-098..101 will layer the apply / `_migrations`-table
//! bookkeeping logic on top.
//!
//! 24. **[`MigrationFile`] is a flat `pub`-fields struct alongside
//!     [`DbConfig`] and [`JoltDb`] in lib.rs (JOLT-RS-094).** Matches the
//!     established crate-flat layout — [`DbConfig`], [`JoltDb`], and
//!     [`TypedQuery`] all live at the crate root. Splitting migrations into
//!     a `mod migrations` submodule would force phase22/23 callers to write
//!     `jolt_db::migrations::MigrationFile` instead of
//!     `jolt_db::MigrationFile`, breaking the established single-namespace
//!     import shape callers learned for the other types. The PRD-094 fields
//!     here are the minimum the discovery slice needs (`name`, `content`);
//!     JOLT-RS-096 will add `checksum: String` once JOLT-RS-095 lands the
//!     SHA-256 helper.
//!
//! 25. **[`read_migration_files`] returns `std::io::Result<Vec<...>>`, not
//!     a wrapped [`sqlx::Error`] shape (JOLT-RS-094).** The work is pure
//!     filesystem ([`std::fs::read_dir`] + [`std::fs::read_to_string`]) —
//!     no Postgres round trip happens — so the natural error type is
//!     [`std::io::Error`]. Wrapping it in `sqlx::Error::Io` would force
//!     callers into a `sqlx::Error` match arm for filesystem errors that
//!     have nothing to do with sqlx, and the future `JoltDb::migrate` call
//!     (JOLT-RS-098+) can re-wrap or `?`-chain through a unified migration
//!     error type when that aggregation actually pays for itself. For now,
//!     `std::io::Result<...>` is the right level — it composes cleanly with
//!     every standard-library filesystem helper a caller might layer on top.
//!
//! 26. **Sort is plain [`Vec::sort_by`] on the `name` field (PRD-094
//!     mandate, JOLT-RS-094).** A natural-sort variant would put
//!     `2_foo.sql` before `10_foo.sql`, but the established migration-file
//!     convention (and the PRD-094 verification fixture `001_init.sql`,
//!     `002_users.sql`) uses zero-padded numeric prefixes which
//!     lexicographic sort already orders correctly. Adding a natural-sort
//!     dependency just for this case would be overkill; the lex-sort
//!     contract is simple to understand and works correctly for both
//!     zero-padded-numeric naming and pure-alphabetic naming
//!     (`migration_a.sql`, `migration_b.sql`).
//!
//! 27. **Non-`.sql` entries (and subdirectories) are silently skipped
//!     (JOLT-RS-094, exercised by JOLT-RS-097's closing tests).** A
//!     migration directory commonly contains README files, editor backup
//!     files (`.bak`, `~`), and the like; erroring on the first non-SQL
//!     entry would force every caller to keep the directory perfectly
//!     clean. Skipping is the lower-friction default — an entry that
//!     actually IS a migration but has the wrong extension is a caller-
//!     side mistake that surfaces loudly as "migration not applied" when
//!     JOLT-RS-098+ runs. Subdirectories are also skipped (no recursive
//!     descent) so a `migrations/archive/` subdir holding superseded
//!     scripts does not get re-run.
//!
//! 28. **`read_migration_files(dir: &str)` accepts `&str`, not
//!     `impl AsRef<Path>` (JOLT-RS-094, PRD-094 mandate).** The PRD
//!     signature is explicit. A future overload that accepts an arbitrary
//!     path-like is a forward-compatible addition; making the parameter
//!     generic now would force a turbofish at call sites that want to
//!     pass a string literal without a path conversion, and the
//!     `&str → Path` step inside the function is a single
//!     [`std::path::Path::new`] call with no runtime cost. Matches the
//!     `dir: &str` shape of every existing migration-discovery API the
//!     port is replacing.
//!
//! [`sha256_hex`] (JOLT-RS-095) is the SHA-256 checksum helper the rest
//! of the migration pipeline composes on top of. Takes a byte slice and
//! returns a 64-character lowercase hexadecimal string. JOLT-RS-096 will
//! extend [`MigrationFile`] with a `checksum: String` field populated by
//! calling `sha256_hex(content.as_bytes())` inside
//! [`read_migration_files`]; JOLT-RS-099 will hash the file content again
//! at apply time and compare against the `_migrations` table's stored
//! checksum to detect tampering / skip already-applied migrations.
//!
//! 29. **Free function `sha256_hex(bytes: &[u8]) -> String`, not a method
//!     on [`MigrationFile`] (JOLT-RS-095).** The helper is needed in at
//!     least three call sites — JOLT-RS-096's `read_migration_files`
//!     loop (file body → checksum), JOLT-RS-099's apply path (compare
//!     against `_migrations.checksum`), and any future caller that wants
//!     to hash a precomputed string before constructing a
//!     [`MigrationFile`] by hand. A method on [`MigrationFile`] would be
//!     more discoverable but would couple the hash primitive to that one
//!     struct; making it a free function at the crate root keeps the
//!     primitive composable. The `&[u8]` (rather than `&str`) parameter
//!     widens the input shape to any byte buffer — SQL bodies, JSON, raw
//!     bytes from a file — without forcing callers through a `.as_bytes()`
//!     conversion at most sites and matches the underlying
//!     [`sha2::Sha256::digest`] signature.
//!
//! 30. **Hex encoding is a 4-line hand-rolled loop, not a `hex` crate
//!     dependency (JOLT-RS-095).** The encoding logic for 32 SHA-256
//!     output bytes is trivial — two nibble lookups against a 16-byte
//!     ASCII table — and pulling
//!     [`hex`](https://docs.rs/hex/latest/hex/) into the workspace solely
//!     for this would be the heavier choice (extra crate, MSRV surface,
//!     supply-chain footprint). The hand-rolled loop is fixed-size,
//!     branch-free per nibble, and produces the canonical lowercase
//!     output that `pg_dump` / `git`-style toolchains expect (so the
//!     stored `_migrations.checksum` column lines up byte-for-byte with
//!     whatever a developer might compute via `shasum -a 256` at the
//!     shell). Output length is always exactly 64 ASCII characters
//!     regardless of input length.
//!
//! 31. **Lowercase hex (`0-9a-f`), not uppercase (JOLT-RS-095).** Matches
//!     the convention used by `shasum -a 256`, `openssl dgst -sha256`,
//!     `git`'s blob hashes, and the
//!     [`Display`](https://docs.rs/sha2/latest/sha2/index.html) impl that
//!     `sha2` itself ships. Mixing cases between the recorded
//!     `_migrations.checksum` value and a developer's manually-computed
//!     reference hash would surface as a false-positive "migration
//!     tampered with" failure at apply time (JOLT-RS-099) even when the
//!     bytes are identical. Pinning lowercase here eliminates the
//!     ambiguity.

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

    /// Publish a notification to a Postgres `LISTEN` channel
    /// (JOLT-RS-092).
    ///
    /// Issues `SELECT pg_notify($1, $2)` through the regular pool with
    /// `channel` and `payload` bound as parameters (decision 21). The
    /// channel name is propagated unmodified to `pg_notify`, which handles
    /// quoting and identifier semantics; the payload is sent as an opaque
    /// UTF-8 string. Subscribers via [`Self::listen`] (or
    /// [`Self::listen_connection`]) receive the notification on the same
    /// channel.
    ///
    /// Returns `Ok(())` on a successful round trip, or the raw
    /// [`sqlx::Error`] on any failure (acquire timeout, connection drop,
    /// etc.). The `pg_notify` row payload (`void`) is discarded
    /// (decision 23).
    ///
    /// # Example
    ///
    /// ```ignore
    /// db.notify("orders", "ord_123").await?;
    /// ```
    pub async fn notify(&self, channel: &str, payload: &str) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(channel)
            .bind(payload)
            .execute(&self.pool)
            .await?;
        Ok(())
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

/// A migration file discovered by [`read_migration_files`] (JOLT-RS-094).
///
/// Holds the bare metadata phase22's sort-by-filename discovery needs: the
/// basename (no directory prefix) and the file's UTF-8 body. See module docs
/// decision 24 for the flat-`pub`-fields layout rationale; JOLT-RS-096 will
/// extend this struct with `checksum: String` once JOLT-RS-095 lands the
/// SHA-256 helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationFile {
    /// Filename (basename, no directory prefix). Used as the sort key in
    /// [`read_migration_files`] and as the identifier the eventual
    /// `_migrations` bookkeeping table (JOLT-RS-098) will record.
    pub name: String,
    /// File contents read as UTF-8. The full SQL body that the apply step
    /// (JOLT-RS-100) will execute inside a transaction.
    pub content: String,
}

/// Discover migration files in `dir` (JOLT-RS-094).
///
/// Reads every entry in `dir`, retains files whose name ends in `.sql`,
/// reads each retained file's contents as UTF-8, and returns the resulting
/// [`MigrationFile`] values sorted lexicographically by filename. See module
/// docs decisions 24–28 for the architectural contract.
///
/// Returns `Err(std::io::Error)` if `dir` is not readable (missing,
/// permission denied, not a directory) or if any individual `.sql` file
/// fails to read as UTF-8. Subdirectories, non-`.sql` files, and entries
/// whose filename is not valid UTF-8 are silently skipped (decision 27).
///
/// # Example
///
/// ```ignore
/// let files = jolt_db::read_migration_files("./migrations")?;
/// for f in &files {
///     println!("{}: {} bytes", f.name, f.content.len());
/// }
/// ```
pub fn read_migration_files(dir: &str) -> std::io::Result<Vec<MigrationFile>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(std::path::Path::new(dir))? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        match path.extension().and_then(|e| e.to_str()) {
            Some("sql") => {}
            _ => continue,
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_owned(),
            None => continue,
        };
        let content = std::fs::read_to_string(&path)?;
        files.push(MigrationFile { name, content });
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

/// Hash `bytes` with SHA-256 and return the digest as a 64-character
/// lowercase hexadecimal string (JOLT-RS-095).
///
/// See module docs decisions 29–31 for the architectural contract:
/// free function (not a method on [`MigrationFile`]) so the primitive
/// composes for both the JOLT-RS-096 file-discovery path and the
/// JOLT-RS-099 apply-time tamper check; hand-rolled hex (no `hex` crate
/// dependency); lowercase output to match `shasum -a 256` / `git` /
/// `openssl dgst -sha256` conventions.
///
/// # Example
///
/// ```
/// // NIST SHA-256 test vector for the ASCII input "abc".
/// assert_eq!(
///     jolt_db::sha256_hex(b"abc"),
///     "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
/// );
/// ```
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in digest.iter() {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        DbConfig, JoltDb, MigrationFile, TypedQuery, read_migration_files, sha256_hex,
    };

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

    // ---- JOLT-RS-092: JoltDb::notify ----

    /// Compile-time pin: `db.notify(&str, &str)` resolves to
    /// `Result<(), sqlx::Error>` (decisions 21–23). The explicit return
    /// annotation forces the typecheck — a regression that surfaces the
    /// `pg_notify` row payload, wraps the error in a custom enum, or
    /// changes the parameter shape would break this build pin without
    /// ever needing a live Postgres.
    #[test]
    fn notify_signature_returns_unit_result() {
        async fn _pin(db: &JoltDb) -> Result<(), sqlx::Error> {
            db.notify("test_ch", "hello").await
        }
    }

    /// PRD-mandated verification for JOLT-RS-092: "notify("test_ch",
    /// "hello") succeeds." Env-gated on `JOLT_TEST_DATABASE_URL` (same
    /// convention as 083/084/086/088/090/091): without a live Postgres
    /// the test skips trivially so the default `cargo test -p jolt-db`
    /// flow stays runnable.
    ///
    /// Calls `notify("_jolt_notify_smoke_ch", "hello")` and asserts the
    /// `Result` is `Ok(())`. Notification delivery (i.e. that a
    /// concurrent `LISTEN`-er actually receives this payload) is
    /// JOLT-RS-093's closing-test slice; here we only verify the write
    /// half round-trips. Pool health is checked afterward to pin
    /// decision 22 (notify uses the regular pool, not a dedicated
    /// connection — an accidental switch to a long-lived connection
    /// would still pass this test individually, but the back-to-back
    /// invocations below would saturate a pool whose `max_connections`
    /// shrunk to a single listener slot).
    #[tokio::test]
    async fn notify_succeeds_when_test_db_available() {
        let Ok(url) = std::env::var("JOLT_TEST_DATABASE_URL") else {
            return;
        };
        let cfg = DbConfig::new(url);
        let db = JoltDb::connect(&cfg).await.expect("connect");

        db.notify("_jolt_notify_smoke_ch", "hello")
            .await
            .expect("notify(test_ch, hello) should succeed against live Postgres");

        // Run a second notify back-to-back to confirm the pool is not
        // serialized behind a single notify-owned connection (decision 22).
        db.notify("_jolt_notify_smoke_ch", "world")
            .await
            .expect("second notify should succeed without contention");

        db.health_check()
            .await
            .expect("pool still healthy after notify");
    }

    /// Decision 21 explicitly: the channel name and payload are bound
    /// parameters, not interpolated SQL text. A payload containing a
    /// single quote (which would terminate a string literal in raw
    /// `NOTIFY ch, '...'` SQL) round-trips without error because sqlx
    /// encodes the bind value through the wire protocol rather than
    /// substituting it into the SQL string. Env-gated.
    #[tokio::test]
    async fn notify_handles_single_quote_payload_when_test_db_available() {
        let Ok(url) = std::env::var("JOLT_TEST_DATABASE_URL") else {
            return;
        };
        let cfg = DbConfig::new(url);
        let db = JoltDb::connect(&cfg).await.expect("connect");

        // A payload that would SQL-inject a raw `NOTIFY ch, '<payload>'`
        // form. With `pg_notify($1, $2)` it's just a string literal on
        // the wire and round-trips cleanly.
        db.notify("_jolt_notify_smoke_ch", "it's safe")
            .await
            .expect("single-quote payload should round-trip via parameter binding");
    }

    // ---- JOLT-RS-094: read_migration_files ----

    /// Self-cleaning temp directory used by the `read_migration_files`
    /// tests. Each instance lives in `std::env::temp_dir()` under a
    /// PID + process-local atomic-counter name so concurrent test threads
    /// don't collide, and the directory is removed on `Drop`. Tests use
    /// the `path_str` accessor to feed the directory's UTF-8 path straight
    /// into [`read_migration_files`] (decision 28).
    struct TestDir {
        path: std::path::PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "jolt-db-094-{}-{}-{}",
                std::process::id(),
                label,
                n,
            ));
            // Best-effort cleanup of any stale directory left by a prior
            // crashed run with the same PID+counter (unlikely with the
            // atomic counter, but free insurance).
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir(&path).expect("create test dir");
            Self { path }
        }

        fn write_file(&self, name: &str, content: &str) {
            std::fs::write(self.path.join(name), content).expect("write file");
        }

        fn path_str(&self) -> &str {
            self.path
                .to_str()
                .expect("std::env::temp_dir() path is UTF-8")
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// Compile-time pin: `read_migration_files(dir: &str)` resolves to
    /// `std::io::Result<Vec<MigrationFile>>` (decisions 25, 28). The
    /// explicit return annotation forces the typecheck — a regression that
    /// switches the parameter to `&Path` / `impl AsRef<Path>` or wraps the
    /// return in a foreign error type breaks this build pin without ever
    /// needing a real directory.
    #[test]
    fn read_migration_files_signature_pins() {
        fn _pin(dir: &str) -> std::io::Result<Vec<MigrationFile>> {
            read_migration_files(dir)
        }
    }

    /// PRD-mandated verification for JOLT-RS-094: two files
    /// `001_init.sql` and `002_users.sql` read back as a Vec sorted
    /// lexicographically by filename → `[001_..., 002_...]`. Writes the
    /// `002_` file first so a sortless implementation that returned
    /// `read_dir`'s incidental order would put `002_` ahead of `001_`
    /// and fail this test on most platforms.
    #[test]
    fn read_migration_files_sorts_by_filename() {
        let dir = TestDir::new("sort");
        dir.write_file("002_users.sql", "CREATE TABLE users();");
        dir.write_file("001_init.sql", "CREATE TABLE init();");

        let files = read_migration_files(dir.path_str()).expect("read");
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["001_init.sql", "002_users.sql"]);
    }

    /// Each [`MigrationFile`] captures both the basename and the file's
    /// UTF-8 body (decision 24). Pins the field-shape contract — a
    /// regression that captures only the name, strips the file content,
    /// or returns a non-UTF-8 buffer fails this test.
    #[test]
    fn read_migration_files_captures_name_and_content() {
        let dir = TestDir::new("body");
        dir.write_file("001_init.sql", "SELECT 1;");

        let files = read_migration_files(dir.path_str()).expect("read");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "001_init.sql");
        assert_eq!(files[0].content, "SELECT 1;");
    }

    // ---- JOLT-RS-095: sha256_hex ----

    /// Compile-time pin: `sha256_hex(bytes: &[u8])` resolves to a
    /// `String` (decisions 29, 30). A regression that narrows the
    /// parameter to `&str` or returns a `Vec<u8>` / `[u8; 32]` breaks
    /// this build pin without ever running.
    #[test]
    fn sha256_hex_signature_pins() {
        fn _pin(bytes: &[u8]) -> String {
            sha256_hex(bytes)
        }
    }

    /// PRD-mandated verification for JOLT-RS-095: a known input hashes
    /// to the documented SHA-256 hex output. Uses the canonical NIST
    /// test vector for `"abc"` — anyone can re-derive this via
    /// `echo -n abc | shasum -a 256`, so a regression in either the
    /// hash kernel or the hex encoder surfaces against a fixed
    /// independently-verifiable reference.
    #[test]
    fn sha256_hex_matches_nist_abc_test_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        );
    }

    /// SHA-256 of the empty input is the canonical
    /// `e3b0c4...` constant. Pins both that the hex output is exactly
    /// 64 characters (no truncation, no padding) and that the helper
    /// handles zero-length input without panicking.
    #[test]
    fn sha256_hex_matches_empty_input_constant() {
        let out = sha256_hex(b"");
        assert_eq!(
            out,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        );
        assert_eq!(out.len(), 64);
    }

    /// Output is lowercase hex, not uppercase (decision 31). Pinned so
    /// a regression that flips to `{:02X}` formatting fails this test —
    /// such a flip would silently desync the recorded
    /// `_migrations.checksum` from a developer's `shasum -a 256`
    /// reference value when JOLT-RS-099's apply-time comparison runs.
    #[test]
    fn sha256_hex_is_lowercase() {
        let out = sha256_hex(b"abc");
        assert!(
            out.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "expected lowercase hex digits, got {out:?}",
        );
    }

    /// Hashing the SQL body of a migration file produces a stable hex
    /// digest that the JOLT-RS-096 `read_migration_files` extension
    /// (and the JOLT-RS-099 apply-time tamper check) can record and
    /// compare against. Reference value computed via
    /// `echo -n 'SELECT 1;' | shasum -a 256`.
    #[test]
    fn sha256_hex_hashes_migration_body() {
        assert_eq!(
            sha256_hex(b"SELECT 1;"),
            "17db4fd369edb9244b9f91d9aeed145c3d04ad8ba6e95d06247f07a63527d11a",
        );
    }
}
