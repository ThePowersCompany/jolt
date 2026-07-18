# Jolt API Server

> A highly ergonomic framework for rapidly building API servers in the Zig programming language.

Zig version: [0.16.0](https://ziglang.org/documentation/0.16.0/)

Derived from [Zap](https://github.com/zigzap/zap) (and [Facil.io](https://facil.io/)).

Built-in support for PostgreSQL via the [`pg.zig`](https://github.com/karlseguin/pg.zig) library.

## Zig 0.16 migration notes

Zig 0.16 moved filesystem access, wall-clock time, randomness, and process
spawning onto the `std.Io` interface. Jolt takes the `Io` at its boundaries and
provides it ambiently to leaf utilities (`uuid`, `datetime`, `password`, ...),
so endpoint and middleware code is unchanged.

- Use the 0.16 main convention and hand process resources to jolt:

  ```zig
  pub fn main(init: std.process.Init) !void {
      var server = try jolt.JoltServer.init(init.gpa, init.io, init.environ_map, .{
          .port = 3333,
          .threads = 2,
      });
      // ...
  }
  ```

- `JoltServer.init(alloc, io, env_map, opts)` — the allocator must be
  thread-safe (per-request arenas and tasks allocate from it on facil.io
  worker threads); `env_map` is a `*std.process.Environ.Map`.
- `database.init`, `migrateDatabase`, `newDatabaseMigration`, `resetDatabase`,
  and `generateTypesFile` now take an `io: std.Io` parameter.
- `EnabledContext.env` is now `*std.process.Environ.Map`.
- Removed: `Request.parametersToOwnedList`, `parametersToOwnedStrList`,
  `cookiesToOwnedList`, `cookiesToOwnedStrList`, and the `Array_Binfile`
  multi-file upload param (pre-0.15 leftovers that no longer compiled).
