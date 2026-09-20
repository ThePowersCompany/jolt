const std = @import("std");
const Allocator = std.mem.Allocator;

const pg = @import("pg");

pub fn Str(comptime T: type) type {
    return struct {
        str: []const u8,
        data: T,

        pub const _repr: type = []const u8;

        const Self = @This();

        pub fn paramParse(alloc: Allocator, param: []const u8) !Self {
            return .{
                .str = param,
                .data = try T.paramParse(alloc, param),
            };
        }

        pub fn jsonParse(alloc: Allocator, source: anytype, options: anytype) !Self {
            const peek: std.json.TokenType = try source.peekNextTokenType();
            switch (peek) {
                inline .string => {
                    const str = try std.json.innerParse([]const u8, alloc, source, options);
                    const data = T.paramParse(alloc, str) catch return error.InvalidCharacter;
                    return .{
                        .str = str,
                        .data = data,
                    };
                },
                else => return error.UnexpectedToken,
            }
        }

        pub fn toPgzParam(self: *const Self) []const u8 {
            return self.str;
        }

        pub fn jsonStringify(self: Self, out: anytype) !void {
            try out.write(self.str);
        }

        pub fn fromPgzRow(value: pg.Result.State.Value, _: i32) !Self {
            if (value.is_null) return error.UnexpectedNull;
            // Row decoding cannot allocate, the string is borrowed
            var buffer = std.heap.FixedBufferAllocator.init(&.{});
            return paramParse(buffer.allocator(), value.data) catch return error.InvalidType;
        }

        pub fn pgzMoveOwner(self: Self, alloc: Allocator) !Self {
            return .{ .str = try alloc.dupe(u8, self.str), .data = self.data };
        }

        pub fn bind(self: *const Self, stmt: *pg.Stmt) !void {
            try stmt.bind(self.str);
        }
    };
}

const DateStr = Str(@import("datetime.zig").Date);

test "paramParse date string" {
    const parsed = try DateStr.paramParse(std.testing.allocator, "1970-01-01");
    try std.testing.expectEqualStrings("1970-01-01", parsed.str);
}

test "jsonParse date string" {
    const alloc = std.testing.allocator;

    const parsed = try std.json.parseFromSlice(DateStr, alloc, "\"1970-01-01\"", .{});
    defer parsed.deinit();

    try std.testing.expectEqualStrings("1970-01-01", parsed.value.str);
}

test "jsonStringify date string" {
    const alloc = std.testing.allocator;

    const date = try DateStr.paramParse(alloc, "2026-09-19");
    const json = try std.json.Stringify.valueAlloc(alloc, date, .{});
    defer alloc.free(json);

    try std.testing.expectEqualStrings("\"2026-09-19\"", json);
}

test "pgzMoveOwner copies the string and preserves the parsed date" {
    const alloc = std.testing.allocator;
    const original = try DateStr.paramParse(alloc, "2026-09-19");

    const copy = try original.pgzMoveOwner(alloc);
    defer alloc.free(copy.str);

    try std.testing.expectEqualStrings(original.str, copy.str);
    try std.testing.expect(original.str.ptr != copy.str.ptr);
    try std.testing.expectEqualDeep(original.data, copy.data);
}

test "fromPgzRow accepts date text regardless of the column OID" {
    const value = pg.Result.State.Value{ .is_null = false, .data = "2026-09-19" };
    const date = try DateStr.fromPgzRow(value, pg.types.Int32.oid.decimal);

    try std.testing.expectEqualStrings("2026-09-19", date.str);
}

test "fromPgzRow rejects null and invalid date values" {
    const oid = pg.types.String.oid.decimal;

    try std.testing.expectError(
        error.UnexpectedNull,
        DateStr.fromPgzRow(.{ .is_null = true, .data = "" }, oid),
    );

    try std.testing.expectError(
        error.InvalidType,
        DateStr.fromPgzRow(.{ .is_null = false, .data = "invalid" }, oid),
    );

    try std.testing.expectError(
        error.InvalidType,
        DateStr.fromPgzRow(.{ .is_null = false, .data = &.{ 0, 0, 0, 1 } }, oid),
    );
}

test "date string input rejects year zero" {
    try std.testing.expectError(
        error.InvalidDate,
        DateStr.paramParse(std.testing.allocator, "0000-01-01"),
    );

    try std.testing.expectError(
        error.InvalidCharacter,
        std.json.parseFromSlice(DateStr, std.testing.allocator, "\"0000-01-01\"", .{}),
    );
}
