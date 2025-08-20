/// Utility types
const std = @import("std");
const json = std.json;

pub fn Optional(comptime T: type) type {
    return union(enum) {
        not_provided,
        value: ?T,

        const Self = @This();

        /// Returns the value if it is present in this Optional, otherwise returns null.
        pub fn get(self: Self) ?T {
            if (self == .value) {
                return self.value;
            }
            return null;
        }

        pub fn jsonParse(
            allocator: std.mem.Allocator,
            source: anytype,
            options: json.ParseOptions,
        ) json.ParseError(@TypeOf(source.*))!Self {
            const val = try json.innerParse(?T, allocator, source, options);
            if (val) |v| return .{ .value = v } else return .{ .value = null };
        }

        pub fn jsonParseFromValue(
            allocator: std.mem.Allocator,
            source: json.Value,
            options: json.ParseOptions,
        ) json.ParseError(@TypeOf(allocator))!Self {
            return switch (source) {
                .null => .{ .value = null },
                else => .{ .value = try json.parseFromValueLeaky(T, allocator, source, options) },
            };
        }
    };
}

test "Optional" {
    const Foo = struct {
        foo: Optional(i32) = .not_provided,
        bar: Optional(i32) = .not_provided,
        baz: Optional(i32) = .not_provided,
    };

    const payload =
        \\ {
        \\   "foo": 123,
        \\   "bar": null
        \\ }
    ;

    const alloc = std.testing.allocator;

    const parsed = try json.parseFromSlice(Foo, alloc, payload, .{});
    defer parsed.deinit();

    const foo = parsed.value;

    if (foo.foo.get()) |v| {
        try std.testing.expect(v == 123);
    }

    try std.testing.expect(foo.foo.value == 123);
    try std.testing.expect(foo.bar.value == null);
    try std.testing.expect(foo.baz == .not_provided);

    if (foo.foo.value) |val| {
        try std.testing.expect(val == 123);
    } else try std.testing.expect(false);

    if (foo.bar.value) |_| try std.testing.expect(false);
}
