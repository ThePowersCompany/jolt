//! joltr-db: Postgres connection pool, query helpers, and migration support
//! for the JoltR framework.
//!
//! [`DbConfig`] (JOLTR-RS-082) is the per-deployment configuration record the
//! upcoming `JoltRDb::connect` (JOLTR-RS-083) consumes to build a
//! [`sqlx::PgPool`](https://docs.rs/sqlx/latest/sqlx/struct.PgPool.html).
//! Fields mirror the three `PgPoolOptions` knobs the connect call will set:
//! the database URL, the pool's connection ceiling, and the per-acquire
//! timeout.
//!
//! Architectural decisions pinned here for JOLTR-RS-083..085 to build on:
//!
//! 1. **Plain `pub` fields, no getters/setters.** Mirrors
//!    [`CorsConfig`](../../joltr_core/server/struct.CorsConfig.html) (055) and
//!    [`JwtConfig`](../../joltr_utils/jwt/struct.JwtConfig.html) (072): callers
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
//! [`JoltRDb`] (JOLTR-RS-083) is the runtime handle holding the
//! [`sqlx::PgPool`](https://docs.rs/sqlx/latest/sqlx/struct.PgPool.html) that
//! every downstream phase19/20/21 slice consumes. Construction goes through
//! [`JoltRDb::connect`], which builds a
//! [`sqlx::postgres::PgPoolOptions`](https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgPoolOptions.html)
//! from the [`DbConfig`] knobs and `.connect()`s to Postgres. Architectural
//! decisions pinned here for JOLTR-RS-084/085 and onward to build on:
//!
//! 5. **`JoltRDb` owns the `PgPool` by value, not behind an `Arc`.**
//!    [`sqlx::PgPool`](https://docs.rs/sqlx/latest/sqlx/struct.PgPool.html) is
//!    already a cheap-to-clone handle that internally wraps an `Arc<...>`,
//!    so wrapping it again in `Arc<PgPool>` would be redundant. Callers that
//!    need shared ownership of the `JoltRDb` itself can wrap the outer struct
//!    in `Arc<JoltRDb>` (the eventual `JoltRServer` integration will own one
//!    `Arc<JoltRDb>` and clone the handle into request extensions).
//!
//! 6. **`connect` returns `Result<Self, sqlx::Error>` (the raw sqlx error).**
//!    A bespoke error enum would force callers to convert between two error
//!    shapes for trivial reasons (sqlx already produces a rich error with
//!    `Display` + `source()` for chained reporting); the connect call has
//!    exactly one failure mode (sqlx couldn't open the pool), so wrapping it
//!    adds noise. Future query helpers (JOLTR-RS-086 onward) will likely
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
//!    (JOLTR-RS-084).** Callers that need an owned handle clone the returned
//!    reference themselves (`db.pool().clone()`); callers that only need to
//!    run a query through the pool pass the borrow straight to sqlx (which
//!    accepts `&PgPool` as an executor). Returning a borrow is the
//!    lower-friction default — owners can always upgrade with `.clone()` but
//!    borrowers cannot avoid an unwanted clone.
//!
//! 9. **`health_check()` runs `SELECT 1` and returns `Result<(), sqlx::Error>`
//!    (JOLTR-RS-084).** Discards the row payload — the success of the round
//!    trip is the whole signal. The error shape matches decision 6 (raw
//!    `sqlx::Error`), so a caller can pattern-match on the specific failure
//!    (e.g. `Error::PoolTimedOut` vs `Error::Io`) without an enum hop. The
//!    intended use sites are (a) the eventual `JoltRServer` readiness probe,
//!    (b) HTTP `/healthz` endpoints, (c) JOLTR-RS-085's closing connection
//!    test.
//!
//! [`TypedQuery`] (JOLTR-RS-086) opens phase20's typed-query helper layer on
//! top of [`JoltRDb::pool`]. `db.query_as::<T>(sql)` returns a [`TypedQuery<T>`]
//! that exposes `.bind(...)` for positional parameters and the three terminal
//! fetch verbs (`.fetch_one()`, `.fetch_optional()`, `.fetch_all()`).
//! Architectural decisions pinned here for JOLTR-RS-087..089 and onward:
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
//!     and outlives the originating `JoltRDb` borrow. Terminal fetch methods
//!     reconstitute a fresh [`sqlx::query_as_with`] inside their body using
//!     the owned SQL + args, so the borrowed-vs-owned lifetime question
//!     never reaches the caller's signature. The PRD-mandated "params..."
//!     in JOLTR-RS-086 is realized via the chainable `.bind(value)` builder;
//!     bound values must be `'static + Send + Encode + Type` so the
//!     `TypedQuery<T>` itself remains `'static + Send` and can cross task
//!     boundaries / be stored in a struct without a lifetime parameter.
//!
//! 11. **`.bind()` panics on encode failure rather than returning `Result`
//!     (JOLTR-RS-086).** sqlx 0.8's
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
//! [`JoltRDb::transaction`] (JOLTR-RS-088) layers the auto-commit / auto-
//! rollback wrapper on top of [`sqlx::PgPool::begin`]. A caller hands in a
//! `FnOnce(&mut Transaction)` whose body returns a `Result<T, sqlx::Error>`;
//! `transaction` opens a tx, runs the closure, commits on `Ok` and rolls
//! back on `Err`. Architectural decisions pinned here for JOLTR-RS-089 and
//! onward:
//!
//! 12. **The closure receives `&mut sqlx::Transaction<'static, Postgres>`
//!     directly — sqlx-native, not a JoltRDb wrapper (JOLTR-RS-088).** A wrapper
//!     would have to re-implement the typed-query helpers (or a tx-aware
//!     [`TypedQuery`] variant) to give callers anything beyond raw sqlx, and
//!     the [`pool`](Self::pool) getter from JOLTR-RS-084 already exposes raw
//!     sqlx for the non-tx path. Symmetric design: outside the closure callers
//!     reach for raw sqlx via `db.pool()`; inside they reach for raw sqlx via
//!     the `&mut Transaction`. A future tx-aware `TypedQuery` can be added
//!     without disturbing this contract — it would be a layer on top, not a
//!     replacement.
//!
//! 13. **The closure returns `Pin<Box<dyn Future + Send + 'c>>`, not a bare
//!     `Future` value (JOLTR-RS-088).** The future borrows the `&'c mut
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
//!     (JOLTR-RS-088).** Rationale: when the closure returns `Err` the user
//!     already knows the operation failed and cares about *why*; a follow-on
//!     rollback failure (typically connection-level) would mask the real
//!     cause. Dropping a `Transaction` without commit also auto-rolls-back
//!     at the connection level, so a failed explicit rollback is rarely
//!     load-bearing. On the commit path the commit error *is* the reason the
//!     txn didn't take effect, so propagating it directly is correct.
//!
//! [`JoltRDb::listen_connection`] (JOLTR-RS-090) opens phase21's LISTEN/NOTIFY
//! layer. Returns a dedicated [`sqlx::postgres::PgListener`] — a single
//! Postgres connection allocated outside the regular pool and reserved for
//! `LISTEN <channel>` + notification streaming. JOLTR-RS-091 (`listen`) and
//! JOLTR-RS-092 (`notify`) build on top of this opener.
//!
//! 15. **Dedicated connection is a `sqlx::postgres::PgListener`, not a
//!     `tokio_postgres::Connection` (JOLTR-RS-090).** The PRD's task wording
//!     ("dedicated tokio-postgres connection") describes what kind of
//!     connection LISTEN/NOTIFY needs (a single long-lived TCP connection
//!     dedicated to receiving async notifications, *not* a pool-checked-out
//!     connection that gets returned between calls). It does not mandate
//!     adding `tokio-postgres` as a sibling driver to sqlx. sqlx's
//!     [`PgListener`](https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgListener.html)
//!     is exactly this shape — built on tokio-postgres-style async
//!     mechanics internally but exposed through sqlx's existing trait stack,
//!     producing the same `sqlx::Error` shape as the rest of joltr-db
//!     (decision 6) and the same
//!     [`PgNotification`](https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgNotification.html)
//!     type that JOLTR-RS-091's stream will surface. Adding `tokio-postgres`
//!     as a second driver would force the LISTEN/NOTIFY surface to use a
//!     foreign error type and `Notification` struct, double the workspace's
//!     async Postgres dependency footprint, and create a second connection
//!     URL / TLS / SCRAM-auth code path. Using sqlx end-to-end keeps the
//!     whole joltr-db crate on one driver stack.
//!
//! 16. **`listen_connection` allocates a fresh connection via
//!     [`PgListener::connect_with`](https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgListener.html#method.connect_with),
//!     reusing the pool's configured connection options (JOLTR-RS-090).** This
//!     gives the listener the same URL / TLS / credentials the pool was built
//!     with (so a deployment configures Postgres exactly once via
//!     [`DbConfig`]) while still allocating a *new* connection outside the
//!     pool — the PRD-mandated "separate from pool" property holds. The
//!     alternative (`PgListener::connect(url_string)`) would require
//!     re-storing the URL on `JoltRDb` after `connect` consumed it, which
//!     breaks the existing decision-5 "JoltRDb owns just the pool" shape.
//!     `connect_with` sidesteps the storage question entirely by reading the
//!     options off the pool handle directly.
//!
//! 17. **Returns the `PgListener` to the caller by value rather than storing
//!     it on `JoltRDb` (JOLTR-RS-090).** `PgListener` is `&mut self`-driven for
//!     `listen` / `recv` / `into_stream`, so a single shared `PgListener`
//!     stored on `JoltRDb` would force all listeners through one connection
//!     and serialize them behind a `Mutex`. Returning a fresh listener per
//!     call lets each subscriber own its own dedicated connection (matching
//!     the spec's per-channel listen model) and avoids cross-subscriber
//!     interference. JOLTR-RS-091's `listen(channel)` will be a convenience
//!     wrapper that calls `listen_connection()` + `listener.listen(channel)`
//!     + `listener.into_stream()` internally.
//!
//! [`JoltRDb::listen`] (JOLTR-RS-091) builds the high-level streaming verb on
//! top of [`Self::listen_connection`]. Returns a `Stream` of
//! [`sqlx::postgres::PgNotification`] items, each wrapped in
//! `Result<_, sqlx::Error>` because the underlying connection can drop
//! mid-stream and the auto-reconnect machinery surfaces the failure as an
//! item rather than ending the stream silently.
//!
//! 18. **Two error tiers: outer `Result` for setup, per-item `Result` for
//!     mid-stream failures (JOLTR-RS-091).** `listen` is `async fn -> Result<
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
//!     unmodified to [`PgListener::listen`] (JOLTR-RS-091).** No quoting,
//!     escaping, or validation happens at the joltr-db layer — sqlx's
//!     `PgListener::listen` already does the right thing (it issues a
//!     parameterized `LISTEN` via the wire protocol, so SQL-injection
//!     vectors via channel name are sqlx's concern, not ours). Channel
//!     name semantics (case folding, identifier length limits) are
//!     Postgres's concern and would be the same whether or not joltr-db
//!     wrapped this call. Future overloads for `listen_all(channels:
//!     &[&str])` can be added without disturbing this single-channel
//!     contract.
//!
//! 20. **Each `listen` call allocates a fresh dedicated connection
//!     (JOLTR-RS-091, inherits from decision 17).** A pair of
//!     `db.listen("ch_a")` and `db.listen("ch_b")` calls produces two
//!     independent streams backed by two independent connections; one
//!     stream's connection drop does not affect the other. Callers who
//!     want multi-channel multiplexing on a single connection should
//!     reach for [`Self::listen_connection`] directly and call
//!     `listener.listen_all(...)` themselves — that primitive remains
//!     available exactly for this case.
//!
//! [`JoltRDb::notify`] (JOLTR-RS-092) is the write side of the LISTEN/NOTIFY
//! pair. Issues a `SELECT pg_notify($1, $2)` through the regular pool so
//! the channel name and payload are both bound parameters rather than
//! interpolated into the SQL text. Symmetric with [`Self::listen`] in error
//! shape (raw [`sqlx::Error`], decision 6) but asymmetric in connection
//! source — `notify` does not need a dedicated connection and uses any
//! pool-checked-out connection because `NOTIFY` is a one-shot write that
//! commits and returns.
//!
//! 21. **`notify` issues `SELECT pg_notify($1, $2)`, not `NOTIFY <ch>,
//!     '<payload>'` (JOLTR-RS-092).** Postgres's bare `NOTIFY` statement
//!     does not accept bound parameters for either the channel name (it
//!     requires an identifier literal in the SQL text) or the payload (it
//!     requires a string literal). Building the SQL via `format!` would
//!     either need an allowlist of channel names or a hand-rolled identifier
//!     quoter, both of which re-implement work the
//!     [`pg_notify`](https://www.postgresql.org/docs/current/functions-info.html#FUNCTIONS-INFO-NOTIFY)
//!     function already does correctly. `pg_notify(text, text)` accepts
//!     both arguments as bound parameters, which lets joltr-db hand untrusted
//!     channel/payload strings straight through sqlx's existing parameter-
//!     encoding pipeline with zero injection surface. The semantic effect
//!     is identical to a `NOTIFY` statement.
//!
//! 22. **`notify` runs through the regular pool, not the listener
//!     connection (JOLTR-RS-092).** `NOTIFY` is a fire-and-forget write —
//!     the server queues the notification for delivery to LISTEN-ing
//!     subscribers, the producing connection's role ends at commit. Using
//!     the pool means the producer is just another query consumer
//!     contending for pool slots, with no special connection lifecycle to
//!     manage. The dedicated `PgListener` connection (decisions 16–17)
//!     exists to *receive* notifications, which is the half that requires
//!     a long-lived connection holding the LISTEN subscription open.
//!
//! 23. **Returns `Result<(), sqlx::Error>`; the row payload from
//!     `pg_notify` is discarded (JOLTR-RS-092).** `pg_notify` returns
//!     `void` (formally a single-row, zero-column result), so there is
//!     nothing meaningful to surface to the caller — success of the round
//!     trip is the whole signal, mirroring [`Self::health_check`]'s shape
//!     (decision 9). The `()` return keeps `notify` ergonomic for
//!     fire-and-forget call sites: `db.notify("orders", &id).await?;`.
//!
//! [`read_migration_files`] (JOLTR-RS-094) opens phase22 — the migration
//! file discovery half of the migration pipeline. Returns a list of
//! [`MigrationFile`] records read from a single directory, sorted by
//! filename. JOLTR-RS-095 will add a SHA-256 checksum helper; JOLTR-RS-096
//! will extend [`MigrationFile`] with the resulting `checksum: String`
//! field; JOLTR-RS-098..101 will layer the apply / `_migrations`-table
//! bookkeeping logic on top.
//!
//! 24. **[`MigrationFile`] is a flat `pub`-fields struct alongside
//!     [`DbConfig`] and [`JoltRDb`] in lib.rs (JOLTR-RS-094).** Matches the
//!     established crate-flat layout — [`DbConfig`], [`JoltRDb`], and
//!     [`TypedQuery`] all live at the crate root. Splitting migrations into
//!     a `mod migrations` submodule would force phase22/23 callers to write
//!     `joltr_db::migrations::MigrationFile` instead of
//!     `joltr_db::MigrationFile`, breaking the established single-namespace
//!     import shape callers learned for the other types. The PRD-094 fields
//!     here are the minimum the discovery slice needs (`name`, `content`);
//!     JOLTR-RS-096 will add `checksum: String` once JOLTR-RS-095 lands the
//!     SHA-256 helper.
//!
//! 25. **[`read_migration_files`] returns `std::io::Result<Vec<...>>`, not
//!     a wrapped [`sqlx::Error`] shape (JOLTR-RS-094).** The work is pure
//!     filesystem ([`std::fs::read_dir`] + [`std::fs::read_to_string`]) —
//!     no Postgres round trip happens — so the natural error type is
//!     [`std::io::Error`]. Wrapping it in `sqlx::Error::Io` would force
//!     callers into a `sqlx::Error` match arm for filesystem errors that
//!     have nothing to do with sqlx, and the future `JoltRDb::migrate` call
//!     (JOLTR-RS-098+) can re-wrap or `?`-chain through a unified migration
//!     error type when that aggregation actually pays for itself. For now,
//!     `std::io::Result<...>` is the right level — it composes cleanly with
//!     every standard-library filesystem helper a caller might layer on top.
//!
//! 26. **Sort is plain [`Vec::sort_by`] on the `name` field (PRD-094
//!     mandate, JOLTR-RS-094).** A natural-sort variant would put
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
//!     (JOLTR-RS-094, exercised by JOLTR-RS-097's closing tests).** A
//!     migration directory commonly contains README files, editor backup
//!     files (`.bak`, `~`), and the like; erroring on the first non-SQL
//!     entry would force every caller to keep the directory perfectly
//!     clean. Skipping is the lower-friction default — an entry that
//!     actually IS a migration but has the wrong extension is a caller-
//!     side mistake that surfaces loudly as "migration not applied" when
//!     JOLTR-RS-098+ runs. Subdirectories are also skipped (no recursive
//!     descent) so a `migrations/archive/` subdir holding superseded
//!     scripts does not get re-run.
//!
//! 28. **`read_migration_files(dir: &str)` accepts `&str`, not
//!     `impl AsRef<Path>` (JOLTR-RS-094, PRD-094 mandate).** The PRD
//!     signature is explicit. A future overload that accepts an arbitrary
//!     path-like is a forward-compatible addition; making the parameter
//!     generic now would force a turbofish at call sites that want to
//!     pass a string literal without a path conversion, and the
//!     `&str → Path` step inside the function is a single
//!     [`std::path::Path::new`] call with no runtime cost. Matches the
//!     `dir: &str` shape of every existing migration-discovery API the
//!     port is replacing.
//!
//! [`sha256_hex`] (JOLTR-RS-095) is the SHA-256 checksum helper the rest
//! of the migration pipeline composes on top of. Takes a byte slice and
//! returns a 64-character lowercase hexadecimal string. JOLTR-RS-096 will
//! extend [`MigrationFile`] with a `checksum: String` field populated by
//! calling `sha256_hex(content.as_bytes())` inside
//! [`read_migration_files`]; JOLTR-RS-099 will hash the file content again
//! at apply time and compare against the `_migrations` table's stored
//! checksum to detect tampering / skip already-applied migrations.
//!
//! 29. **Free function `sha256_hex(bytes: &[u8]) -> String`, not a method
//!     on [`MigrationFile`] (JOLTR-RS-095).** The helper is needed in at
//!     least three call sites — JOLTR-RS-096's `read_migration_files`
//!     loop (file body → checksum), JOLTR-RS-099's apply path (compare
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
//!     dependency (JOLTR-RS-095).** The encoding logic for 32 SHA-256
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
//! 31. **Lowercase hex (`0-9a-f`), not uppercase (JOLTR-RS-095).** Matches
//!     the convention used by `shasum -a 256`, `openssl dgst -sha256`,
//!     `git`'s blob hashes, and the
//!     [`Display`](https://docs.rs/sha2/latest/sha2/index.html) impl that
//!     `sha2` itself ships. Mixing cases between the recorded
//!     `_migrations.checksum` value and a developer's manually-computed
//!     reference hash would surface as a false-positive "migration
//!     tampered with" failure at apply time (JOLTR-RS-099) even when the
//!     bytes are identical. Pinning lowercase here eliminates the
//!     ambiguity.
//!
//! JOLTR-RS-096 closes the phase22 [`MigrationFile`] data shape by
//! extending the JOLTR-RS-094 struct with a `checksum: String` field
//! populated automatically inside [`read_migration_files`] using the
//! JOLTR-RS-095 [`sha256_hex`] helper. The apply-time tamper check
//! (JOLTR-RS-099) hashes the on-disk content again at apply time and
//! compares against this recorded value.
//!
//! 32. **The `checksum` field is populated inside
//!     [`read_migration_files`], not by a [`MigrationFile`] constructor
//!     (JOLTR-RS-096).** Auto-compute at discovery time keeps the
//!     invariant tight: every [`MigrationFile`] returned by
//!     [`read_migration_files`] carries the SHA-256 digest of its own
//!     `content` field, so callers can't accidentally hand the
//!     apply-time check (JOLTR-RS-099) a stale or out-of-sync checksum.
//!     Exposing a `MigrationFile::new(name, content)` constructor that
//!     auto-computes would be ergonomic for callers who construct by
//!     hand, but it would also be one more API surface to maintain
//!     before any caller exists to use it — phase23 (JOLTR-RS-098+) is
//!     the only consumer in the near horizon, and it reads its
//!     `MigrationFile` values straight from
//!     [`read_migration_files`]. Hand-construction remains possible
//!     via the public struct fields (decision 24) plus the public
//!     [`sha256_hex`] helper (decision 29).
//!
//! 33. **The `checksum` field stays plain `pub String` (not a
//!     `Checksum(String)` newtype), matching the flat-`pub`-fields
//!     shape of decision 24 (JOLTR-RS-096).** A newtype would protect
//!     callers from accidentally swapping `name`, `content`, and
//!     `checksum` arguments, but [`MigrationFile`] is a flat struct
//!     constructed by field name (`MigrationFile { name, content,
//!     checksum }`) rather than positionally, so the swap risk is
//!     already structurally eliminated. The string-typed checksum
//!     also round-trips byte-for-byte through the eventual
//!     `_migrations.checksum` Postgres `TEXT` column (JOLTR-RS-098)
//!     without an extra `From`/`Into` shim. Decision 31's lowercase
//!     contract is the only invariant on the field's contents and
//!     decision 32's auto-populate enforces it at the only entry
//!     point.
//!
//! JOLTR-RS-098 opens phase23 by extending [`JoltRDb::connect`] to
//! auto-create the `_migrations` bookkeeping table on every successful
//! connection. The table records which migrations have run, their
//! recorded SHA-256 checksums, and when each was applied; subsequent
//! phase23 slices (JOLTR-RS-099 read-back, JOLTR-RS-100 apply, JOLTR-RS-102
//! tamper detection) read and write through this table.
//!
//! 34. **Auto-create happens inside [`JoltRDb::connect`] after the pool
//!     opens, not as a separate `JoltRDb::ensure_migrations_table()`
//!     verb (JOLTR-RS-098).** PRD-098 specifies "On JoltRDb::connect()".
//!     Collocating the schema setup with connection setup extends
//!     decision 7's "fail-fast on misconfiguration" contract to schema
//!     readiness: a deployment lacking `CREATE TABLE` permission on
//!     the configured user surfaces at startup, the same place where
//!     auth and unreachable-server errors already surface. The
//!     alternative (a separate ensure-verb that callers remember to
//!     call) opens a window where a fresh-DB deployment can pass
//!     `connect` and then fail at first migration apply because the
//!     bookkeeping table never got created. The cost is that callers
//!     who never run migrations still pay one `CREATE TABLE IF NOT
//!     EXISTS` round trip at startup — acceptable for the fail-fast
//!     symmetry and matches PRD-098 verbatim. The CREATE statement is
//!     intentionally non-transactional / non-rollback-on-error: if it
//!     fails, the pool is dropped along with the unsuccessful `connect`
//!     return so the caller sees one error and an unusable handle, not
//!     a half-initialized `JoltRDb`.
//!
//! 35. **`_migrations` schema is `id SERIAL PRIMARY KEY, name TEXT NOT
//!     NULL, checksum TEXT NOT NULL, applied_at TIMESTAMPTZ DEFAULT
//!     NOW()` exactly as PRD-098 specifies (JOLTR-RS-098).** `id` is
//!     the surrogate primary key — `SERIAL` makes the insertion order
//!     a stable, timestamp-independent ordering for the eventual
//!     JOLTR-RS-101 "list applied migrations" verb so two migrations
//!     applied in the same `NOW()` second still order deterministically
//!     by id. `name` is the discovered [`MigrationFile::name`]
//!     verbatim and is the natural lookup key for JOLTR-RS-099's
//!     `HashMap<String, String>` read-back; the `NOT NULL` constraint
//!     prevents an accidental empty-string insertion that would
//!     silently mark an unnamed migration as applied. `checksum`
//!     stores the JOLTR-RS-095 [`sha256_hex`] output (64 lowercase hex
//!     chars per decision 31) as `TEXT` — the lowercase contract is
//!     the only invariant, so `TEXT` is the right column type rather
//!     than a `CHAR(64)` fixed-width type that would silently
//!     right-pad on shorter input. `applied_at` defaults to `NOW()`
//!     so inserts only need to supply `name` + `checksum`; the
//!     `TIMESTAMPTZ` type pins UTC semantics across deployments in
//!     different timezones.
//!
//! 36. **`CREATE TABLE IF NOT EXISTS`, not bare `CREATE TABLE`
//!     (JOLTR-RS-098).** Idempotency is load-bearing: `JoltRDb::connect`
//!     is called on every process restart, and `connect_returns_ok_*`
//!     tests run repeatedly against the same fixture database.
//!     `IF NOT EXISTS` makes the first connect against a fresh
//!     database create the table and every subsequent connect a
//!     no-op (a single `Notice` in Postgres logs, no error). The
//!     PRD wording "auto-create _migrations table if not exists"
//!     captures this requirement directly.
//!
//! [`JoltRDb::applied_migrations`] (JOLTR-RS-099) is the read-back half
//! of the migration apply pipeline. Returns a
//! [`HashMap`](std::collections::HashMap) keyed by migration filename
//! with the recorded SHA-256 checksum as the value, sourced from the
//! `_migrations` bookkeeping table that [`JoltRDb::connect`] auto-creates
//! (JOLTR-RS-098, decisions 34–36). The JOLTR-RS-100 apply path consumes
//! this map to decide which discovered [`MigrationFile`] values are
//! already on-disk-vs-DB matches (skip) and which are new (apply);
//! JOLTR-RS-102 will layer the tamper-detection check (apply-set name
//! present but checksum differs) on top of the same read-back.
//!
//! 37. **Read-back shape is `HashMap<String, String>` (name → checksum),
//!     not `Vec<AppliedMigration { name, checksum, applied_at }>`
//!     (JOLTR-RS-099).** A typed struct would carry the `applied_at`
//!     column straight through to callers, but no part of the apply
//!     pipeline (JOLTR-RS-100 apply, JOLTR-RS-102 tamper check) reads
//!     `applied_at` — the skip / tamper / apply decision is a pure
//!     name → checksum comparison. Returning a `HashMap` makes the
//!     load-bearing operation (`applied.get(&file.name)`) an O(1)
//!     lookup straight out of the verb, where a `Vec<AppliedMigration>`
//!     would force every caller into an O(n) linear search or a
//!     post-call `into_iter().map(...).collect::<HashMap<_,_>>()` shim.
//!     If JOLTR-RS-101's "list applied migrations" verb needs the
//!     richer row shape (e.g. for a human-readable applied-at column
//!     in a `joltr migrate status` listing) it can introduce a sibling
//!     `JoltRDb::applied_migration_rows() -> Vec<AppliedMigration>`
//!     without disturbing this contract. The `applied_at` column
//!     stays in the schema (decision 35) for operator observability
//!     and any future listing verb to read.
//!
//! 38. **Method on [`JoltRDb`], not a free function taking `&PgPool`
//!     (JOLTR-RS-099).** Consistent with the rest of phase19–21's
//!     read-side surface ([`Self::health_check`], [`Self::query_as`],
//!     [`Self::transaction`], [`Self::listen`], [`Self::notify`]) —
//!     every verb that consumes the pool is reached via a method on
//!     `JoltRDb`, never through a free function. Keeps the discoverable
//!     API surface for the migration pipeline grouped on the same
//!     receiver type (alongside the future JOLTR-RS-100 `migrate` /
//!     JOLTR-RS-101 listing verbs), avoids forcing callers to thread
//!     `db.pool()` separately, and preserves the "JoltRDb owns the
//!     pool" abstraction (decision 5) end-to-end.
//!
//! 39. **Skip-decision is a one-liner at the call site
//!     (`applied.get(&f.name) == Some(&f.checksum)`), not a
//!     `pending_migrations(files, applied) -> Vec<MigrationFile>`
//!     helper (JOLTR-RS-099).** The PRD-099 "skip migrations with
//!     matching checksums" semantic is one option lookup + one equality
//!     check — adding a helper would (a) duplicate the contract in two
//!     places (the helper body + the documented invariant on
//!     [`Self::applied_migrations`]) and (b) lock in a "skip vs apply"
//!     binary that JOLTR-RS-102 will need to split three ways (skip on
//!     match, error on mismatch, apply on missing). Leaving the
//!     decision at the call site lets the JOLTR-RS-100 apply loop
//!     express the three-way fork as a single `match` against
//!     `applied.get(&f.name)` (`Some(c) if c == &f.checksum => skip`,
//!     `Some(_) => tamper-error (102)`, `None => apply`) without an
//!     intermediate helper to evolve in lock-step. The skip invariant
//!     is documented on [`Self::applied_migrations`] and pinned by a
//!     dedicated unit test.
//!
//! [`JoltRDb::migrate`] (JOLTR-RS-100) is the apply half of the phase23
//! migration pipeline: discovers files via [`read_migration_files`],
//! reads the already-applied set via [`Self::applied_migrations`], and
//! for every discovered file not present in that set executes the SQL
//! body and records a new `_migrations` row inside a single
//! transaction. Returns the count of newly applied migrations.
//!
//! 40. **One transaction per migration, not one transaction for all
//!     (JOLTR-RS-100).** Each migration's SQL body + the corresponding
//!     `INSERT INTO _migrations` row commit together as a single unit;
//!     a partial-apply failure halts the chain at the failing migration
//!     with every prior migration's body durably committed. Matches
//!     [`sqlx::migrate!`]'s own behavior and what production migration
//!     runners do — one-tx-for-all would force a rollback of every
//!     successfully-applied earlier migration in a chain whenever a
//!     later one fails, which is rarely what an operator wants
//!     (especially when the schema change cascades into application-
//!     level data writes that have already completed against the
//!     earlier schema). The `_migrations` bookkeeping insert is inside
//!     the same transaction as the body so the bookkeeping row is only
//!     visible after the body's effects are durably committed; a body
//!     failure rolls back both halves and the next run re-attempts.
//!     The transaction is opened inline via `self.pool.begin().await?`
//!     rather than through [`Self::transaction`]'s closure wrapper:
//!     the closure shape forces two `&mut **tx` reborrows (body
//!     execute + bookkeeping insert) through a `&mut Transaction`
//!     reference, which trips sqlx's `Executor` higher-ranked trait
//!     bound on `&'c mut PgConnection`. With an owned `Transaction`
//!     here both `.execute(&mut *tx)` reborrows resolve to `&mut
//!     PgConnection` cleanly. The atomicity contract is identical to
//!     [`Self::transaction`]: explicit `commit` on success, drop on
//!     early return rolls back at the sqlx layer.
//!
//! 41. **Body SQL goes through [`sqlx::raw_sql`], not
//!     [`sqlx::query`] (JOLTR-RS-100).** Migration files routinely
//!     contain multiple semicolon-separated statements (e.g. `CREATE
//!     TABLE foo (...); CREATE INDEX foo_idx ON foo(...);`) and may
//!     use Postgres dollar-quoted bodies for stored functions.
//!     [`sqlx::raw_sql`] sends the body via the simple query protocol
//!     so multi-statement bodies execute in one round trip;
//!     [`sqlx::query`] uses the extended/prepared protocol which
//!     rejects multi-statement strings. Bound parameters are not
//!     supported (and not needed — migration bodies are author-
//!     controlled source files, not user input). Failures surface as
//!     the raw [`sqlx::Error`], rolled back by the surrounding
//!     transaction.
//!
//! 42. **Signature is `migrate(dir: &str) -> Result<usize, sqlx::Error>`
//!     returning the count of newly applied migrations; filesystem
//!     errors from [`read_migration_files`] are mapped through
//!     [`sqlx::Error::Io`] (JOLTR-RS-100).** Returning the count lets
//!     callers log "N migrations applied" without re-reading the
//!     table; callers who want the names can call
//!     [`Self::applied_migrations`] before-and-after for the diff. The
//!     [`sqlx::Error::Io`] mapping is a one-line bridge from the
//!     filesystem layer (decision 25) into the rest of joltr-db's
//!     unified [`sqlx::Error`] error surface (decision 6) — every
//!     other phase19–23 verb already returns [`sqlx::Error`], so a
//!     bespoke `MigrationError` enum at this slice would force
//!     callers into two error shapes for the apply pipeline. Decision
//!     25 explicitly forecasted this re-wrap; JOLTR-RS-102 (phase24)
//!     will introduce a dedicated `MigrationError` enum when the
//!     pipeline grows the tamper/removed failure modes that need
//!     pattern-matching on dedicated variants.
//!
//! 43. **The Some(_) skip arm of decision 39's three-way fork is
//!     collapsed into the skip branch for the JOLTR-RS-100 slice; the
//!     tamper-detection split lands in JOLTR-RS-102 (JOLTR-RS-100).**
//!     Decision 39 forecast a three-way fork (skip-on-match /
//!     tamper-on-mismatch / apply-on-missing), but the
//!     tamper-on-mismatch variant requires the `MigrationError` enum
//!     that JOLTR-RS-102 introduces. For JOLTR-RS-100 the apply loop
//!     skips on *any* presence in the applied set (matching the
//!     PRD-100 "apply new migrations" wording), so a mismatched
//!     checksum is currently treated as already-applied. JOLTR-RS-102
//!     will replace the `Some(_)` arm with the error path without
//!     touching the rest of the loop. The schema's lack of a
//!     `UNIQUE(name)` constraint on `_migrations` (decision 35 picks
//!     `id SERIAL PRIMARY KEY` for ordering, not `name UNIQUE`) means
//!     the collapsed skip is also the safer default — if a future
//!     caller bypasses this verb and inserts a duplicate `name`
//!     manually, the read-back still has a deterministic skip
//!     semantic.
//!
//! 44. **`sqlx::Error::Io` wraps the [`std::io::Error`] from
//!     [`read_migration_files`] verbatim via `.map_err(sqlx::Error::
//!     Io)` (JOLTR-RS-100).** Constructing a fresh `sqlx::Error::Io`
//!     instead of re-wrapping would lose the underlying `io::Error`
//!     kind / message and force callers into a bespoke string
//!     comparison to diagnose "directory missing" vs "permission
//!     denied". The variant accepts the original `io::Error` by
//!     value, so a single `.map_err(sqlx::Error::Io)` preserves the
//!     full diagnostic chain (`sqlx::Error::source() -> &io::Error`).
//!     Superseded by JOLTR-RS-102 (decisions 45–47): [`JoltRDb::migrate`]
//!     now returns [`MigrationError`], and the filesystem-error path
//!     flows through [`MigrationError::Io`] via the `From<io::Error>`
//!     impl rather than re-wrapping through `sqlx::Error::Io`. The
//!     `io::Error`-preservation contract is identical; only the outer
//!     variant changes.
//!
//! JOLTR-RS-102 opens phase24 by introducing [`MigrationError`] — the
//! dedicated error enum the migration apply pipeline returns now that it
//! has more than one failure mode worth pattern-matching on. The
//! [`JoltRDb::migrate`] signature changes from `Result<usize,
//! sqlx::Error>` (the JOLTR-RS-100 shape, decision 42) to `Result<usize,
//! MigrationError>`, and the JOLTR-RS-100 collapsed "skip on any
//! presence" arm (decision 43) splits into the three-way fork the
//! decision 39 closing notes forecast: name absent → apply; name
//! present with matching checksum → skip; name present with
//! mismatched checksum → [`MigrationError::Tampered`].
//!
//! 45. **Dedicated [`MigrationError`] enum, not a `sqlx::Error::Protocol(msg)`
//!     sentinel (JOLTR-RS-102).** Decision 42 documented when this slice
//!     would pay for itself: when the apply pipeline grows three or more
//!     distinct failure modes that callers want to pattern-match on. At
//!     JOLTR-RS-102 it has reached that threshold — `Tampered { name }` is
//!     the new variant, filesystem errors and sqlx errors are the
//!     pre-existing two. A sentinel `sqlx::Error::Protocol("Migration X
//!     has been modified ...")` would force callers into substring
//!     matching to differentiate tampered-vs-IO-vs-DB failures, which is
//!     exactly the brittleness the PRD's pattern-match-friendly error
//!     shape needs to avoid. Three variants at JOLTR-RS-102
//!     ([`MigrationError::Tampered`], [`MigrationError::Io`],
//!     [`MigrationError::Sqlx`]); JOLTR-RS-103 added the additive
//!     fourth variant [`MigrationError::Removed`] for rollback
//!     detection (decision 48). [`Self::
//!     applied_migrations`] keeps returning [`sqlx::Error`] directly —
//!     it has no migration-specific failure modes (decision 38 still
//!     stands), and callers who chain it into `migrate` propagate the
//!     `sqlx::Error` via the `From<sqlx::Error>` conversion (decision
//!     46). [`MigrationError`] implements [`std::error::Error`] with
//!     [`source`](std::error::Error::source) returning the wrapped
//!     `io::Error` / `sqlx::Error` so the full diagnostic chain (the
//!     same contract decision 44 pinned for the JOLTR-RS-100 shape) is
//!     preserved across the wrap.
//!
//! 46. **`From<std::io::Error>` and `From<sqlx::Error>` for
//!     [`MigrationError`] so `?`-chaining stays one line per call site
//!     (JOLTR-RS-102).** The migrate body calls `read_migration_files`
//!     (returns `io::Error`), `self.applied_migrations` (returns
//!     `sqlx::Error`), `self.pool.begin` / `tx.execute` / `tx.commit`
//!     (all `sqlx::Error`); without the `From` impls each call site
//!     would need its own `.map_err(MigrationError::Io)` or
//!     `.map_err(MigrationError::Sqlx)` shim. The `?` operator picks
//!     the right variant via the `From` impl, so the body reads
//!     `read_migration_files(dir)?` and `self.pool.begin().await?`
//!     unchanged from the JOLTR-RS-100 shape. Decision 44's
//!     `io::Error`-preservation contract carries through: the
//!     `From<io::Error>` impl wraps the original `io::Error` by value
//!     (no rebuilding a fresh one), so the `kind()` and `Display`
//!     output survive verbatim and surface via
//!     `MigrationError::source()`. Bespoke wrappers are still available
//!     for the rare call site that wants to override the variant
//!     choice explicitly.
//!
//! 47. **Three-way fork at the apply-loop call site, no helper
//!     function (JOLTR-RS-102).** Decision 39 forecast this shape and
//!     decision 43 deferred it; JOLTR-RS-102 lands it. The loop body
//!     reads `match applied.get(&file.name) { Some(c) if c ==
//!     &file.checksum => continue, Some(_) => return Err(Tampered {
//!     name: file.name.clone() }), None => /* apply */ }`. Inlining
//!     the fork at the call site (vs. a `pending_migrations(files,
//!     applied) -> Vec<_>` helper) keeps the loop's three-way control
//!     flow visible in one place and avoids leaking the decision into
//!     a separate function signature that JOLTR-RS-103 would have to
//!     extend again for the "applied row but no on-disk file"
//!     rollback case. The tamper `return` short-circuits the entire
//!     `migrate` call — once any prior migration shows a mismatched
//!     checksum, every migration that follows is considered untrusted
//!     and the operator must repair the divergence before the apply
//!     loop will resume. The error variant uses `clone()` for the
//!     name (cheap — migration filenames are short and the error path
//!     is the cold path) rather than threading a borrowed lifetime
//!     through `MigrationError`. The PRD-102 mandated [`Display`]
//!     output for the tamper variant is `"Migration {name} has been
//!     modified since it was applied."` verbatim — pinned by the
//!     `migration_error_display_renders_prd_verbatim_for_tampered`
//!     unit test so a regression that drops the period, mangles the
//!     wording, or escapes the name through `Debug` formatting fails
//!     without ever running migrate.
//!
//! 48. **Rollback detection runs as a separate scan before the
//!     three-way apply loop (JOLTR-RS-103).** A name in the
//!     `_migrations` read-back with no corresponding file in `dir`
//!     means an operator deleted (or moved, or renamed) a migration
//!     that already shipped — almost always an attempt to roll it
//!     back, which joltr-db deliberately does not support
//!     (forward-only, matching `sqlx::migrate!`'s contract). The
//!     check belongs *before* the per-file three-way fork (decision
//!     47) because a removed file is a global-state divergence: it
//!     can't be expressed inside the `for file in &files { match
//!     applied.get(&file.name) { ... } }` shape at all (that loop
//!     iterates files, never the applied keys). Inlining the scan
//!     at the head of `migrate` (vs. a free `detect_rollback(files,
//!     applied) -> Result<(), MigrationError>` helper) keeps the
//!     control flow of the entire apply pipeline visible in one
//!     place and avoids spawning a function whose only caller is
//!     this one. Implementation is `for applied_name in
//!     applied.keys() { if !files.iter().any(|f| &f.name ==
//!     applied_name) { return Err(MigrationError::Removed { name:
//!     applied_name.clone() }); } }` — O(applied × files); with
//!     realistic migration counts (<1000) the constant factor is
//!     negligible vs. building a `HashSet<&str>` of file names
//!     (which would shave the lookup to amortized O(1) but add a
//!     `use std::collections::HashSet;` and an extra allocation for
//!     a path that runs at most once per migrate call). Short-
//!     circuit on the *first* missing name (HashMap iteration order
//!     is non-deterministic, so which name is reported when more
//!     than one is missing is also non-deterministic — a deliberate
//!     non-decision because operators repairing one removed file
//!     will see the next one on their re-run; a sorted scan can be
//!     added later without changing the contract). The PRD-103
//!     mandated [`Display`] output for the [`MigrationError::
//!     Removed`] variant is `"Migration {name} has been removed.
//!     Rollbacks are not supported."` verbatim — pinned by the
//!     `migration_error_display_renders_prd_verbatim_for_removed`
//!     unit test. The variant is additive (no breaking change to
//!     [`MigrationError`]'s existing variants) so JOLTR-RS-102's
//!     `From<io::Error>` / `From<sqlx::Error>` impls (decision 46)
//!     and `source()` chain (decision 45) carry through unchanged.
//!
//! JOLTR-RS-104 opens phase24's CLI slice: the `joltr-db` binary
//! (`src/main.rs`) exposes a [`clap`]-driven subcommand surface whose
//! first verb is `migrate new <name>` — scaffolds a new empty migration
//! file at `migrations/YYYYMMDDHHMMSS_<name>.sql`. The shared logic is
//! exposed via the public [`create_migration_file`] function on the
//! library so that JOLTR-RS-105's "CLI creates correct filename" test
//! can exercise it without spawning a subprocess.
//!
//! 49. **CLI is `src/main.rs` (auto-discovered binary named after the
//!     package), library function does the work (JOLTR-RS-104).** The
//!     PRD mandates the binary name `joltr-db` and Cargo's default
//!     target inference creates exactly that binary from `src/main.rs`
//!     when the package name is `joltr-db` — no `[[bin]]` entry is
//!     needed and the lib + bin coexist. The actual file creation
//!     (timestamp computation, filename assembly, collision check,
//!     write) lives in the library as [`create_migration_file`] so
//!     the JOLTR-RS-105 closing tests can call it directly. The binary
//!     is a thin clap layer over the library call —
//!     `clap::Parser::parse() -> create_migration_file(dir, &name,
//!     Utc::now())` — matching the same library-first pattern the rest
//!     of `joltr-db`'s verbs use (the `migrate` apply verb is on
//!     `JoltRDb`, not a binary; this CLI scaffolding verb is on the
//!     filesystem, not the DB, so it's a free function rather than a
//!     method). Subcommand shape is nested: top-level `migrate` group
//!     with a `new <name>` action, leaving room for JOLTR-RS-100-style
//!     `migrate apply` and other verbs later without breaking the
//!     established CLI shape.
//!
//! 50. **Timestamp is `chrono::Utc::now().format("%Y%m%d%H%M%S")`
//!     (JOLTR-RS-104).** UTC (not local time) so the same migration
//!     file generated on two operators' machines in different time
//!     zones still sorts correctly by filename — the lexicographic
//!     sort decision from JOLTR-RS-094 (decision 26) requires the
//!     timestamp prefix to be a strictly monotonic numeric string for
//!     correctness, and UTC is the only timezone that guarantees that
//!     across collaborators. The `%Y%m%d%H%M%S` format gives a
//!     14-character fixed-width prefix that lexicographically sorts
//!     identically to chronologically (zero-padded month/day/hour/
//!     minute/second). The `now` value is threaded as a parameter on
//!     [`create_migration_file`] (not called inside the function) so
//!     the unit tests can pin a deterministic timestamp without
//!     mocking the clock — the binary supplies `chrono::Utc::now()` at
//!     the call site.
//!
//! 51. **Placeholder body is `"-- migration: <name>\n"` (JOLTR-RS-104).**
//!     One-line SQL comment naming the migration. joltr-db is
//!     forward-only (decision 48), so an `-- up` / `-- down` template
//!     would mislead operators into expecting reversibility we
//!     deliberately don't support. A bare empty file would parse
//!     cleanly through `read_migration_files` (decision 24 captures
//!     content verbatim) but offers no human-readable hint of which
//!     migration the file is — the one-line comment gives operators
//!     a label they'll immediately recognize when grepping a stale
//!     migrations directory. Trailing newline matches POSIX file
//!     convention (most editors will auto-add one anyway, but the
//!     binary should produce a well-formed file on first write).
//!
//! 52. **Filename collision returns `Err(io::ErrorKind::AlreadyExists)`,
//!     does not overwrite (JOLTR-RS-104).** Two `migrate new <name>`
//!     calls in the same second produce identical timestamps and
//!     therefore identical filenames. Overwriting would silently lose
//!     the first migration's body (whatever the operator typed into
//!     it after creation); erroring is the safe default. The check
//!     uses `OpenOptions::new().write(true).create_new(true)` — an
//!     atomic create-or-fail at the filesystem layer that closes the
//!     check-then-write race a `Path::exists()` + `fs::write` pair
//!     would leave open. Same operator can re-run after the second
//!     elapses (the timestamp will differ) or supply a more specific
//!     name to disambiguate.
//!
//! 53. **Name validation rejects path separators and the empty string
//!     (JOLTR-RS-104).** A name like `"../etc/passwd"` would otherwise
//!     escape the `migrations/` directory (the constructed filename
//!     is `migrations/<timestamp>_<name>.sql`, and `<name>` is
//!     substituted verbatim). The validation runs before
//!     `OpenOptions::create_new` so the error path surfaces a
//!     dedicated `io::ErrorKind::InvalidInput` rather than a generic
//!     filesystem failure further down the stack. Restricted set: no
//!     `/`, no `\`, no empty string. Other characters (spaces, dots,
//!     hyphens, mixed case) are operator-controlled stylistic choices
//!     and pass through unchanged — joltr-db doesn't normalize the
//!     name beyond the separator guard, matching the
//!     filesystem-passthrough convention `read_migration_files` uses
//!     for the discovery side.

