//! Ambient `std.Io` access for jolt.
//!
//! Explicit `io` parameters are used at architectural boundaries (JoltServer,
//! database pool, migrations, typegen). Leaf utilities (uuid, datetime,
//! password, ...) reach for the ambient Io here so their signatures stay
//! io-free. `JoltServer.init` calls `set`; anything running before that (or
//! outside a server) falls back to a blocking single-threaded Io. Tests always
//! use `std.testing.io`.

const std = @import("std");
const builtin = @import("builtin");

var global: ?std.Io = null;

/// Must be called before facil.io worker threads spawn (`JoltServer.init` does).
pub fn set(io: std.Io) void {
    global = io;
}

pub fn get() std.Io {
    if (builtin.is_test) return std.testing.io;
    return global orelse std.Io.Threaded.global_single_threaded.io();
}

/// Unix timestamp in seconds.
pub fn nowSec() i64 {
    return std.Io.Clock.real.now(get()).toSeconds();
}

/// Unix timestamp in milliseconds.
pub fn nowMs() i64 {
    return std.Io.Clock.real.now(get()).toMilliseconds();
}

/// Unix timestamp in microseconds.
pub fn nowUs() i64 {
    return std.Io.Clock.real.now(get()).toMicroseconds();
}

/// Fill `buf` with random bytes from a per-thread CSPRNG.
pub fn randomBytes(buf: []u8) void {
    get().random(buf);
}
