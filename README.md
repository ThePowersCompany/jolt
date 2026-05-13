# JoltR

A high-performance web framework for people who prefer their code to work the first time.

JoltR is a full-stack Rust web framework — endpoint routing, middleware, WebSockets, SSE, pub/sub, JWT auth, database migrations, template rendering, and compile-time TypeScript type generation. It started as a Zig project, then grew up and got a job.

## Why Rust

Zig is brilliant if you enjoy debugging segfaults at 2am, rewriting your entire codebase every six months when the standard library changes, and pretending "comptime" makes up for not having a real macro system. For everyone else, there's Rust.

| Zig experience | Rust equivalent |
|---|---|
| `@import("std").debug.print` | `dbg!()` |
| "undefined behavior is just a suggestion" | the borrow checker politely declines |
| 45-minute compile times for C dependencies | crates.io exists |
| Rewriting JSON parsing from scratch | `serde_json::from_str` |
| `@compileError` | actual readable compiler errors |
| "it works on my machine" | `cargo test` passes in CI |
| `@ptrCast(*anyopaque, @alignCast(...))` | `.clone()` |
| LLVM cross-compilation promises that still need a C toolchain | `cross` or GitHub Actions |

## Crates

| Crate | Purpose |
|---|---|
| `joltr-core` | Server builder, router, request/response, WS, SSE, pub/sub, tasks, TLS |
| `joltr-macros` | `#[endpoint]`, `#[derive(AutoMiddleware)]`, `#[derive(PatchQuery)]`, `#[derive(TsExport)]` |
| `joltr-db` | sqlx pool, file-based migrations with SHA-256 checksums, LISTEN/NOTIFY via `PgListener` |
| `joltr-utils` | JWT (HS256/384/512, RS256/384/512, ES256/384), PBKDF2 hashing, UUID v4/v7, `Optional<T>` tri-state |
| `joltr-templates` | Handlebars rendering — no C FFI, no custom parser, just `handlebars` |

## Quick Start

```rust
use joltr_core::prelude::*;

struct Hello;

#[joltr_macros::endpoint("/hello")]
impl Hello {
    #[get]
    fn greet(&self, req: Request) -> Response<String> {
        Response::ok("Hello from JoltR — compiled in under 10 seconds")
    }
}

#[tokio::main]
async fn main() {
    JoltRServer::new()
        .start("0.0.0.0:3000")
        .await
        .unwrap();
}
```

## Features

- **Derive-driven endpoints** — `#[endpoint("/path")]` with `#[get]`/`#[post]`/`#[put]`/`#[patch]`/`#[delete]` on impl blocks
- **Auto-middleware** — `#[derive(AutoMiddleware)]` inspects struct fields and auto-wires body parsing, query extraction, CORS, and custom middleware steps
- **WebSocket auth** — JWT extraction from `Sec-WebSocket-Protocol: joltr-jwt, <token>` header
- **Pub/sub** — `tokio::sync::broadcast` channels keyed in a `DashMap`, no C pubsub daemon required
- **SSE** — Server-sent events via `axum::response::Sse`
- **TypeScript typegen** — `#[derive(TsExport)]` walks your Rust types at compile time and emits `types.d.ts`
- **PATCH/UPSERT** — `#[derive(PatchQuery)]` generates dynamic SQL from `Optional<T>` fields
- **Migrations** — File-based SQL migrations with SHA-256 checksums, auto-applied at startup
- **Background tasks** — Scheduled tasks via `tokio::time::interval` with retry logic

## Implementation Notes

- PostgreSQL LISTEN/NOTIFY uses `sqlx::postgres::PgListener`, not a separate `tokio-postgres` client. Listener streams use dedicated connections; publishes use `SELECT pg_notify($1, $2)` through the regular pool.
- JWT support intentionally exceeds the original HS256-only port target: HMAC, RSA, and ECDSA algorithms share the same `JwtConfig::new(key_material, algorithm)` API, with PEM key material for asymmetric algorithms.

## Final Rust Port Verification

Run the final local verification path from the workspace root:

```sh
cargo fmt --check
cargo check --workspace --all-targets
cargo test --workspace
docker build -t joltr-basic-example:local -f examples/basic/Dockerfile .
JOLTR_TYPES_OUT=target/joltr-basic-example-types.d.ts cargo run -q -p joltr-basic-example -- --generate-types
examples/basic/integration/run.sh
```

The Docker image builds the `joltr-basic-example` binary from the local workspace crates, generates `/workspace/types.d.ts`, and starts without external services when `DATABASE_URL` is unset.

The TypeScript integration test starts the example container, waits for `GET /api/test/typed`, copies the generated `types.d.ts`, type-checks against `TypedTestResponse`, and validates this stable JSON contract:

```json
{
  "contract_version": 1,
  "service": "joltr-basic-example",
  "ok": true,
  "features": ["endpoint-macro", "ts-export"]
}
```

When running the example directly with `cargo run -p joltr-basic-example`, the endpoint is available at `http://127.0.0.1:3000/api/test/typed`.

## FAQ

**Is this a port of the Zig JoltR?**

Yes. The original Zig codebase still exists in `src/` as a historical artifact and compatibility reference. The Rust workspace is the primary development target.

**Why keep the Zig code?**

Sentimental value. Also, it's useful to occasionally run `zig build` and remember why we switched.

**Does JoltR support Zig?**

The Zig runtime was retired. The `facil.io` C dependency tree is a submodule for posterity. If you need Zig interop, compile your Zig to a shared library and call it from a Rust endpoint — the borrow checker doesn't judge.

**Can I contribute Zig code?**

We prefer Rust contributions. Zig PRs will be reviewed with the same care and attention that Zig gives to its documentation — which is to say, sporadically and without clear guidance. If you really want to, look in `src/` and `build.zig`, but don't expect `@import` paths to survive the next compiler release.

## License

MIT
