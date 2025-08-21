/// Utility types
const std = @import("std");
const json = std.json;

pub fn Unwrap(T: type) type {
    const info = @typeInfo(T);
    if (info == .optional) return Unwrap(info.optional.child);
    return T;
}

pub fn unwrap(T: type, t: T) ?Unwrap(T) {
    const info = @typeInfo(T);
    if (info == .optional) {
        if (t) |inner| {
            return unwrap(info.optional.child, inner);
        }
    }
    return t;
}

pub fn Optional(comptime T: type) type {
    return union(enum) {
        not_provided,
        value: T,

        const Self = @This();

        /// Returns the value if it is present in this Optional, otherwise returns null.
        /// This function will unwrap multiple levels of null, down to the actual value.
        pub fn get(self: Self) ?Unwrap(T) {
            if (self == .value) return unwrap(T, self.value);
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
        foo: Optional(?i32) = .not_provided,
        bar: Optional(?i32) = .not_provided,
        baz: Optional(?i32) = .not_provided,
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

    {
        const inner = foo.foo.get();
        if (inner) |v| {
            try std.testing.expect(v == 123);
        } else try std.testing.expect(false);
    }

    {
        const inner = foo.bar.get();
        if (inner) |_| {
            try std.testing.expect(false);
        } else try std.testing.expect(inner == null);
    }

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
