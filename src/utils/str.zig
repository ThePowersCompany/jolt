const std = @import("std");
const Allocator = std.mem.Allocator;

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
            switch (try source.peekNextTokenType()) {
                inline .string => |str| {
                    const data = try T.jsonParse(alloc, source, options);
                    return .{
                        .str = str,
                        .data = data,
                    };
                },
                else => return error.UnexpectedToken,
            }
        }
    };
}
