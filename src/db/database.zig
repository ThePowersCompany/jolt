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

/// Optimize read_buffer and result_state_size for your use case,
/// to prevent allocations during queries.
pub const DbOptions = struct {
    host: []const u8,
    port: u16,
    database: []const u8,
    username: []const u8,
    password: []const u8,
    timeout: u32 = 10_000,
    pool_size: u16 = 10,
    read_buffer: ?u16 = null,
    result_state_size: u16 = 32,
};

pub fn init(alloc: std.mem.Allocator, opts: DbOptions) !void {
    pool = try Pool.init(alloc, .{
        .size = opts.pool_size,
        .connect = .{
            .host = opts.host,
            .port = opts.port,
            .read_buffer = opts.read_buffer,
            .result_state_size = opts.result_state_size,
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
    .{ "23502", PGError.NotNull },
    .{ "23503", PGError.ForeignKey },
    .{ "23505", PGError.Unique },
    .{ "23514", PGError.Check },
    .{ "23P01", PGError.Exclusion },
    .{ "42601", PGError.Syntax },
    // E.g. text input does not match any enum states
    .{ "22P02", PGError.InvalidTextRepresentation },
});

const err_class_map = std.StaticStringMap(PGError).initComptime(.{
    .{ "22", PGError.DataException },
    .{ "23", PGError.IntegrityConstraintViolation },
    .{ "42", PGError.SyntaxOrAccessRuleViolation },
});

pub const PGError = error{
    Restrict,
    NotNull,
    ForeignKey,
    Unique,
    Check,
    Exclusion,
    Syntax,
    InvalidTextRepresentation,
    /// Class 22 errors
    DataException,
    /// Class 23 errors
    IntegrityConstraintViolation,
    /// Class 42 errors
    SyntaxOrAccessRuleViolation,
    // anyerror, i.e. not a PGError.
    Any,
};

fn strEquals(s1: []const u8, s2: []const u8) bool {
    return std.mem.eql(u8, s1, s2);
}

pub fn isIntegrityConstraintViolation(err: anyerror, conn: *Conn) bool {
    if (err != error.PG) return false;
    const pge = conn.err orelse return false;
    return errorClass(pge.code) == PGError.IntegrityConstraintViolation;
}

pub fn constraintViolation(err: anyerror, conn: *Conn) ?[]const u8 {
    if (!isIntegrityConstraintViolation(err, conn)) return null;
    const pge = conn.err orelse return null;
    return pge.constraint;
}

/// Returns whether a database error names the given constraint.
pub fn isConstraintViolation(err: anyerror, conn: *Conn, name: []const u8) bool {
    if (constraintViolation(err, conn)) |constraint| {
        return eql(u8, constraint, name);
    }
    return false;
}

pub fn refineError(err: anyerror, conn: *Conn) PGError {
    if (err != error.PG) return PGError.Any;
    const pge = conn.err orelse return PGError.Any;
    if (err_map.get(pge.code)) |refined| return refined;
    return errorClass(pge.code);
}

fn errorClass(code: []const u8) PGError {
    if (code.len != 5) return PGError.Any;
    return err_class_map.get(code[0..2]) orelse PGError.Any;
}

pub fn logError(err: anyerror, conn: *Conn) PGError {
    const refined = refineError(err, conn);
    std.log.err("{} - {any}:", .{ err, refined });
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