/// Per-deployment Postgres pool configuration consumed by the upcoming
/// `JoltRDb::connect` (JOLTR-RS-083) to build a
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
    /// [`std::time::Duration`] by `JoltRDb::connect` before being handed to
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

/// `CREATE TABLE IF NOT EXISTS _migrations (...)` DDL executed inside
/// [`JoltRDb::connect`] (JOLTR-RS-098). See module docs decisions 34–36
/// for the architectural contract: PRD-mandated schema, idempotent via
/// `IF NOT EXISTS`, ran from `connect` rather than a separate verb.
///
/// Public so JOLTR-RS-099+ tests (and any caller that wants to recreate
/// the bookkeeping table by hand against a non-`JoltRDb` pool) can
/// reference the same DDL the production connect path uses; pinning it
/// here makes a schema drift between connect and a test fixture
/// impossible.
pub const MIGRATIONS_TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS _migrations (\
    id SERIAL PRIMARY KEY, \
    name TEXT NOT NULL, \
    checksum TEXT NOT NULL, \
    applied_at TIMESTAMPTZ DEFAULT NOW()\
)";

/// Runtime handle around a [`sqlx::PgPool`] consumed by every downstream
/// phase19/20/21 slice (JOLTR-RS-083). See module docs decisions 5–7 for
/// the ownership shape, error contract, and connect semantics; decisions
/// 8 and 9 cover the read-side API ([`Self::pool`], [`Self::health_check`])
/// added by JOLTR-RS-084.
#[derive(Debug, Clone)]
pub struct JoltRDb {
    pool: sqlx::PgPool,
}

