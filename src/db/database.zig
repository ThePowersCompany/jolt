const std = @import("std");
const pg = @import("pg");
const Error = pg.Error;
const Pool = pg.Pool;
const Result = pg.Result;
const Conn = pg.Conn;
const Listener = pg.Listener;
const eql = std.mem.eql;

pub const Self = @This();

var pool: *Pool = undefined;

pub const DbOptions = struct {
    host: []const u8,
    port: u16,
    database: []const u8,
    username: []const u8,
    password: []const u8,
    timeout: u32 = 10_000,
    pool_size: u16 = 10,
};

pub fn init(alloc: std.mem.Allocator, opts: DbOptions) !void {
    pool = try Pool.init(alloc, .{
        .size = opts.pool_size,
        .connect = .{
            .host = opts.host,
            .port = opts.port,
            // TODO: Optimize these for our use case to prevent allocations during queries
            // read_buffer: ?u16 = null,
            // result_state_size: u16 = 32,
        },
        .auth = .{
            .username = opts.username,
            .database = opts.database,
            .password = opts.password,
            .timeout = opts.timeout,
        },
    });
}

pub fn deinit() void {
    pool.deinit();
}

pub fn acquireConnection() !*Conn {
    return pool.acquire();
}

pub fn newListener() !Listener {
    return pool.newListener();
}

var err_map = std.StaticStringMap(PGError).initComptime(.{
    .{ "23001", PGError.Restrict },
    .{ "23503", PGError.ForeignKey },
    .{ "23505", PGError.Unique },
    .{ "23514", PGError.Check },
    .{ "42000", PGError.Syntax },
    .{ "42601", PGError.Syntax },
    // E.g. text input does not match any enum states
    .{ "22P02", PGError.InvalidTextRepresentation },
});

pub const PGError = error{
    Restrict,
    ForeignKey,
    Unique,
    Check,
    Syntax,
    InvalidTextRepresentation,
    // anyerror, i.e. not a PGError.
    Any,
};

fn strEquals(s1: []const u8, s2: []const u8) bool {
    return std.mem.eql(u8, s1, s2);
}

pub fn isIntegrityConstraintViolation(err: anyerror, conn: *Conn) bool {
    return constraintViolation(err, conn) != null;
}

pub fn constraintViolation(err: anyerror, conn: *Conn) ?[]const u8 {
    if (err != error.PG) return null;
    const pge = conn.err orelse return null;
    if (!std.mem.startsWith(u8, pge.code, "23")) return null;
    return pge.constraint;
}

pub fn refineError(err: anyerror, conn: *Conn) PGError {
    if (err != error.PG) return PGError.Any;
    const pge = conn.err orelse return PGError.Any;
    return err_map.get(pge.code) orelse PGError.Any;
}

pub fn logError(err: anyerror, conn: *Conn) PGError {
    const refined = refineError(err, conn);
    std.log.err("{any}:", .{refined});
    if (conn.err) |e| printPgError(e);
    return refined;
}

pub fn printPgError(err: Error) void {
    const info = @typeInfo(Error);
    inline for (info.@"struct".fields) |field| {
        const field_info = @typeInfo(field.type);
        if (field_info == .pointer and field_info.pointer.child == u8) {
            std.log.err("{s}: {s}", .{ field.name, @field(err, field.name) });
        } else if (field_info == .optional) {
            const unwrapped_info = @typeInfo(field_info.optional.child);
            if (unwrapped_info == .pointer and unwrapped_info.pointer.child == u8) {
                const data = @field(err, field.name);
                if (data) |d| std.log.err("{s}: {s}", .{ field.name, d });
            }
        }
    }
}
