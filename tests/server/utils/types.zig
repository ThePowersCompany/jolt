const std = @import("std");
const Allocator = std.mem.Allocator;

const jolt = @import("jolt");
const UnionRepr = jolt.UnionRepr;

pub const IdOrAuto = union(enum) {
    id: i32,
    auto,

    pub const _repr: UnionRepr = .untagged;

    /// Override default union parsing behavior
    pub fn jsonParse(alloc: Allocator, source: anytype, opts: std.json.ParseOptions) !@This() {
        const token: std.json.TokenType = try source.peekNextTokenType();
        return switch (token) {
            .number => .{ .id = try std.json.innerParse(i32, alloc, source, opts) },
            .string => {
                const str: []const u8 = try std.json.innerParse([]const u8, alloc, source, opts);
                if (!std.mem.eql(u8, str, "auto")) {
                    return std.json.ParseFromValueError.InvalidEnumTag;
                }
                return .auto;
            },
            else => error.UnexpectedToken,
        };
    }
};