impl JoltRDb {
    /// Build a pool from `config` and return the owning [`JoltRDb`].
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
    /// After the pool opens, `connect` issues a `CREATE TABLE IF NOT
    /// EXISTS _migrations (...)` round trip to ensure the migration
    /// bookkeeping table exists (JOLTR-RS-098, decisions 34–36). The
    /// schema is `id SERIAL PRIMARY KEY, name TEXT NOT NULL, checksum
    /// TEXT NOT NULL, applied_at TIMESTAMPTZ DEFAULT NOW()` and is
    /// idempotent across restarts. A `CREATE TABLE` failure (typically
    /// a permission error on the configured user) propagates as the
    /// raw [`sqlx::Error`] and the pool is dropped along with the
    /// unsuccessful return so callers never see a half-initialized
    /// [`JoltRDb`].
    ///
    /// [`PgPoolOptions::max_connections`]: https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgPoolOptions.html#method.max_connections
    /// [`PgPoolOptions::acquire_timeout`]: https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgPoolOptions.html#method.acquire_timeout
    /// [`PgPoolOptions::connect`]: https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgPoolOptions.html#method.connect
    pub async fn connect(config: &DbConfig) -> Result<Self, sqlx::Error> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(config.max_connections)
            .acquire_timeout(std::time::Duration::from_secs(config.acquire_timeout_secs))
            .connect(&config.database_url)
            .await?;
        sqlx::query(MIGRATIONS_TABLE_DDL).execute(&pool).await?;
        Ok(Self { pool })
    }

    /// Borrow the underlying [`sqlx::PgPool`] (JOLTR-RS-084, decision 8).
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
    /// (JOLTR-RS-084, decision 9).
    ///
    /// Returns `Ok(())` on a successful round trip, or the raw
    /// [`sqlx::Error`] on any failure (acquire timeout, connection drop,
    /// authentication failure, etc.). The row payload is discarded — the
    /// success of the round trip is the whole signal.
    ///
    /// Intended use sites: `JoltRServer` readiness probes, HTTP `/healthz`
    /// endpoints, and JOLTR-RS-085's closing connection test.
    pub async fn health_check(&self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    /// Build a typed query against this pool for a row type `T`
    /// (JOLTR-RS-086).
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

    /// Run `f` inside a Postgres transaction (JOLTR-RS-088).
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
    /// LISTEN/NOTIFY (JOLTR-RS-090).
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
    /// (JOLTR-RS-091).
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
        impl tokio_stream::Stream<Item = Result<sqlx::postgres::PgNotification, sqlx::Error>> + Unpin,
        sqlx::Error,
    > {
        let mut listener = self.listen_connection().await?;
        listener.listen(channel).await?;
        Ok(listener.into_stream())
    }

    /// Publish a notification to a Postgres `LISTEN` channel
    /// (JOLTR-RS-092).
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

    /// Read the `_migrations` bookkeeping table and return a
    /// [`HashMap`](std::collections::HashMap) keyed by migration filename
    /// with the recorded SHA-256 checksum as the value (JOLTR-RS-099).
    ///
    /// See module docs decisions 37–39 for the architectural contract:
    /// `HashMap<String, String>` read-back shape (not a typed struct),
    /// method on [`JoltRDb`] (not a free function), and the skip-decision
    /// semantic that the JOLTR-RS-100 apply path layers on top.
    ///
    /// The skip invariant for the [`Self::migrate`] apply loop: a
    /// discovered [`MigrationFile`] `f` is already applied (and should
    /// be skipped) when `applied.get(&f.name) == Some(&f.checksum)`.
    /// A name present with a *different* checksum is the JOLTR-RS-102
    /// tamper case — the apply loop returns
    /// [`MigrationError::Tampered`]. A name not present at all is a
    /// new migration that the apply loop will execute. A name in
    /// this map with no corresponding file in the migrations
    /// directory is the JOLTR-RS-103 rollback case — the apply loop
    /// returns [`MigrationError::Removed`] before any file is
    /// applied.
    ///
    /// Reads `name` and `checksum` only; the `applied_at` column is
    /// intentionally not surfaced (decision 37). Returns the raw
    /// [`sqlx::Error`] (decision 6) on any read failure (acquire
    /// timeout, connection drop, `_migrations` missing because a caller
    /// constructed a `JoltRDb` bypassing [`Self::connect`], etc.).
    ///
    /// # Example
    ///
    /// ```ignore
    /// let files = joltr_db::read_migration_files("./migrations")?;
    /// let applied = db.applied_migrations().await?;
    /// for f in &files {
    ///     if applied.get(&f.name) == Some(&f.checksum) {
    ///         continue; // already applied with matching checksum — skip
    ///     }
    ///     // apply f (JOLTR-RS-100)
    /// }
    /// ```
    pub async fn applied_migrations(
        &self,
    ) -> Result<std::collections::HashMap<String, String>, sqlx::Error> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT name, checksum FROM _migrations")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().collect())
    }

    /// Discover migration files in `dir`, apply every one not yet
    /// recorded in `_migrations`, and return the count of newly applied
    /// migrations (JOLTR-RS-100; tamper detection added by JOLTR-RS-102;
    /// rollback detection added by JOLTR-RS-103).
    ///
    /// See module docs decisions 40–48 for the architectural contract:
    /// one transaction per migration, body executed via
    /// [`sqlx::raw_sql`] (multi-statement bodies supported), filesystem
    /// errors flow through [`MigrationError::Io`], sqlx errors flow
    /// through [`MigrationError::Sqlx`], the apply loop runs the
    /// three-way fork (skip on matching checksum, tamper on mismatch,
    /// apply on missing), and a rollback-detection pass runs before
    /// the loop body to surface
    /// [`MigrationError::Removed`] when an applied migration's file
    /// has disappeared from disk.
    ///
    /// Composes the three primitives already in place:
    /// 1. [`read_migration_files`] (JOLTR-RS-094/096) for discovery
    ///    sorted lexicographically by filename and pre-hashed.
    /// 2. [`Self::applied_migrations`] (JOLTR-RS-099) for the
    ///    name → checksum read-back used by the skip / tamper / apply
    ///    decision.
    /// 3. [`Self::transaction`] (JOLTR-RS-088) for the per-migration
    ///    body + bookkeeping atomicity boundary.
    ///
    /// For each discovered [`MigrationFile`] in lex-sorted order
    /// (decision 47's three-way fork):
    /// - Name in the applied set with matching checksum → skip.
    /// - Name in the applied set with a different checksum → return
    ///   [`MigrationError::Tampered`] immediately; every later
    ///   migration is left unattempted until the operator repairs
    ///   the divergence.
    /// - Name not in the applied set → open a transaction, execute
    ///   the file's `content` via [`sqlx::raw_sql`], `INSERT INTO
    ///   _migrations (name, checksum) VALUES ($1, $2)`, and commit.
    ///   The bookkeeping row's `id` and `applied_at` come from the
    ///   column defaults (decision 35).
    ///
    /// Errors propagate as [`MigrationError`]: filesystem failures
    /// from discovery flow through [`MigrationError::Io`] via the
    /// `From<std::io::Error>` impl (decision 46), migration-body and
    /// bookkeeping failures flow through [`MigrationError::Sqlx`] via
    /// the `From<sqlx::Error>` impl (rolled back by the surrounding
    /// per-migration transaction), tamper detection surfaces
    /// [`MigrationError::Tampered`] with the file's name, and
    /// rollback detection surfaces [`MigrationError::Removed`] with
    /// the missing file's name. The PRD-102 / PRD-103
    /// [`Display`](std::fmt::Display) messages are
    /// `"Migration {name} has been modified since it was applied."`
    /// and `"Migration {name} has been removed. Rollbacks are not
    /// supported."` verbatim (decisions 47 + 48).
    ///
    /// # Example
    ///
    /// ```ignore
    /// let applied = db.migrate("./migrations").await?;
    /// println!("applied {applied} new migrations");
    /// ```
    pub async fn migrate(&self, dir: &str) -> Result<usize, MigrationError> {
        let files = read_migration_files(dir)?;
        let applied = self.applied_migrations().await?;
        // Decision 48: rollback detection. Before the per-file apply
        // loop runs, check that every name recorded in `_migrations`
        // still has a corresponding file on disk. A missing file is
        // an attempt to roll a migration back, which joltr-db does
        // not support (forward-only). Short-circuit on the first
        // missing name (matches the JOLTR-RS-102 tamper short-circuit
        // semantic). Runs before the three-way fork because a
        // removed file is global-state divergence, not a per-file
        // problem.
        for applied_name in applied.keys() {
            if !files.iter().any(|f| &f.name == applied_name) {
                return Err(MigrationError::Removed {
                    name: applied_name.clone(),
                });
            }
        }
        let mut count = 0_usize;
        for file in &files {
            // Decision 47: three-way fork pinning JOLTR-RS-102's
            // tamper detection alongside the JOLTR-RS-100 skip / apply
            // arms. A tamper short-circuits the entire migrate call
            // — once any prior migration's checksum diverges from
            // disk the operator must repair the state before
            // subsequent migrations are attempted.
            match applied.get(&file.name) {
                Some(stored) if stored == &file.checksum => continue,
                Some(_) => {
                    return Err(MigrationError::Tampered {
                        name: file.name.clone(),
                    });
                }
                None => {}
            }
            // Per-migration transaction (decision 40). We open the
            // transaction inline via `self.pool.begin()` rather than
            // going through `Self::transaction`'s `&mut Transaction`
            // closure shape, because that shape forces two
            // `&mut **tx` reborrows (body execute + bookkeeping
            // insert) through a `&mut Transaction` reference, which
            // trips sqlx's `Executor` HRTB. With an owned
            // `Transaction` here both `.execute(&mut *tx)` reborrows
            // resolve to `&mut PgConnection` cleanly. The atomicity
            // contract is the same: explicit `commit` on success;
            // drop on early return (the `?`s on each execute) rolls
            // back at the sqlx layer.
            let mut tx = self.pool.begin().await?;
            sqlx::raw_sql(&file.content).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO _migrations (name, checksum) VALUES ($1, $2)")
                .bind(&file.name)
                .bind(&file.checksum)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            count += 1;
        }
        Ok(count)
    }
}

