const std = @import("std");
const Allocator = std.mem.Allocator;

const pg = @import("pg");

pub fn Str(comptime T: type) type {
    return struct {
        str: []const u8,
        data: T,

        const Self = @This();

        pub fn paramParse(alloc: Allocator, param: []const u8) !Self {
            return .{
                .str = param,
                .data = try T.paramParse(alloc, param),
            };
        }

        pub fn jsonParse(alloc: Allocator, source: anytype, options: anytype) !Self {
            _ = options;

            const peek: std.json.TokenType = try source.peekNextTokenType();
            switch (peek) {
                inline .string => {
                    const token: std.json.Token = try source.nextAlloc(alloc, .alloc_if_needed);
                    const str: []const u8 = switch (token) {
                        .string, .allocated_string => |str| str,
                        else => return error.UnexpectedToken,
                    };
                    const data = T.paramParse(alloc, str) catch return error.InvalidCharacter;
                    return .{
                        .str = str,
                        .data = data,
                    };
                },
                else => return error.UnexpectedToken,
            }
        }

        pub fn bind(self: *const Self, stmt: *pg.Stmt) !void {
            try stmt.bind(self.str);
        }
    };
}

test "paramParse date string" {
    const Date = @import("datetime.zig").Date;
    const DateStr = Str(Date);

    const parsed = try DateStr.paramParse(std.testing.allocator, "1970-01-01");
    try std.testing.expectEqualStrings("1970-01-01", parsed.str);
}

test "jsonParse date string" {
    const Date = @import("datetime.zig").Date;
    const DateStr = Str(Date);

    const alloc = std.testing.allocator;
    const parsed = try std.json.parseFromSlice(DateStr, alloc, "\"1970-01-01\"", .{});
    defer parsed.deinit();

    try std.testing.expectEqualStrings("1970-01-01", parsed.value.str);
}