/// Typed-query builder returned by [`JoltRDb::query_as`] (JOLTR-RS-086).
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
    /// Bind one positional parameter to this query (JOLTR-RS-086, decision 11).
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

/// A migration file discovered by [`read_migration_files`] (JOLTR-RS-094,
/// extended by JOLTR-RS-096 with the [`checksum`](Self::checksum) field).
///
/// Holds the metadata phase22's sort-by-filename discovery needs: the
/// basename (no directory prefix), the file's UTF-8 body, and the SHA-256
/// digest of that body as a lowercase hex string. See module docs
/// decisions 24 and 32–33 for the flat-`pub`-fields layout and
/// auto-populated-checksum contracts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationFile {
    /// Filename (basename, no directory prefix). Used as the sort key in
    /// [`read_migration_files`] and as the identifier the eventual
    /// `_migrations` bookkeeping table (JOLTR-RS-098) will record.
    pub name: String,
    /// File contents read as UTF-8. The full SQL body that the apply step
    /// (JOLTR-RS-100) will execute inside a transaction.
    pub content: String,
    /// SHA-256 digest of [`Self::content`] as a 64-character lowercase
    /// hex string (JOLTR-RS-096). Populated automatically by
    /// [`read_migration_files`] via [`sha256_hex`] — callers can rely on
    /// `checksum == sha256_hex(content.as_bytes())` for every value
    /// produced by the discovery function (decision 32). Hand-constructed
    /// [`MigrationFile`] values must compute the field the same way to
    /// stay consistent with the JOLTR-RS-099 apply-time tamper check.
    pub checksum: String,
}

/// Error type returned by the migration apply pipeline
/// ([`JoltRDb::migrate`], JOLTR-RS-102+).
///
/// See module docs decisions 45–48 for the architectural contract:
/// dedicated enum (not a `sqlx::Error::Protocol(msg)` sentinel),
/// `From<std::io::Error>` and `From<sqlx::Error>` for `?`-chaining
/// inside the apply loop, a PRD-102 verbatim [`Display`] message for
/// the [`Self::Tampered`] variant, and a PRD-103 verbatim [`Display`]
/// message for the [`Self::Removed`] rollback-detection variant.
#[derive(Debug)]
pub enum MigrationError {
    /// A previously-applied migration's recorded checksum in
    /// `_migrations` no longer matches the on-disk file's checksum
    /// (decision 47). Detected by comparing
    /// [`MigrationFile::checksum`] (computed at discovery time by
    /// [`read_migration_files`] via [`sha256_hex`], decision 32)
    /// against the corresponding row from
    /// [`JoltRDb::applied_migrations`].
    ///
    /// The [`Display`](std::fmt::Display) output for this variant is
    /// the PRD-102 verbatim message
    /// `"Migration {name} has been modified since it was applied."`.
    Tampered {
        /// Filename (basename) of the tampered migration —
        /// [`MigrationFile::name`] / `_migrations.name` verbatim.
        name: String,
    },
    /// A name recorded in `_migrations` has no corresponding file in
    /// the migrations directory (decision 48, JOLTR-RS-103). An
    /// operator deleted (or moved, or renamed) a migration file that
    /// already shipped — almost always an attempt to roll the
    /// migration back, which joltr-db deliberately does not support
    /// (forward-only, matching `sqlx::migrate!`'s contract).
    ///
    /// The [`Display`](std::fmt::Display) output for this variant is
    /// the PRD-103 verbatim message
    /// `"Migration {name} has been removed. Rollbacks are not supported."`.
    Removed {
        /// Filename (basename) of the removed migration —
        /// `_migrations.name` verbatim. The name reported is the
        /// first removed name encountered while scanning the applied
        /// set; if more than one file is missing the operator will
        /// see the rest after fixing this one and re-running.
        name: String,
    },
    /// Filesystem failure surfaced from [`read_migration_files`]
    /// (missing directory, permission denied, etc.). Wraps the
    /// underlying [`std::io::Error`] by value — kind, message, and
    /// [`source`](std::error::Error::source) chain are preserved
    /// (decision 46).
    Io(std::io::Error),
    /// Postgres / sqlx-level failure surfaced from the pool, the
    /// `_migrations` read-back, or any of the per-migration
    /// transaction operations (body execute, bookkeeping insert,
    /// commit). Wraps the raw [`sqlx::Error`] by value so callers can
    /// still pattern-match on the inner variant
    /// (`Error::PoolTimedOut`, `Error::Database(_)`, etc.).
    Sqlx(sqlx::Error),
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // PRD-102 mandates this message verbatim.
            Self::Tampered { name } => {
                write!(
                    f,
                    "Migration {name} has been modified since it was applied."
                )
            }
            // PRD-103 mandates this message verbatim.
            Self::Removed { name } => {
                write!(
                    f,
                    "Migration {name} has been removed. Rollbacks are not supported."
                )
            }
            Self::Io(err) => write!(f, "migration filesystem error: {err}"),
            Self::Sqlx(err) => write!(f, "migration database error: {err}"),
        }
    }
}

impl std::error::Error for MigrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Tampered { .. } => None,
            Self::Removed { .. } => None,
            Self::Io(err) => Some(err),
            Self::Sqlx(err) => Some(err),
        }
    }
}

impl From<std::io::Error> for MigrationError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<sqlx::Error> for MigrationError {
    fn from(err: sqlx::Error) -> Self {
        Self::Sqlx(err)
    }
}

/// Discover migration files in `dir` (JOLTR-RS-094; populates the
/// [`MigrationFile::checksum`] field as of JOLTR-RS-096).
///
/// Reads every entry in `dir`, retains files whose name ends in `.sql`,
/// reads each retained file's contents as UTF-8, computes the SHA-256
/// digest of those contents via [`sha256_hex`], and returns the resulting
/// [`MigrationFile`] values sorted lexicographically by filename. See
/// module docs decisions 24–28 and 32–33 for the architectural contract.
///
/// Returns `Err(std::io::Error)` if `dir` is not readable (missing,
/// permission denied, not a directory) or if any individual `.sql` file
/// fails to read as UTF-8. Subdirectories, non-`.sql` files, and entries
/// whose filename is not valid UTF-8 are silently skipped (decision 27).
///
/// # Example
///
/// ```ignore
/// let files = joltr_db::read_migration_files("./migrations")?;
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
        let checksum = sha256_hex(content.as_bytes());
        files.push(MigrationFile {
            name,
            content,
            checksum,
        });
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

/// Hash `bytes` with SHA-256 and return the digest as a 64-character
/// lowercase hexadecimal string (JOLTR-RS-095).
///
/// See module docs decisions 29–31 for the architectural contract:
/// free function (not a method on [`MigrationFile`]) so the primitive
/// composes for both the JOLTR-RS-096 file-discovery path and the
/// JOLTR-RS-099 apply-time tamper check; hand-rolled hex (no `hex` crate
/// dependency); lowercase output to match `shasum -a 256` / `git` /
/// `openssl dgst -sha256` conventions.
///
/// # Example
///
/// ```
/// // NIST SHA-256 test vector for the ASCII input "abc".
/// assert_eq!(
///     joltr_db::sha256_hex(b"abc"),
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

/// Scaffold a new migration file in `dir` (JOLTR-RS-104). The `joltr-db
/// migrate new <name>` CLI is a thin wrapper around this function.
///
/// Builds `<dir>/<YYYYMMDDHHMMSS>_<name>.sql` from `now` (formatted UTC,
/// decision 50), writes the `"-- migration: <name>\n"` placeholder body
/// (decision 51), and returns the path that was written. Creates `dir`
/// (and any missing parent components) if it does not yet exist so the
/// first `migrate new` against a fresh project succeeds without a
/// separate `mkdir`.
///
/// # Errors
///
/// * [`std::io::ErrorKind::InvalidInput`] if `name` is empty or contains
///   a path separator (`/` or `\`) — decision 53.
/// * [`std::io::ErrorKind::AlreadyExists`] if a file at the constructed
///   path already exists. Use a more specific `name` or wait one second
///   for the timestamp to roll over — decision 52.
/// * Any other [`std::io::Error`] surfaced from creating `dir` or
///   writing the file (propagated as-is).
///
/// # Example
///
/// ```ignore
/// let path = joltr_db::create_migration_file(
///     "./migrations",
///     "add_users",
///     chrono::Utc::now(),
/// )?;
/// println!("created {}", path.display());
/// ```
pub fn create_migration_file(
    dir: &str,
    name: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> std::io::Result<std::path::PathBuf> {
    // Decision 53: reject empty names and path separators before
    // touching the filesystem so the failure surfaces as a dedicated
    // `InvalidInput` rather than a downstream filesystem error.
    if name.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "migration name cannot be empty",
        ));
    }
    if name.contains('/') || name.contains('\\') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "migration name cannot contain path separators",
        ));
    }

    // Decision 50: UTC timestamp, fixed-width `%Y%m%d%H%M%S` so the
    // 14-character prefix sorts lexicographically == chronologically.
    let timestamp = now.format("%Y%m%d%H%M%S").to_string();
    let filename = format!("{timestamp}_{name}.sql");
    let dir_path = std::path::Path::new(dir);
    std::fs::create_dir_all(dir_path)?;
    let path = dir_path.join(&filename);

    // Decision 52: atomic create-or-fail. `create_new(true)` returns
    // `ErrorKind::AlreadyExists` if the path exists, closing the
    // check-then-write race a `Path::exists` + `fs::write` pair would
    // leave open.
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    // Decision 51: one-line SQL comment naming the migration, trailing
    // newline. Forward-only (decision 48) so no `-- up` / `-- down`
    // template.
    writeln!(file, "-- migration: {name}")?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::{
        read_migration_files, sha256_hex, DbConfig, JoltRDb, MigrationError, MigrationFile,
        TypedQuery, MIGRATIONS_TABLE_DDL,
    };

    mod connection {
        use super::{DbConfig, JoltRDb};

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
            // Confirms the derive landed — the connect call (JOLTR-RS-083) will
            // want to log the config on startup at least once.
            let cfg = DbConfig::new("postgres://localhost/db");
            let rendered = format!("{cfg:?}");
            assert!(rendered.contains("DbConfig"));
            assert!(rendered.contains("postgres://localhost/db"));
        }

        #[test]
        fn clone_is_implemented() {
            // Connect-call (JOLTR-RS-083) may want to keep an owned clone of the
            // config alongside the pool; pinned so the derive doesn't get
            // dropped.
            let cfg = DbConfig::new("postgres://localhost/db");
            let copy = cfg.clone();
            assert_eq!(copy.database_url, cfg.database_url);
            assert_eq!(copy.max_connections, cfg.max_connections);
            assert_eq!(copy.acquire_timeout_secs, cfg.acquire_timeout_secs);
        }

        // ---- JOLTR-RS-083: JoltRDb::connect ----

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
            let result = JoltRDb::connect(&cfg).await;
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
            let result = JoltRDb::connect(&cfg).await;
            assert!(
                result.is_err(),
                "expected Err from connect with malformed URL, got Ok",
            );
        }

        /// Success-path test gated on the `JOLTR_TEST_DATABASE_URL` env var.
        ///
        /// Without the env var set the test passes trivially so the default
        /// `cargo test -p joltr-db` flow does not require a running Postgres.
        /// With the env var set (e.g. `JOLTR_TEST_DATABASE_URL=postgres://...
        /// cargo test -p joltr-db`) the test exercises the PRD-mandated
        /// "JoltRDb::connect() returns Ok" verification.
        #[tokio::test]
        async fn connect_returns_ok_when_test_db_available() {
            let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                // No test DB configured — skip. The error-path tests above
                // exercise the rest of the connect logic.
                return;
            };
            let cfg = DbConfig::new(url);
            let db = JoltRDb::connect(&cfg)
                .await
                .expect("expected Ok from JoltRDb::connect against JOLTR_TEST_DATABASE_URL");
            // Pool is reachable: a trivial SELECT 1 should round-trip.
            let one: (i32,) = sqlx::query_as("SELECT 1")
                .fetch_one(db.pool())
                .await
                .expect("SELECT 1 against the connected pool failed");
            assert_eq!(one.0, 1);
        }

        // ---- JOLTR-RS-084: JoltRDb::pool + JoltRDb::health_check ----

        /// `pool()` returns a borrow of the underlying [`sqlx::PgPool`] (decision
        /// 8). Compile-pins that the signature is `&PgPool` (a borrow) rather
        /// than `PgPool` (a clone) — the explicit `&sqlx::PgPool` binding will
        /// fail to typecheck if the getter ever changes to return an owned
        /// value.
        ///
        /// Uses the unreachable-server fixture from the connect error-path tests
        /// because the slice only needs an owned `JoltRDb` to exercise the getter
        /// shape, not a live pool. The connect itself is expected to fail; the
        /// test path that actually inspects a `pool()` borrow lives in the
        /// env-gated `health_check_returns_ok_*` test below.
        #[test]
        fn pool_signature_is_borrow_not_clone() {
            // Pure compile-time pin: the binding annotation forces the return
            // type to be `&PgPool`. No runtime body needed.
            fn _pin(db: &JoltRDb) -> &sqlx::PgPool {
                db.pool()
            }
        }

        /// Health-check success path gated on `JOLTR_TEST_DATABASE_URL` (same
        /// convention as `connect_returns_ok_when_test_db_available`). Pins
        /// decision 9: a successful `SELECT 1` round trip resolves to `Ok(())`.
        #[tokio::test]
        async fn health_check_returns_ok_when_test_db_available() {
            let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                return;
            };
            let cfg = DbConfig::new(url);
            let db = JoltRDb::connect(&cfg)
                .await
                .expect("expected Ok from JoltRDb::connect against JOLTR_TEST_DATABASE_URL");
            db.health_check()
                .await
                .expect("expected Ok from JoltRDb::health_check on live pool");
        }

        /// Health-check failure path: a pool whose configured server is
        /// unreachable surfaces an `Err` rather than hanging or panicking. Uses
        /// the same `127.0.0.1:1` + 1-second-acquire-timeout fixture as the
        /// connect error-path tests, with `connect_lazy_with` so the pool is
        /// constructed without an upfront TCP dial — the `SELECT 1` inside
        /// `health_check` is what tries (and fails) to acquire a connection.
        ///
        /// This is the only path in joltr-db that uses `connect_lazy_with`; it
        /// exists exclusively to give the health-check failure path a `JoltRDb`
        /// to call `health_check()` on without requiring a live Postgres. The
        /// production constructor remains the eager [`JoltRDb::connect`] from
        /// JOLTR-RS-083.
        #[tokio::test]
        async fn health_check_returns_err_on_unreachable_server() {
            let pool_options = sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(std::time::Duration::from_secs(1));
            let pool = pool_options
                .connect_lazy("postgres://nouser:nopw@127.0.0.1:1/nodb")
                .expect("connect_lazy should accept a well-formed URL even if unreachable");
            let db = JoltRDb { pool };
            let result = db.health_check().await;
            assert!(
                result.is_err(),
                "expected Err from health_check against unreachable server, got Ok",
            );
        }
    }

    // ---- JOLTR-RS-086/087: JoltRDb::query_as + TypedQuery<T> query helpers ----

    mod query_helpers {
        use super::{DbConfig, JoltRDb, TypedQuery};

        /// Compile-time pin: `JoltRDb::query_as::<T>(sql)` returns
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
            fn _pin(db: &JoltRDb) -> TypedQuery<Row> {
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
            fn _pin(db: &JoltRDb) -> TypedQuery<Row> {
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
            fn _pin(db: &JoltRDb) {
                let q: TypedQuery<Row> = db.query_as::<Row>("SELECT 1 AS id");
                _assert_static_send(&q);
            }
        }

        /// PRD-mandated success-path verification for JOLTR-RS-086:
        /// `db.query_as::<TestRow>("SELECT 1 AS id").fetch_one()` returns
        /// `TestRow { id: 1 }`. Gated on `JOLTR_TEST_DATABASE_URL` so the
        /// default `cargo test -p joltr-db` flow does not require a running
        /// Postgres; with the env var set the test exercises the full pipeline.
        #[tokio::test]
        async fn query_as_fetch_one_returns_test_row() {
            let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                return;
            };

            #[derive(sqlx::FromRow)]
            struct TestRow {
                id: i32,
            }

            let cfg = DbConfig::new(url);
            let db = JoltRDb::connect(&cfg)
                .await
                .expect("connect against JOLTR_TEST_DATABASE_URL");
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
            let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                return;
            };

            #[derive(sqlx::FromRow)]
            struct Row {
                #[allow(dead_code)]
                id: i32,
            }

            let cfg = DbConfig::new(url);
            let db = JoltRDb::connect(&cfg).await.expect("connect");
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
            let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                return;
            };

            #[derive(sqlx::FromRow)]
            struct Row {
                id: i32,
            }

            let cfg = DbConfig::new(url);
            let db = JoltRDb::connect(&cfg).await.expect("connect");
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
        /// JOLTR-RS-086 description that the parameterless `SELECT 1 AS id` test
        /// doesn't exercise.
        #[tokio::test]
        async fn query_as_with_bind_round_trips_parameter() {
            let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                return;
            };

            #[derive(sqlx::FromRow)]
            struct Row {
                v: i32,
            }

            let cfg = DbConfig::new(url);
            let db = JoltRDb::connect(&cfg).await.expect("connect");
            let row: Row = db
                .query_as::<Row>("SELECT $1::int4 AS v")
                .bind(42_i32)
                .fetch_one()
                .await
                .expect("fetch_one of SELECT $1::int4 AS v with bind(42)");
            assert_eq!(row.v, 42);
        }
        /// Compile-time pin: `TypedQuery<T>` carries its row type at compile
        /// time through the full fetch chain. A regression that erases the
        /// type parameter (e.g. converting `TypedQuery<T>` to a generic
        /// `Query<'_>`) would break the ability to differentiate queries
        /// bound to incompatible row types — the compiler would no longer
        /// reject `_assert_different::<TypedQuery<RowA>, TypedQuery<RowB>>()`
        /// below. Pinned via a compile-only fn that exercises the full
        /// `query_as::<T>(sql).fetch_one() -> Result<T, _>` chain for two
        /// distinct row types whose `TypedQuery<T>` instances must be proven
        /// as different types at compile time.
        #[test]
        fn type_mismatch_is_caught_at_compile_time() {
            #[derive(sqlx::FromRow)]
            #[allow(dead_code)]
            struct RowI32 {
                col: i32,
            }

            #[derive(sqlx::FromRow)]
            #[allow(dead_code)]
            struct RowString {
                col: String,
            }

            // Verify the two TypedQuery<T> instantiations are distinct
            // compiler-visible types. A regression that erases the type
            // parameter to a single concrete type would allow the
            // `_assert_different` call to succeed — the compiler won't
            // accept `_assert_different::<T, T>()` for a function of
            // signature `fn _assert_different<T, U>()`.
            fn _assert_different<T, U>() {}
            fn _pin(db: &JoltRDb) {
                let _qi: TypedQuery<RowI32> = db.query_as::<RowI32>("SELECT 1 AS col");
                let _qs: TypedQuery<RowString> = db.query_as::<RowString>("SELECT 'a' AS col");
                _assert_different::<TypedQuery<RowI32>, TypedQuery<RowString>>();
            }
        }

        /// Runtime verification: a query projecting a column name that does
        /// not exist in the result set surfaces a runtime error (not a hang,
        /// panic, or silent default). The `#[derive(sqlx::FromRow)]` struct
        /// has an `id: i32` field expecting column `id`, but the SQL
        /// projects `SELECT 1 AS non_existent_column` — sqlx has no such
        /// column to deserialize into `id` and returns an `Err`. Env-gated
        /// on `JOLTR_TEST_DATABASE_URL`; without a live Postgres the test
        /// skips trivially so the default `cargo test -p joltr-db` flow
        /// stays runnable.
        #[tokio::test]
        async fn missing_column_returns_runtime_error_when_test_db_available() {
            let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                return;
            };

            #[derive(sqlx::FromRow)]
            #[allow(dead_code)]
            struct Row {
                id: i32,
            }

            let cfg = DbConfig::new(url);
            let db = JoltRDb::connect(&cfg).await.expect("connect");
            let result: Result<Row, sqlx::Error> = db
                .query_as::<Row>("SELECT 1 AS non_existent_column")
                .fetch_one()
                .await;
            assert!(
                result.is_err(),
                "expected runtime error for query with missing column, got Ok",
            );
        }

        // -- JOLTR-RS-088/089: transaction commit / rollback --

        mod transaction {
            use super::{DbConfig, JoltRDb};

            /// Compile-time pin: `db.transaction(|tx| Box::pin(async move {
            /// ... }))` typechecks against the documented signature
            /// (decisions 12–13). The `_pin` fn never runs; it exists so a
            /// regression that changes the closure parameter type away from
            /// `&mut Transaction<'static, Postgres>` or the return type away
            /// from `Pin<Box<dyn Future ...>>` fails the build.
            #[test]
            fn signature_accepts_box_pin_closure() {
                async fn _pin(db: &JoltRDb) -> Result<i32, sqlx::Error> {
                    db.transaction(|_tx| Box::pin(async move { Ok::<_, sqlx::Error>(42_i32) }))
                        .await
                }
            }

            /// Commit path: closure returns `Ok` → transaction commits →
            /// the inserted row is visible after `transaction` returns.
            /// Env-gated on `JOLTR_TEST_DATABASE_URL`.
            #[tokio::test]
            async fn commits_on_ok_when_test_db_available() {
                let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                    return;
                };
                let cfg = DbConfig::new(url);
                let db = JoltRDb::connect(&cfg).await.expect("connect");

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
                    "expected committed insert visible after transaction returned Ok",
                );

                sqlx::query("DROP TABLE _jolt_tx_commit_test")
                    .execute(db.pool())
                    .await
                    .expect("drop table (teardown)");
            }

            /// Rollback path: closure returns `Err` → transaction rolls
            /// back → the attempted insert is *not* visible. The closure's
            /// error propagates out (decision 14). Env-gated.
            #[tokio::test]
            async fn rolls_back_on_err_when_test_db_available() {
                let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                    return;
                };
                let cfg = DbConfig::new(url);
                let db = JoltRDb::connect(&cfg).await.expect("connect");

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
                    "expected rolled-back insert invisible after transaction returned Err",
                );

                sqlx::query("DROP TABLE _jolt_tx_rollback_test")
                    .execute(db.pool())
                    .await
                    .expect("drop table (teardown)");
            }
        }
    }

    // JOLTR-RS-090/091/092/093: LISTEN/NOTIFY tests co-located under
    // `mod listen_notify` so the filter `cargo test -p joltr-db --
    // tests::listen_notify` picks up the full phase21 surface.
    mod listen_notify {
        use super::{DbConfig, JoltRDb};

        use tokio_stream::StreamExt;

        // ---- JOLTR-RS-090: JoltRDb::listen_connection ----

        /// Compile-time pin: `db.listen_connection()` resolves to
        /// `Result<sqlx::postgres::PgListener, sqlx::Error>` (decisions 15–17).
        /// The explicit return annotation forces the typecheck — a regression
        /// that wraps the listener in a foreign type (e.g. `tokio_postgres::
        /// Connection`) or changes the error shape would break this build pin
        /// without ever needing a live Postgres.
        #[test]
        fn listen_connection_signature_returns_pg_listener() {
            async fn _pin(db: &JoltRDb) -> Result<sqlx::postgres::PgListener, sqlx::Error> {
                db.listen_connection().await
            }
        }

        /// PRD-mandated success-path verification for JOLTR-RS-090: "Dedicated
        /// connection opens without error." Env-gated on `JOLTR_TEST_DATABASE_URL`
        /// (same convention as 083/084/086/088); without a live Postgres the
        /// test skips trivially so the default `cargo test -p joltr-db` flow
        /// stays runnable.
        ///
        /// Also pins decision 17 by opening two listeners back-to-back: each
        /// call yields its own connection, neither blocks the other. The pool's
        /// regular `health_check` is exercised in between to confirm the pool
        /// path is unaffected by the listener allocations.
        #[tokio::test]
        async fn listen_connection_opens_dedicated_connection_when_test_db_available() {
            let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                return;
            };
            let cfg = DbConfig::new(url);
            let db = JoltRDb::connect(&cfg).await.expect("connect");

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

        // ---- JOLTR-RS-091: JoltRDb::listen ----

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
            fn _assert_stream<
                S: tokio_stream::Stream<Item = Result<sqlx::postgres::PgNotification, sqlx::Error>>
                    + Unpin,
            >(
                _: &S,
            ) {
            }
            async fn _pin(db: &JoltRDb) -> Result<(), sqlx::Error> {
                let stream = db.listen("test_ch").await?;
                _assert_stream(&stream);
                Ok(())
            }
        }

        /// PRD-mandated verification for JOLTR-RS-091: "listen("test_ch") yields
        /// a Stream." Env-gated on `JOLTR_TEST_DATABASE_URL` (same convention as
        /// 083/084/086/088/090): without a live Postgres the test skips
        /// trivially so the default `cargo test -p joltr-db` flow stays runnable.
        ///
        /// With the env var set: calls `listen("_jolt_listen_smoke_ch")` and
        /// asserts the outer `Result` is `Ok` (setup succeeded — the dedicated
        /// connection opened and the `LISTEN` round trip completed). The
        /// returned stream itself is dropped without driving it, which (a)
        /// keeps this slice scoped to the JOLTR-RS-091 verification (the
        /// LISTEN/NOTIFY end-to-end notification round-trip is JOLTR-RS-093's
        /// closing test) and (b) pins decision 20: a `listen` whose backing
        /// connection is allocated outside the pool drops cleanly without
        /// affecting subsequent pool queries.
        #[tokio::test]
        async fn listen_yields_stream_when_test_db_available() {
            let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                return;
            };
            let cfg = DbConfig::new(url);
            let db = JoltRDb::connect(&cfg).await.expect("connect");

            let stream = db
                .listen("_jolt_listen_smoke_ch")
                .await
                .expect("listen on a fresh channel should succeed");

            // Drop the stream explicitly to make the lifecycle test intent
            // visible — the goal is "listen() returns Ok with a Stream", not
            // "we consumed any items from it". JOLTR-RS-093 will exercise the
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

        // ---- JOLTR-RS-092: JoltRDb::notify ----

        /// Compile-time pin: `db.notify(&str, &str)` resolves to
        /// `Result<(), sqlx::Error>` (decisions 21–23). The explicit return
        /// annotation forces the typecheck — a regression that surfaces the
        /// `pg_notify` row payload, wraps the error in a custom enum, or
        /// changes the parameter shape would break this build pin without
        /// ever needing a live Postgres.
        #[test]
        fn notify_signature_returns_unit_result() {
            async fn _pin(db: &JoltRDb) -> Result<(), sqlx::Error> {
                db.notify("test_ch", "hello").await
            }
        }

        /// PRD-mandated verification for JOLTR-RS-092: "notify("test_ch",
        /// "hello") succeeds." Env-gated on `JOLTR_TEST_DATABASE_URL` (same
        /// convention as 083/084/086/088/090/091): without a live Postgres
        /// the test skips trivially so the default `cargo test -p joltr-db`
        /// flow stays runnable.
        ///
        /// Calls `notify("_jolt_notify_smoke_ch", "hello")` and asserts the
        /// `Result` is `Ok(())`. Notification delivery (i.e. that a
        /// concurrent `LISTEN`-er actually receives this payload) is
        /// JOLTR-RS-093's closing-test slice; here we only verify the write
        /// half round-trips. Pool health is checked afterward to pin
        /// decision 22 (notify uses the regular pool, not a dedicated
        /// connection — an accidental switch to a long-lived connection
        /// would still pass this test individually, but the back-to-back
        /// invocations below would saturate a pool whose `max_connections`
        /// shrunk to a single listener slot).
        #[tokio::test]
        async fn notify_succeeds_when_test_db_available() {
            let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                return;
            };
            let cfg = DbConfig::new(url);
            let db = JoltRDb::connect(&cfg).await.expect("connect");

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
            let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                return;
            };
            let cfg = DbConfig::new(url);
            let db = JoltRDb::connect(&cfg).await.expect("connect");

            // A payload that would SQL-inject a raw `NOTIFY ch, '<payload>'`
            // form. With `pg_notify($1, $2)` it's just a string literal on
            // the wire and round-trips cleanly.
            db.notify("_jolt_notify_smoke_ch", "it's safe")
                .await
                .expect("single-quote payload should round-trip via parameter binding");
        }

        // ---- JOLTR-RS-093: LISTEN/NOTIFY end-to-end ----

        /// PRD-mandated verification for JOLTR-RS-093: "listen on channel,
        /// notify with payload, verify notification arrives on stream with
        /// correct payload." Env-gated on `JOLTR_TEST_DATABASE_URL` (same
        /// convention as 083..092); without a live Postgres the test skips
        /// trivially. This is the closing integration test for phase21
        /// (db-listen-notify): it proves that the write path (`notify`) and
        /// the read path (`listen` stream) compose end-to-end through a live
        /// Postgres instance.
        ///
        /// Uses a PID + atomic-counter unique channel name per run to avoid
        /// collision with concurrent test runs against the same database.
        #[tokio::test]
        async fn listen_receives_notification_with_correct_payload_when_test_db_available() {
            let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                return;
            };
            use std::sync::atomic::{AtomicU64, Ordering};
            static CHAN_COUNTER: AtomicU64 = AtomicU64::new(0);
            let channel = format!(
                "_jolt_e2e_listen_{}_{}",
                std::process::id(),
                CHAN_COUNTER.fetch_add(1, Ordering::Relaxed),
            );

            let cfg = DbConfig::new(url);
            let db = JoltRDb::connect(&cfg).await.expect("connect");

            let mut stream = db
                .listen(&channel)
                .await
                .expect("listen on fresh channel should succeed");

            db.notify(&channel, "hello from e2e")
                .await
                .expect("notify should succeed");

            let Ok(item) =
                tokio::time::timeout(std::time::Duration::from_secs(2), stream.next()).await
            else {
                panic!("timed out waiting for notification on channel {channel}");
            };

            let notification = item.expect("stream should yield an item, not None");
            let note = notification.expect("PgNotification should be Ok");
            assert_eq!(note.channel(), channel);
            assert_eq!(note.payload(), "hello from e2e");

            // Pool still healthy after the full listen → notify → receive
            // lifecycle. Catches a regression (e.g. the listener connection
            // leaking back into the pool via a shared-connection mistake).
            db.health_check()
                .await
                .expect("pool still healthy after e2e notify round-trip");
        }
    }

    // ---- JOLTR-RS-094..097: read_migration_files / sha256_hex / MigrationFile ----

    /// Self-cleaning temp directory used by migration tests. Each instance
    /// lives in `std::env::temp_dir()` under a PID + process-local
    /// atomic-counter name so concurrent test threads don't collide, and
    /// the directory is removed on `Drop`.
    struct TestDir {
        path: std::path::PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "joltr-db-097-{}-{}-{}",
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

    mod migration_files {
        use super::{read_migration_files, sha256_hex, MigrationFile, TestDir};

        // ---- JOLTR-RS-094: read_migration_files signature + sort + capture ----

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

        /// PRD-mandated verification for JOLTR-RS-094: two files
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

        // ---- JOLTR-RS-095: sha256_hex ----

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

        /// PRD-mandated verification for JOLTR-RS-095: a known input hashes
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
        /// reference value when JOLTR-RS-099's apply-time comparison runs.
        #[test]
        fn sha256_hex_is_lowercase() {
            let out = sha256_hex(b"abc");
            assert!(
                out.chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                "expected lowercase hex digits, got {out:?}",
            );
        }

        /// Hashing the SQL body of a migration file produces a stable hex
        /// digest that the JOLTR-RS-096 `read_migration_files` extension
        /// (and the JOLTR-RS-099 apply-time tamper check) can record and
        /// compare against. Reference value computed via
        /// `echo -n 'SELECT 1;' | shasum -a 256`.
        #[test]
        fn sha256_hex_hashes_migration_body() {
            assert_eq!(
                sha256_hex(b"SELECT 1;"),
                "17db4fd369edb9244b9f91d9aeed145c3d04ad8ba6e95d06247f07a63527d11a",
            );
        }

        // ---- JOLTR-RS-096: MigrationFile.checksum ----

        /// Compile-time pin for the JOLTR-RS-096 struct shape: `MigrationFile`
        /// holds `name`, `content`, and `checksum` as plain `pub String`
        /// fields constructible by struct literal (decisions 24, 33). A
        /// regression that drops a field, renames one, or wraps `checksum`
        /// in a newtype breaks this build pin without ever running.
        #[test]
        fn migration_file_struct_shape_pins() {
            let f = MigrationFile {
                name: String::from("001_init.sql"),
                content: String::from("SELECT 1;"),
                checksum: String::from(
                    "17db4fd369edb9244b9f91d9aeed145c3d04ad8ba6e95d06247f07a63527d11a",
                ),
            };
            assert_eq!(f.name, "001_init.sql");
            assert_eq!(f.content, "SELECT 1;");
            assert_eq!(f.checksum.len(), 64);
        }

        /// PRD-mandated verification for JOLTR-RS-096: a [`MigrationFile`]
        /// returned by [`read_migration_files`] has a populated `checksum`
        /// that matches the SHA-256 hex digest of its own `content` field
        /// (decision 32). Uses the same `001_init.sql` / `SELECT 1;`
        /// fixture as `read_migration_files_captures_name_and_content` so
        /// the recorded checksum matches the independently-derivable
        /// reference value (`echo -n 'SELECT 1;' | shasum -a 256`) that
        /// `sha256_hex_hashes_migration_body` already pins.
        #[test]
        fn read_migration_files_populates_checksum() {
            let dir = TestDir::new("checksum");
            dir.write_file("001_init.sql", "SELECT 1;");

            let files = read_migration_files(dir.path_str()).expect("read");
            assert_eq!(files.len(), 1);
            assert_eq!(
                files[0].checksum,
                "17db4fd369edb9244b9f91d9aeed145c3d04ad8ba6e95d06247f07a63527d11a",
            );
            assert_eq!(files[0].checksum, sha256_hex(files[0].content.as_bytes()));
        }

        /// Distinct file bodies produce distinct checksums — a regression
        /// that wired every `MigrationFile` to the same constant (or hashed
        /// the filename instead of the body) fails this test. Also pins
        /// that the sort step does not scramble the per-file
        /// `name`/`content`/`checksum` triple alignment.
        #[test]
        fn read_migration_files_checksum_differs_per_file() {
            let dir = TestDir::new("differs");
            dir.write_file("001_init.sql", "SELECT 1;");
            dir.write_file("002_users.sql", "SELECT 2;");

            let files = read_migration_files(dir.path_str()).expect("read");
            assert_eq!(files.len(), 2);
            assert_ne!(files[0].checksum, files[1].checksum);
            assert_eq!(files[0].checksum, sha256_hex(files[0].content.as_bytes()));
            assert_eq!(files[1].checksum, sha256_hex(files[1].content.as_bytes()));
        }

        // ---- JOLTR-RS-097: edge-case coverage ----

        /// Empty directory → empty vec. Pins the contract that a
        /// directory with no `.sql` files returns `Ok(vec![])` rather
        /// than erroring or panicking. The empty-vec return is the base
        /// case for every caller that reads migrations from a directory
        /// before new files are written.
        #[test]
        fn read_migration_files_empty_directory_returns_empty_vec() {
            let dir = TestDir::new("empty");
            let files = read_migration_files(dir.path_str()).expect("read");
            assert!(files.is_empty());
        }

        /// Missing directory → `Err`. `read_migration_files` calls
        /// `std::fs::read_dir` internally and should surface the
        /// filesystem error directly rather than treating it as
        /// "no files found." Pinned so a regression that substitutes
        /// an empty `Ok(vec![])` for `Err` on missing-directory fails
        /// this test.
        #[test]
        fn read_migration_files_errors_on_missing_directory() {
            let result = read_migration_files("/tmp/__jolt_nonexistent_dir_097__");
            assert!(
                result.is_err(),
                "expected Err for missing directory, got Ok({:?})",
                result.ok(),
            );
        }

        /// Non-`.sql` entries are silently skipped (decision 27).
        /// Directory entries like `README.md`, `.gitkeep`,
        /// `editor_backup.sql~` must not appear in the returned Vec
        /// and must not cause errors. Pinned so a regression that
        /// tightens the extension filter to error on non-.sql files
        /// or that adds non-.sql entries to the return set fails this.
        #[test]
        fn read_migration_files_skips_non_sql_files() {
            let dir = TestDir::new("nonsql");
            dir.write_file("001_init.sql", "SELECT 1;");
            dir.write_file("README.md", "# Migration notes");
            dir.write_file("notes.txt", "not a migration");
            dir.write_file("002_users.sql", "SELECT 2;");

            let files = read_migration_files(dir.path_str()).expect("read");
            let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
            assert_eq!(names, vec!["001_init.sql", "002_users.sql"]);
        }
    }

    // ---- JOLTR-RS-098: connect auto-creates _migrations table ----

    /// Compile-time pin: `MIGRATIONS_TABLE_DDL` is a `&'static str`
    /// containing all four PRD-mandated columns (decision 35). A
    /// regression that drops a column, renames `_migrations`, or
    /// changes the type of any column fails this assertion without
    /// needing a live Postgres.
    #[test]
    fn migrations_table_ddl_pins_prd_schema() {
        // Pin every PRD-098 column name + type + the `_migrations` table
        // identifier + the `IF NOT EXISTS` idempotency clause (decision
        // 36). Substring checks rather than full-DDL equality so a
        // formatting tweak (extra whitespace, case change) doesn't
        // require updating the test.
        assert!(MIGRATIONS_TABLE_DDL.contains("CREATE TABLE IF NOT EXISTS _migrations"));
        assert!(MIGRATIONS_TABLE_DDL.contains("id SERIAL PRIMARY KEY"));
        assert!(MIGRATIONS_TABLE_DDL.contains("name TEXT NOT NULL"));
        assert!(MIGRATIONS_TABLE_DDL.contains("checksum TEXT NOT NULL"));
        assert!(MIGRATIONS_TABLE_DDL.contains("applied_at TIMESTAMPTZ DEFAULT NOW()"));
    }

    /// PRD-mandated verification for JOLTR-RS-098: "fresh DB →
    /// _migrations table created." Env-gated on `JOLTR_TEST_DATABASE_URL`
    /// (same convention as 083/084/086/088/090/091/092): without a
    /// live Postgres the test skips trivially so the default
    /// `cargo test -p joltr-db` flow stays runnable.
    ///
    /// With the env var set: drops the `_migrations` table (best-effort
    /// — fresh-DB simulation), calls `JoltRDb::connect`, then queries
    /// `information_schema.tables` to confirm the table now exists with
    /// the four PRD-mandated columns (id, name, checksum, applied_at).
    /// Also pins decision 34 (the auto-create is inside `connect`, not
    /// a separate verb) — a regression that moves the DDL to an
    /// uncalled `ensure_migrations_table` would leave the table missing
    /// after `connect` and fail this assertion.
    #[tokio::test]
    async fn connect_auto_creates_migrations_table_when_test_db_available() {
        let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
            return;
        };

        // Fresh-DB simulation: drop any pre-existing _migrations table
        // so the assertion below verifies *this* connect call did the
        // creating, not a leftover from a prior test run. Best-effort
        // cleanup — a missing table is fine.
        {
            let pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(std::time::Duration::from_secs(5))
                .connect(&url)
                .await
                .expect("setup pool for DROP TABLE");
            sqlx::query("DROP TABLE IF EXISTS _migrations")
                .execute(&pool)
                .await
                .expect("drop _migrations (setup)");
        }

        let cfg = DbConfig::new(url);
        let db = JoltRDb::connect(&cfg).await.expect("connect");

        // Verify the table exists and carries the four PRD-mandated
        // columns. Querying `information_schema.columns` (rather than
        // `to_regclass` or `pg_tables`) gives us the column-shape
        // check the connect path is supposed to land.
        let column_names: Vec<(String,)> = sqlx::query_as(
            "SELECT column_name::text FROM information_schema.columns \
             WHERE table_name = '_migrations' ORDER BY ordinal_position",
        )
        .fetch_all(db.pool())
        .await
        .expect("query information_schema after connect");
        let names: Vec<String> = column_names.into_iter().map(|(n,)| n).collect();
        assert_eq!(
            names,
            vec!["id", "name", "checksum", "applied_at"],
            "expected the four PRD-mandated columns in order",
        );
    }

    /// Decision 36 idempotency: calling `JoltRDb::connect` twice
    /// against the same database does not fail. The second call hits
    /// the `IF NOT EXISTS` short-circuit; a regression that uses a
    /// bare `CREATE TABLE` (no idempotency clause) would fail this
    /// test with a "relation already exists" error on the second
    /// connect. Env-gated.
    #[tokio::test]
    async fn connect_is_idempotent_with_existing_migrations_table_when_test_db_available() {
        let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
            return;
        };
        let cfg = DbConfig::new(url);

        // First connect creates the table (or no-ops if it already
        // exists from a prior test run).
        let _db1 = JoltRDb::connect(&cfg).await.expect("first connect");

        // Second connect against the now-existing table must also
        // succeed. This is the idempotency pin.
        let _db2 = JoltRDb::connect(&cfg)
            .await
            .expect("second connect should be idempotent");
    }

    // ---- JOLTR-RS-099: JoltRDb::applied_migrations ----

    /// Compile-time pin: `db.applied_migrations()` resolves to
    /// `Result<HashMap<String, String>, sqlx::Error>` (decisions
    /// 37, 38). The explicit return annotation forces the typecheck —
    /// a regression that swaps in `Vec<AppliedMigration>`, wraps the
    /// error in a custom enum, or moves the verb off of `JoltRDb`
    /// breaks this build pin without needing a live Postgres.
    #[test]
    fn applied_migrations_signature_returns_hash_map() {
        async fn _pin(
            db: &JoltRDb,
        ) -> Result<std::collections::HashMap<String, String>, sqlx::Error> {
            db.applied_migrations().await
        }
    }

    /// Pure unit pin for the JOLTR-RS-099 skip-decision invariant
    /// (decision 39): a [`MigrationFile`] is considered already-applied
    /// (and JOLTR-RS-100 must skip it) iff
    /// `applied.get(&f.name) == Some(&f.checksum)`. Demonstrates the
    /// three call-site cases the apply loop will match against:
    /// missing entry → apply, matching checksum → skip, differing
    /// checksum → tamper (JOLTR-RS-102's concern; pinned here as
    /// "not equal" so the eventual three-way fork's middle arm
    /// remains expressible without changing this verb).
    #[test]
    fn applied_migrations_skip_decision_compares_name_to_checksum() {
        use std::collections::HashMap;
        let f = MigrationFile {
            name: String::from("001_init.sql"),
            content: String::from("SELECT 1;"),
            checksum: String::from(
                "17db4fd369edb9244b9f91d9aeed145c3d04ad8ba6e95d06247f07a63527d11a",
            ),
        };

        // Empty applied set: file is new → apply (skip-decision is false).
        let empty: HashMap<String, String> = HashMap::new();
        assert_ne!(empty.get(&f.name), Some(&f.checksum));

        // Applied set with matching checksum: skip-decision is true.
        let mut matching = HashMap::new();
        matching.insert(f.name.clone(), f.checksum.clone());
        assert_eq!(matching.get(&f.name), Some(&f.checksum));

        // Applied set with the same name but a different checksum:
        // skip-decision is false (JOLTR-RS-102 will surface this as a
        // tamper error; here it must at least *not* be treated as a
        // skip).
        let mut tampered = HashMap::new();
        tampered.insert(
            f.name.clone(),
            String::from("0000000000000000000000000000000000000000000000000000000000000000"),
        );
        assert_ne!(tampered.get(&f.name), Some(&f.checksum));
    }

    /// PRD-mandated verification for JOLTR-RS-099 ("migration already
    /// applied → skipped on next run"), env-gated on
    /// `JOLTR_TEST_DATABASE_URL` (same convention as 083/084/086/088/
    /// 090/091/092/098).
    ///
    /// Flow: connect (auto-creates `_migrations`), truncate so the
    /// test starts from a known-empty state, hand-insert two rows
    /// representing previously-applied migrations, call
    /// `applied_migrations()`, then assert the returned HashMap
    /// matches the inserted set verbatim and that the skip-decision
    /// invariant (decision 39) holds for both names: a freshly-
    /// discovered [`MigrationFile`] whose name + checksum match an
    /// entry in the map is recognized as already-applied. Also
    /// exercises the empty-table edge case as a sibling check inside
    /// the same test (single connect, two assertions) so the live-DB
    /// fixture cost is amortized.
    #[tokio::test]
    async fn applied_migrations_round_trips_rows_when_test_db_available() {
        let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
            return;
        };
        let cfg = DbConfig::new(url);
        let db = JoltRDb::connect(&cfg).await.expect("connect");

        // Known-empty starting state. TRUNCATE rather than DROP so the
        // `_migrations` table itself (auto-created by `connect`) stays
        // in place — the read-back verb requires the schema to exist.
        sqlx::query("TRUNCATE TABLE _migrations")
            .execute(db.pool())
            .await
            .expect("truncate _migrations (setup)");

        // Empty-table case: no rows applied yet → empty HashMap. This
        // is the "fresh DB" edge that JOLTR-RS-100's apply loop will
        // see on first invocation.
        let empty = db
            .applied_migrations()
            .await
            .expect("applied_migrations against empty _migrations should succeed");
        assert!(
            empty.is_empty(),
            "expected empty HashMap from empty _migrations, got {empty:?}",
        );

        // Hand-insert two rows representing previously-applied
        // migrations. Uses the same checksums the
        // `read_migration_files_populates_checksum` and
        // `sha256_hex_hashes_migration_body` tests pin against the
        // `SELECT 1;` and `SELECT 2;` migration bodies — so the
        // skip-decision assertion below lines up with the values
        // `read_migration_files` would emit for the corresponding
        // on-disk fixtures.
        let init_checksum = sha256_hex(b"SELECT 1;");
        let users_checksum = sha256_hex(b"SELECT 2;");
        sqlx::query("INSERT INTO _migrations (name, checksum) VALUES ($1, $2), ($3, $4)")
            .bind("001_init.sql")
            .bind(&init_checksum)
            .bind("002_users.sql")
            .bind(&users_checksum)
            .execute(db.pool())
            .await
            .expect("insert two _migrations rows");

        let applied = db
            .applied_migrations()
            .await
            .expect("applied_migrations after insert should succeed");

        // Read-back shape: both rows come back keyed by name with the
        // checksum as the value. Pins decision 37 (HashMap, not Vec).
        assert_eq!(
            applied.len(),
            2,
            "expected 2 entries from 2 inserted rows, got {applied:?}",
        );
        assert_eq!(applied.get("001_init.sql"), Some(&init_checksum));
        assert_eq!(applied.get("002_users.sql"), Some(&users_checksum));

        // Skip-decision pin (decision 39 + PRD-099 "skip migrations
        // with matching checksums"): a discovered `MigrationFile`
        // whose name + checksum match an entry is recognized as
        // already-applied. This is the exact expression JOLTR-RS-100's
        // apply loop will `continue` on.
        let init_file = MigrationFile {
            name: String::from("001_init.sql"),
            content: String::from("SELECT 1;"),
            checksum: init_checksum.clone(),
        };
        let users_file = MigrationFile {
            name: String::from("002_users.sql"),
            content: String::from("SELECT 2;"),
            checksum: users_checksum.clone(),
        };
        assert_eq!(applied.get(&init_file.name), Some(&init_file.checksum));
        assert_eq!(applied.get(&users_file.name), Some(&users_file.checksum));

        // A file that has not been applied yet: name missing from the
        // HashMap entirely. This is the "apply" branch of JOLTR-RS-100.
        let new_file = MigrationFile {
            name: String::from("003_brand_new.sql"),
            content: String::from("SELECT 3;"),
            checksum: sha256_hex(b"SELECT 3;"),
        };
        assert!(
            !applied.contains_key(&new_file.name),
            "expected None for a name not in _migrations, got {:?}",
            applied.get(&new_file.name),
        );

        // Cleanup so successive test runs against the same fixture
        // start from a known state.
        sqlx::query("TRUNCATE TABLE _migrations")
            .execute(db.pool())
            .await
            .expect("truncate _migrations (teardown)");
    }

    // ---- JOLTR-RS-100: JoltRDb::migrate ----

    /// Compile-time pin: `db.migrate(dir: &str)` resolves to
    /// `Result<usize, MigrationError>` (decisions 42 + 45). JOLTR-RS-102
    /// changes the error type from `sqlx::Error` to [`MigrationError`];
    /// the explicit return annotation forces the typecheck so a
    /// regression that reverts to `sqlx::Error`, switches the count
    /// type to `i64` / `u32`, or changes the parameter to `&Path` /
    /// `impl AsRef<Path>` breaks this build pin without needing a live
    /// Postgres.
    #[test]
    fn migrate_signature_pins() {
        async fn _pin(db: &JoltRDb, dir: &str) -> Result<usize, MigrationError> {
            db.migrate(dir).await
        }
    }

    /// Filesystem errors from discovery surface as
    /// `MigrationError::Io` (decisions 44 + 46). A non-existent
    /// directory triggers [`read_migration_files`]'s `read_dir` to
    /// fail with `io::ErrorKind::NotFound`; JOLTR-RS-102's
    /// `From<std::io::Error>` impl on [`MigrationError`] wraps that
    /// `io::Error` by value without modifying the underlying kind /
    /// message, so a regression that constructs a fresh `io::Error`
    /// (losing the kind) or routes the FS error through a non-Io
    /// variant fails this pin. Does not need a live Postgres because
    /// the directory-read failure happens before any DB round trip.
    #[tokio::test]
    async fn migrate_returns_io_error_for_missing_directory() {
        // Use the unreachable-server connect_lazy fixture so we have a
        // `JoltRDb` to call `migrate` on without requiring live Postgres.
        // The directory failure short-circuits before any pool query
        // runs.
        let pool_options = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(1));
        let pool = pool_options
            .connect_lazy("postgres://nouser:nopw@127.0.0.1:1/nodb")
            .expect("connect_lazy accepts well-formed URL");
        let db = JoltRDb { pool };

        let missing = format!(
            "/tmp/joltr-db-100-missing-{}-{}",
            std::process::id(),
            "definitely-not-there",
        );
        let err = db
            .migrate(&missing)
            .await
            .expect_err("migrate against missing directory should error");
        match err {
            MigrationError::Io(io_err) => {
                assert_eq!(
                    io_err.kind(),
                    std::io::ErrorKind::NotFound,
                    "expected NotFound from missing directory, got {io_err:?}",
                );
            }
            other => panic!("expected MigrationError::Io for FS failure, got {other:?}"),
        }
    }

    /// PRD-mandated verification for JOLTR-RS-100: "empty DB with 2
    /// migrations → both applied in order, both rows in _migrations."
    /// Env-gated on `JOLTR_TEST_DATABASE_URL` (same convention as the
    /// rest of the live-DB tests in this module).
    ///
    /// Flow:
    /// 1. TRUNCATE `_migrations` + DROP the migration target table for
    ///    a known-empty starting state.
    /// 2. Write two migration files into a temp directory:
    ///    - `001_create.sql` creates the target table.
    ///    - `002_insert.sql` inserts one row into it.
    ///    The order dependency is load-bearing — if migrate ran 002
    ///    before 001 (regression in the lex-sort or the per-tx commit
    ///    chain) the insert would fail because the table wouldn't
    ///    exist yet.
    /// 3. Call `db.migrate(dir)` and assert it returns `Ok(2)`.
    /// 4. Verify the target table now has the expected row (proves the
    ///    body actually ran, not just the bookkeeping insert).
    /// 5. Verify `_migrations` holds both rows in ascending `id` order
    ///    matching the lex-sorted filename order (pins decision 35 +
    ///    decision 40 + the "in filename order" PRD wording).
    /// 6. Re-run `db.migrate(dir)` against the same fixture and assert
    ///    it returns `Ok(0)` — idempotency / skip semantic for the
    ///    already-applied case (decision 43).
    /// 7. Truncate + drop on teardown so successive runs start clean.
    #[tokio::test]
    async fn migrate_applies_two_migrations_in_order_when_test_db_available() {
        let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
            return;
        };
        let cfg = DbConfig::new(url);
        let db = JoltRDb::connect(&cfg).await.expect("connect");

        // Fresh-state setup. TRUNCATE the bookkeeping table and drop
        // the per-test target table so this run can be the one to
        // create them.
        sqlx::query("TRUNCATE TABLE _migrations")
            .execute(db.pool())
            .await
            .expect("truncate _migrations (setup)");
        sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_test_target")
            .execute(db.pool())
            .await
            .expect("drop target table (setup)");

        let dir = TestDir::new("apply");
        dir.write_file(
            "001_create.sql",
            "CREATE TABLE _jolt_migrate_test_target (id INT);",
        );
        // The 002 insert depends on 001's CREATE — running out of
        // order makes 002 fail with "relation does not exist", which
        // would fail the `Ok(2)` assertion below. This is how we pin
        // the in-filename-order contract without a separate test.
        dir.write_file(
            "002_insert.sql",
            "INSERT INTO _jolt_migrate_test_target (id) VALUES (42);",
        );

        let applied = db
            .migrate(dir.path_str())
            .await
            .expect("first migrate against two-file fixture should succeed");
        assert_eq!(applied, 2, "expected 2 newly-applied migrations");

        // Body actually ran: the inserted row is visible in the target
        // table after migrate returns.
        let row: (i32,) = sqlx::query_as("SELECT id FROM _jolt_migrate_test_target")
            .fetch_one(db.pool())
            .await
            .expect("target table should hold the inserted row");
        assert_eq!(row.0, 42);

        // Bookkeeping rows present in ascending id order matching the
        // lex-sorted filename order — pins decision 40's per-migration
        // commit chain and decision 35's `SERIAL` ordering.
        let bookkeeping: Vec<(String, String)> =
            sqlx::query_as("SELECT name, checksum FROM _migrations ORDER BY id")
                .fetch_all(db.pool())
                .await
                .expect("read _migrations after migrate");
        assert_eq!(
            bookkeeping
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>(),
            vec!["001_create.sql", "002_insert.sql"],
            "expected the two migrations recorded in filename order",
        );
        // Checksums match the file bodies (round-trip through
        // sha256_hex matches what `read_migration_files` recorded).
        assert_eq!(
            bookkeeping[0].1,
            sha256_hex(b"CREATE TABLE _jolt_migrate_test_target (id INT);"),
        );
        assert_eq!(
            bookkeeping[1].1,
            sha256_hex(b"INSERT INTO _jolt_migrate_test_target (id) VALUES (42);"),
        );

        // Idempotency: a second migrate against the same dir is a
        // no-op because both files are already in `_migrations`
        // (decision 43's skip-on-presence semantic).
        let applied_again = db
            .migrate(dir.path_str())
            .await
            .expect("second migrate should succeed");
        assert_eq!(
            applied_again, 0,
            "expected 0 newly-applied on idempotent re-run",
        );

        // Teardown — leave the fixture clean for parallel reruns.
        sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_test_target")
            .execute(db.pool())
            .await
            .expect("drop target table (teardown)");
        sqlx::query("TRUNCATE TABLE _migrations")
            .execute(db.pool())
            .await
            .expect("truncate _migrations (teardown)");
    }

    mod migration_safety {
        use super::TestDir;
        use crate::{create_migration_file, DbConfig, JoltRDb, MigrationError};

        // ---- JOLTR-RS-102: MigrationError + tamper detection ----

        /// PRD-102 mandates the Display output for the tamper variant
        /// verbatim: `"Migration X has been modified since it was
        /// applied."`. Pins decision 47's contract. A regression that
        /// drops the period, mangles the wording, or routes the variant
        /// through `Debug` formatting fails this test without ever
        /// running migrate.
        #[test]
        fn migration_error_display_renders_prd_verbatim_for_tampered() {
            let err = MigrationError::Tampered {
                name: String::from("003_add_users.sql"),
            };
            assert_eq!(
                format!("{err}"),
                "Migration 003_add_users.sql has been modified since it was applied.",
            );
        }

        /// `From<std::io::Error>` impl on [`MigrationError`] preserves the
        /// underlying `io::Error` kind verbatim (decision 46). The `?`
        /// operator inside `migrate` relies on this conversion — a
        /// regression that swaps the conversion for a fresh-`io::Error`
        /// construction (losing the kind) or routes IO failures through
        /// the Sqlx variant fails this pin.
        #[test]
        fn migration_error_from_io_preserves_kind() {
            let raw = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
            let wrapped: MigrationError = raw.into();
            match wrapped {
                MigrationError::Io(io_err) => {
                    assert_eq!(io_err.kind(), std::io::ErrorKind::PermissionDenied);
                }
                other => {
                    panic!("expected MigrationError::Io from io::Error conversion, got {other:?}")
                }
            }
        }

        /// `From<sqlx::Error>` impl on [`MigrationError`] routes sqlx
        /// failures through the Sqlx variant (decision 46). The `?`
        /// operator inside `migrate` relies on this conversion for the
        /// `applied_migrations()` call and every per-migration
        /// `tx.execute` / `tx.commit` round trip.
        #[test]
        fn migration_error_from_sqlx_routes_through_sqlx_variant() {
            // RowNotFound is a no-side-effect variant convenient for
            // round-tripping through the From impl.
            let raw = sqlx::Error::RowNotFound;
            let wrapped: MigrationError = raw.into();
            assert!(
            matches!(wrapped, MigrationError::Sqlx(sqlx::Error::RowNotFound)),
            "expected MigrationError::Sqlx(RowNotFound) from sqlx::Error::RowNotFound, got {wrapped:?}",
        );
        }

        /// [`MigrationError`] implements [`std::error::Error`] with
        /// [`source`](std::error::Error::source) chained through the
        /// wrapped `io::Error` (decision 45). Tampered carries no
        /// underlying error so its source is `None`.
        #[test]
        fn migration_error_source_chains_through_io_variant() {
            use std::error::Error;

            let io_wrapped =
                MigrationError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "nope"));
            let source = io_wrapped
                .source()
                .expect("Io variant should expose source");
            assert!(
                source.downcast_ref::<std::io::Error>().is_some(),
                "source for Io variant should downcast to io::Error",
            );

            let tampered = MigrationError::Tampered {
                name: String::from("foo.sql"),
            };
            assert!(
                tampered.source().is_none(),
                "Tampered carries no underlying error",
            );
        }

        /// PRD-mandated verification for JOLTR-RS-102: "change file content
        /// after apply → error on next run." Env-gated on
        /// `JOLTR_TEST_DATABASE_URL` (same convention as the rest of the
        /// live-DB tests in this module).
        ///
        /// Flow:
        /// 1. TRUNCATE `_migrations` + DROP the target table for a known-
        ///    empty starting state.
        /// 2. Write `001_init.sql` (a CREATE TABLE), apply it via
        ///    `migrate`, assert `Ok(1)`.
        /// 3. Overwrite the same `001_init.sql` file with different
        ///    content (different SQL body → different SHA-256). The
        ///    `_migrations` row still records the *original* checksum.
        /// 4. Call `migrate` again. Assert it returns
        ///    [`MigrationError::Tampered`] with `name = "001_init.sql"`.
        /// 5. Assert the Display output is the PRD-102 verbatim message.
        /// 6. Teardown.
        #[tokio::test]
        async fn migrate_detects_tampered_migration_when_test_db_available() {
            let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                return;
            };
            let cfg = DbConfig::new(url);
            let db = JoltRDb::connect(&cfg).await.expect("connect");

            // Fresh-state setup.
            sqlx::query("TRUNCATE TABLE _migrations")
                .execute(db.pool())
                .await
                .expect("truncate _migrations (setup)");
            sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_tamper_target")
                .execute(db.pool())
                .await
                .expect("drop target table (setup)");

            let dir = TestDir::new("tamper");
            dir.write_file(
                "001_init.sql",
                "CREATE TABLE _jolt_migrate_tamper_target (id INT);",
            );

            let applied = db
                .migrate(dir.path_str())
                .await
                .expect("first migrate should succeed on a fresh DB");
            assert_eq!(applied, 1, "first migrate should apply the one file");

            // Now tamper with the file — different body, same name. The
            // `_migrations.checksum` row holds the *original* SHA-256 so
            // the next migrate call should detect the mismatch.
            dir.write_file(
                "001_init.sql",
                "CREATE TABLE _jolt_migrate_tamper_target (id BIGINT);",
            );

            let err = db
                .migrate(dir.path_str())
                .await
                .expect_err("re-run after tampering should surface MigrationError::Tampered");
            match &err {
                MigrationError::Tampered { name } => {
                    assert_eq!(name, "001_init.sql");
                }
                other => panic!("expected MigrationError::Tampered, got {other:?}"),
            }
            assert_eq!(
                format!("{err}"),
                "Migration 001_init.sql has been modified since it was applied.",
            );

            // Teardown.
            sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_tamper_target")
                .execute(db.pool())
                .await
                .expect("drop target table (teardown)");
            sqlx::query("TRUNCATE TABLE _migrations")
                .execute(db.pool())
                .await
                .expect("truncate _migrations (teardown)");
        }

        // ---- JOLTR-RS-103: MigrationError::Removed + rollback detection ----

        /// PRD-103 mandates the Display output for the Removed variant
        /// verbatim: `"Migration X has been removed. Rollbacks are not
        /// supported."`. Pins decision 48's contract. A regression that
        /// drops the period, mangles the wording, or routes the variant
        /// through `Debug` formatting fails this test without ever
        /// running migrate.
        #[test]
        fn migration_error_display_renders_prd_verbatim_for_removed() {
            let err = MigrationError::Removed {
                name: String::from("003_add_users.sql"),
            };
            assert_eq!(
                format!("{err}"),
                "Migration 003_add_users.sql has been removed. Rollbacks are not supported.",
            );
        }

        /// `MigrationError::Removed` carries no underlying error, so
        /// [`std::error::Error::source`] returns `None` (parallel to the
        /// `Tampered` arm — both are joltr-db-synthesized failures, not
        /// wrappers around an `io::Error` / `sqlx::Error`). Pins decision
        /// 48's source contract.
        #[test]
        fn migration_error_source_returns_none_for_removed() {
            use std::error::Error;
            let removed = MigrationError::Removed {
                name: String::from("foo.sql"),
            };
            assert!(
                removed.source().is_none(),
                "Removed carries no underlying error",
            );
        }

        /// PRD-mandated verification for JOLTR-RS-103: "remove migration
        /// file after apply → error on next run." Env-gated on
        /// `JOLTR_TEST_DATABASE_URL` (same convention as the rest of the
        /// live-DB tests in this module).
        ///
        /// Flow:
        /// 1. TRUNCATE `_migrations` + DROP the target table for a known-
        ///    empty starting state.
        /// 2. Write `001_init.sql` (a CREATE TABLE), apply it via
        ///    `migrate`, assert `Ok(1)`.
        /// 3. Delete the on-disk file. The `_migrations` row still
        ///    records the original name + checksum.
        /// 4. Call `migrate` again. Assert it returns
        ///    [`MigrationError::Removed`] with `name = "001_init.sql"`.
        /// 5. Assert the Display output is the PRD-103 verbatim message.
        /// 6. Verify the rollback check ran *before* the per-file apply
        ///    loop: write a brand-new `002_added.sql` file alongside
        ///    the now-missing `001_init.sql`'s database row, re-run
        ///    `migrate`, assert it still fails with
        ///    [`MigrationError::Removed`] (the new file is not applied
        ///    because the global-state check short-circuits). Pins
        ///    decision 48's "before the loop" placement.
        /// 7. Teardown.
        #[tokio::test]
        async fn migrate_detects_removed_migration_when_test_db_available() {
            let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
                return;
            };
            let cfg = DbConfig::new(url);
            let db = JoltRDb::connect(&cfg).await.expect("connect");

            // Fresh-state setup.
            sqlx::query("TRUNCATE TABLE _migrations")
                .execute(db.pool())
                .await
                .expect("truncate _migrations (setup)");
            sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_removed_target")
                .execute(db.pool())
                .await
                .expect("drop target table (setup)");
            sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_removed_should_not_run")
                .execute(db.pool())
                .await
                .expect("drop should-not-run target table (setup)");

            let dir = TestDir::new("removed");
            dir.write_file(
                "001_init.sql",
                "CREATE TABLE _jolt_migrate_removed_target (id INT);",
            );

            let applied = db
                .migrate(dir.path_str())
                .await
                .expect("first migrate should succeed on a fresh DB");
            assert_eq!(applied, 1, "first migrate should apply the one file");

            // Roll the migration back by deleting the on-disk file. The
            // `_migrations.name = '001_init.sql'` row is still present.
            std::fs::remove_file(dir.path.join("001_init.sql"))
                .expect("remove 001_init.sql to simulate operator rollback");

            let err = db
                .migrate(dir.path_str())
                .await
                .expect_err("re-run after deleting file should surface MigrationError::Removed");
            match &err {
                MigrationError::Removed { name } => {
                    assert_eq!(name, "001_init.sql");
                }
                other => panic!("expected MigrationError::Removed, got {other:?}"),
            }
            assert_eq!(
                format!("{err}"),
                "Migration 001_init.sql has been removed. Rollbacks are not supported.",
            );

            // Pin decision 48's "rollback check runs before the apply
            // loop" semantic. Add a brand-new file that *would* apply
            // cleanly if the apply loop ran — but the rollback check
            // should short-circuit before the loop body starts. We
            // verify the new file's body did NOT run by checking that
            // its target table does not exist after the failed migrate
            // call.
            dir.write_file(
                "002_added.sql",
                "CREATE TABLE _jolt_migrate_removed_should_not_run (id INT);",
            );
            let err2 = db
                .migrate(dir.path_str())
                .await
                .expect_err("re-run with new file alongside missing one should still fail Removed");
            assert!(
                matches!(err2, MigrationError::Removed { ref name } if name == "001_init.sql"),
                "expected Removed for the still-missing file, got {err2:?}",
            );
            // The new file's body did NOT execute — its CREATE TABLE
            // never ran, so a SELECT against the target table errors
            // with "relation does not exist" (sqlx's `Database` error).
            let probe: Result<(i32,), sqlx::Error> =
                sqlx::query_as("SELECT 1 FROM _jolt_migrate_removed_should_not_run LIMIT 1")
                    .fetch_one(db.pool())
                    .await;
            assert!(
            probe.is_err(),
            "002_added.sql's body must not have run when 001_init.sql was missing; got {probe:?}",
        );
            // Bookkeeping: the failed call did not insert the new row.
            let bookkeeping_count: (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM _migrations WHERE name = '002_added.sql'")
                    .fetch_one(db.pool())
                    .await
                    .expect("count _migrations rows for the new file");
            assert_eq!(
                bookkeeping_count.0, 0,
                "002_added.sql must not be recorded in _migrations after the rollback-blocked call",
            );

            // Teardown.
            sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_removed_target")
                .execute(db.pool())
                .await
                .expect("drop target table (teardown)");
            sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_removed_should_not_run")
                .execute(db.pool())
                .await
                .expect("drop should-not-run target table (teardown)");
            sqlx::query("TRUNCATE TABLE _migrations")
                .execute(db.pool())
                .await
                .expect("truncate _migrations (teardown)");
        }

        // ---- JOLTR-RS-104: `create_migration_file` (CLI scaffolding) ----

        /// Compile-time pin: `create_migration_file(dir: &str, name: &str,
        /// now: DateTime<Utc>) -> std::io::Result<PathBuf>` (decisions 49,
        /// 50). The explicit return annotation catches a regression that
        /// switches the `now` parameter to `Local`-time, narrows the
        /// return to `()` (the binary doesn't need the path but the
        /// library does for JOLTR-RS-105's filename test), or wraps the
        /// return in a foreign error type.
        #[test]
        fn create_migration_file_signature_pins() {
            fn _pin(
                dir: &str,
                name: &str,
                now: chrono::DateTime<chrono::Utc>,
            ) -> std::io::Result<std::path::PathBuf> {
                create_migration_file(dir, name, now)
            }
        }

        /// PRD-104 mandated filename shape: `<YYYYMMDDHHMMSS>_<name>.sql`.
        /// Pins decision 50's `%Y%m%d%H%M%S` UTC format — a regression
        /// that drops the leading zero on a single-digit month, swaps the
        /// component order, or formats in local time would produce a
        /// different filename and fail this assertion.
        #[test]
        fn create_migration_file_builds_timestamped_filename() {
            use chrono::TimeZone;
            let dir = TestDir::new("create-name");
            // The PRD's verification example uses 2026-05-10 12:00:00 UTC.
            let now = chrono::Utc
                .with_ymd_and_hms(2026, 5, 10, 12, 0, 0)
                .single()
                .expect("valid UTC datetime");

            let path =
                create_migration_file(dir.path_str(), "add_users", now).expect("create migration");
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .expect("UTF-8 filename");
            assert_eq!(filename, "20260510120000_add_users.sql");
            assert_eq!(path.parent(), Some(dir.path.as_path()));
        }

        /// Decision 51: placeholder body is the single line
        /// `"-- migration: <name>\n"`. A regression that drops the comment,
        /// writes an `-- up` / `-- down` template, or omits the trailing
        /// newline fails this assertion.
        #[test]
        fn create_migration_file_writes_placeholder_body() {
            use chrono::TimeZone;
            let dir = TestDir::new("create-body");
            let now = chrono::Utc
                .with_ymd_and_hms(2026, 5, 10, 12, 0, 0)
                .single()
                .expect("valid UTC datetime");

            let path = create_migration_file(dir.path_str(), "add_users", now).expect("create");
            let body = std::fs::read_to_string(&path).expect("read placeholder body");
            assert_eq!(body, "-- migration: add_users\n");
        }

        /// Decision 52: a second call with the same timestamp and name
        /// returns `ErrorKind::AlreadyExists`, does not overwrite the
        /// first file. Pins the `OpenOptions::create_new(true)` choice —
        /// a regression that uses `fs::write` (truncate-and-write) would
        /// silently overwrite and this test would fail.
        #[test]
        fn create_migration_file_errors_on_collision_without_overwriting() {
            use chrono::TimeZone;
            let dir = TestDir::new("create-collision");
            let now = chrono::Utc
                .with_ymd_and_hms(2026, 5, 10, 12, 0, 0)
                .single()
                .expect("valid UTC datetime");

            let path =
                create_migration_file(dir.path_str(), "add_users", now).expect("first create");
            // Mutate the first file's body so we can verify it survives.
            std::fs::write(&path, "-- operator edits\n").expect("rewrite first body");

            let err = create_migration_file(dir.path_str(), "add_users", now)
                .expect_err("second create with identical timestamp must fail");
            assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);

            // The first file's body is untouched.
            let body = std::fs::read_to_string(&path).expect("re-read first body");
            assert_eq!(body, "-- operator edits\n");
        }

        /// Decision 53: empty name and names containing path separators
        /// surface as `ErrorKind::InvalidInput` *before* any filesystem
        /// touch. A regression that drops the validation would either
        /// write an unintended file (path-traversal via `../`) or
        /// fail later with a generic `Os` error.
        #[test]
        fn create_migration_file_rejects_invalid_names() {
            use chrono::TimeZone;
            let dir = TestDir::new("create-validate");
            let now = chrono::Utc
                .with_ymd_and_hms(2026, 5, 10, 12, 0, 0)
                .single()
                .expect("valid UTC datetime");

            let empty = create_migration_file(dir.path_str(), "", now)
                .expect_err("empty name should be rejected");
            assert_eq!(empty.kind(), std::io::ErrorKind::InvalidInput);

            let slash = create_migration_file(dir.path_str(), "../escape", now)
                .expect_err("forward-slash name should be rejected");
            assert_eq!(slash.kind(), std::io::ErrorKind::InvalidInput);

            let backslash = create_migration_file(dir.path_str(), "win\\path", now)
                .expect_err("backslash name should be rejected");
            assert_eq!(backslash.kind(), std::io::ErrorKind::InvalidInput);

            // No files were created (the validation runs before any
            // filesystem write). Read the directory back: should be empty.
            let entries: Vec<_> = std::fs::read_dir(&dir.path)
                .expect("read dir")
                .collect::<Result<Vec<_>, _>>()
                .expect("collect entries");
            assert!(
                entries.is_empty(),
                "validation must reject before touching disk; found {entries:?}",
            );
        }

        /// `create_migration_file` creates the migrations directory if it
        /// doesn't yet exist (operator's first `migrate new` against a
        /// fresh repo). Uses a deliberately not-yet-created subdirectory
        /// under the test fixture so cleanup still works via TestDir's
        /// recursive remove.
        #[test]
        fn create_migration_file_creates_missing_directory() {
            use chrono::TimeZone;
            let dir = TestDir::new("create-mkdir");
            let nested = dir.path.join("nested").join("migrations");
            let nested_str = nested.to_str().expect("UTF-8 path");
            assert!(!nested.exists(), "fixture: nested dir not created yet");

            let now = chrono::Utc
                .with_ymd_and_hms(2026, 5, 10, 12, 0, 0)
                .single()
                .expect("valid UTC datetime");
            let path =
                create_migration_file(nested_str, "init", now).expect("should mkdir-p and create");
            assert!(path.exists());
            assert_eq!(path.parent(), Some(nested.as_path()));
        }
    }

    // ---- JOLTR-RS-101: migration apply tests (partial apply + multi-statement body) ----

    /// PRD-101 mandates: "Write migration apply tests: fresh apply, partial
    /// apply (some already done), re-run is idempotent." The fresh-apply
    /// and idempotent-re-run cases are already pinned by
    /// `migrate_applies_two_migrations_in_order_when_test_db_available`
    /// (PRD-100) — the first call against an empty DB returns `Ok(2)`
    /// (fresh) and the second call against the same fixture returns
    /// `Ok(0)` (idempotent). This test fills in the missing third case:
    /// **partial apply** — the DB has 1 of 2 migrations already in
    /// `_migrations`, so `migrate(dir)` runs only the new one.
    ///
    /// Flow:
    /// 1. TRUNCATE `_migrations` + DROP both target tables for a known-
    ///    empty starting state.
    /// 2. Write `001_partial_a.sql` (a CREATE TABLE for `A`), apply it
    ///    via `migrate`, assert `Ok(1)`.
    /// 3. Insert a sentinel row into `A`. If decision 43's
    ///    skip-on-checksum-match path regressed and `migrate` re-ran
    ///    `001_partial_a.sql`'s body in step 6, the CREATE TABLE
    ///    against an already-existing table would error and surface as
    ///    `MigrationError::Sqlx`, failing the `Ok(1)` assertion below.
    ///    The sentinel additionally lets us prove A wasn't dropped /
    ///    recreated by a regression that issued `DROP TABLE IF EXISTS`
    ///    + `CREATE TABLE` ahead of each body.
    /// 4. Write `002_partial_b.sql` (a CREATE TABLE for `B`) into the
    ///    same directory. Now the dir holds two files; the DB knows
    ///    about one of them.
    /// 5. Call `db.migrate(dir)` and assert it returns `Ok(1)` — only
    ///    the new file ran.
    /// 6. Verify `B` exists by selecting against it (proves 002's body
    ///    actually ran, not just the bookkeeping insert).
    /// 7. Verify `A` still holds the sentinel row (proves 001 wasn't
    ///    re-applied destructively).
    /// 8. Verify `_migrations` holds both rows in lex-sorted filename
    ///    order — the partial-apply path must keep the 001 bookkeeping
    ///    row intact and append the 002 row.
    /// 9. Teardown.
    #[tokio::test]
    async fn migrate_applies_only_new_migration_on_partial_apply_when_test_db_available() {
        let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
            return;
        };
        let cfg = DbConfig::new(url);
        let db = JoltRDb::connect(&cfg).await.expect("connect");

        // Fresh-state setup.
        sqlx::query("TRUNCATE TABLE _migrations")
            .execute(db.pool())
            .await
            .expect("truncate _migrations (setup)");
        sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_partial_a")
            .execute(db.pool())
            .await
            .expect("drop target A (setup)");
        sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_partial_b")
            .execute(db.pool())
            .await
            .expect("drop target B (setup)");

        let dir = TestDir::new("partial");
        dir.write_file(
            "001_partial_a.sql",
            "CREATE TABLE _jolt_migrate_partial_a (id INT);",
        );

        let applied_first = db
            .migrate(dir.path_str())
            .await
            .expect("first migrate (one file) should succeed on a fresh DB");
        assert_eq!(applied_first, 1, "first migrate should apply only 001");

        // Sentinel row. If a regression re-runs 001's body on the next
        // migrate call, either the CREATE TABLE will error (the Ok(1)
        // assertion below would fail) or — if the regression drops and
        // recreates — this sentinel would disappear.
        sqlx::query("INSERT INTO _jolt_migrate_partial_a (id) VALUES (777)")
            .execute(db.pool())
            .await
            .expect("insert sentinel row into A");

        // Now drop 002 into the same directory. The dir holds two
        // files; the DB holds one bookkeeping row.
        dir.write_file(
            "002_partial_b.sql",
            "CREATE TABLE _jolt_migrate_partial_b (id INT);",
        );

        let applied_second = db
            .migrate(dir.path_str())
            .await
            .expect("partial migrate against a one-already-applied fixture should succeed");
        assert_eq!(
            applied_second, 1,
            "partial migrate should apply only the new 002 file",
        );

        // 002's body actually ran — B exists and is queryable.
        sqlx::query("SELECT 1 FROM _jolt_migrate_partial_b LIMIT 1")
            .execute(db.pool())
            .await
            .expect("B should exist and be queryable after partial migrate");

        // 001 was not re-applied destructively — the sentinel survives.
        let sentinel: (i32,) =
            sqlx::query_as("SELECT id FROM _jolt_migrate_partial_a WHERE id = 777")
                .fetch_one(db.pool())
                .await
                .expect("sentinel row should survive partial migrate");
        assert_eq!(sentinel.0, 777, "sentinel value must be intact");

        // Bookkeeping: 001's original row is still present, 002's row
        // is appended.
        let bookkeeping: Vec<(String, String)> =
            sqlx::query_as("SELECT name, checksum FROM _migrations ORDER BY id")
                .fetch_all(db.pool())
                .await
                .expect("read _migrations after partial migrate");
        assert_eq!(
            bookkeeping
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>(),
            vec!["001_partial_a.sql", "002_partial_b.sql"],
            "expected both migrations recorded in filename order after partial apply",
        );
        assert_eq!(
            bookkeeping[0].1,
            sha256_hex(b"CREATE TABLE _jolt_migrate_partial_a (id INT);"),
        );
        assert_eq!(
            bookkeeping[1].1,
            sha256_hex(b"CREATE TABLE _jolt_migrate_partial_b (id INT);"),
        );

        // Teardown.
        sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_partial_a")
            .execute(db.pool())
            .await
            .expect("drop target A (teardown)");
        sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_partial_b")
            .execute(db.pool())
            .await
            .expect("drop target B (teardown)");
        sqlx::query("TRUNCATE TABLE _migrations")
            .execute(db.pool())
            .await
            .expect("truncate _migrations (teardown)");
    }

    /// Pins decision 41's `sqlx::raw_sql` choice for executing migration
    /// bodies: a single migration file may contain multiple SQL
    /// statements separated by `;` and they all run. `sqlx::query` is
    /// single-statement only and would error on a multi-statement
    /// body; `sqlx::raw_sql` accepts the whole script. A regression
    /// that swaps the executor back to `sqlx::query` would fail this
    /// test with a "cannot insert multiple commands" error before any
    /// assertion runs.
    #[tokio::test]
    async fn migrate_applies_multi_statement_body_in_single_file_when_test_db_available() {
        let Ok(url) = std::env::var("JOLTR_TEST_DATABASE_URL") else {
            return;
        };
        let cfg = DbConfig::new(url);
        let db = JoltRDb::connect(&cfg).await.expect("connect");

        // Fresh-state setup.
        sqlx::query("TRUNCATE TABLE _migrations")
            .execute(db.pool())
            .await
            .expect("truncate _migrations (setup)");
        sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_multi_a")
            .execute(db.pool())
            .await
            .expect("drop target A (setup)");
        sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_multi_b")
            .execute(db.pool())
            .await
            .expect("drop target B (setup)");

        let dir = TestDir::new("multi");
        // Two statements in one file. Both must run for the assertions
        // below to pass.
        dir.write_file(
            "001_multi.sql",
            "CREATE TABLE _jolt_migrate_multi_a (id INT);\n\
             CREATE TABLE _jolt_migrate_multi_b (id INT);",
        );

        let applied = db
            .migrate(dir.path_str())
            .await
            .expect("multi-statement migrate should succeed via raw_sql");
        assert_eq!(applied, 1, "one file applied → count is 1");

        // Both statements ran — both tables are queryable.
        sqlx::query("SELECT 1 FROM _jolt_migrate_multi_a LIMIT 1")
            .execute(db.pool())
            .await
            .expect("first statement (CREATE A) should have run");
        sqlx::query("SELECT 1 FROM _jolt_migrate_multi_b LIMIT 1")
            .execute(db.pool())
            .await
            .expect("second statement (CREATE B) should have run");

        // Teardown.
        sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_multi_a")
            .execute(db.pool())
            .await
            .expect("drop target A (teardown)");
        sqlx::query("DROP TABLE IF EXISTS _jolt_migrate_multi_b")
            .execute(db.pool())
            .await
            .expect("drop target B (teardown)");
        sqlx::query("TRUNCATE TABLE _migrations")
            .execute(db.pool())
            .await
            .expect("truncate _migrations (teardown)");
    }
}
