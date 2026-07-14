/// Utility types
const std = @import("std");
const json = std.json;

const pg = @import("pg");

const expect = std.testing.expect;
const expectEqual = std.testing.expectEqual;
const expectEqualStrings = std.testing.expectEqualStrings;
const expectError = std.testing.expectError;

pub fn Unwrap(T: type) type {
    if (isOptional(T)) return Unwrap(T.childType());
    if (@typeInfo(T) == .optional) return Unwrap(@typeInfo(T).optional.child);
    return T;
}

pub fn unwrap(T: type, t: T) ?Unwrap(T) {
    const info = @typeInfo(T);
    if (info == .optional) {
        if (t) |inner| {
            return unwrap(info.optional.child, inner);
        }
        return null;
    }
    return t;
}

pub fn unwrapPtr(T: type, t: *T) ?*Unwrap(T) {
    const info = @typeInfo(T);
    if (info == .optional) {
        if (t.*) |*inner| {
            return unwrapPtr(info.optional.child, inner);
        }
        return null;
    }
    return t;
}

pub fn isOptional(comptime T: type) bool {
    return @typeInfo(T) == .@"union" and @hasField(T, "value") and @hasField(T, "not_provided");
}

pub fn Optional(comptime T: type) type {
    return union(enum) {
        not_provided,
        value: T,

        const Self = @This();

        pub fn childType() type {
            return T;
        }

        /// Returns the value if it is present in this Optional, otherwise returns null.
        /// This function will unwrap multiple levels of null, down to the actual value.
        pub fn get(self: Self) ?Unwrap(T) {
            return switch (self) {
                .value => |v| unwrap(T, v),
                else => null,
            };
        }

        /// Returns a pointer to the value if it is present in this Optional, otherwise returns null.
        /// This function will unwrap multiple levels of null, down to the actual value.
        pub fn getPtr(self: *Self) ?*Unwrap(T) {
            return switch (self.*) {
                .value => |*v| unwrapPtr(T, v),
                else => null,
            };
        }

        /// Wraps a value in an Optional.
        pub fn to(value: ?T) Optional(T) {
            if (value) |v| return .{ .value = v };
            return .not_provided;
        }

        pub fn jsonParse(
            allocator: std.mem.Allocator,
            source: anytype,
            options: json.ParseOptions,
        ) json.ParseError(@TypeOf(source.*))!Self {
            const info = @typeInfo(T);
            if (info == .optional) {
                return .{ .value = try json.innerParse(T, allocator, source, options) };
            }
            // Supports implicitly parsing `null` to `.not_provided`
            const value: ?T = try json.innerParse(?T, allocator, source, options);
            return if (value) |v| .{ .value = v } else .not_provided;
        }

        pub fn jsonParseFromValue(
            allocator: std.mem.Allocator,
            source: json.Value,
            options: json.ParseOptions,
        ) json.ParseError(@TypeOf(allocator))!Self {
            const info = @typeInfo(T);
            if (info == .optional) {
                return .{ .value = try json.parseFromValueLeaky(T, allocator, source, options) };
            }
            // Supports implicitly parsing `null` to `.not_provided`
            const value: ?T = try json.parseFromValueLeaky(?T, allocator, source, options);
            return if (value) |v| .{ .value = v } else .not_provided;
        }

        pub fn jsonStringify(self: *const Self, jws: anytype) !void {
            switch (self.*) {
                .value => |v| try json.Stringify.write(jws, v),
                .not_provided => {
                    // It's not possible to stringify an optional that's not provided,
                    // so the only logical action is to error.
                    // It's the responsibility of the parent struct (JsonObject)
                    // to skip stringifying the optional field.
                    return json.Stringify.Error.WriteFailed;
                },
            }
        }

        pub fn fromPgzRow(
            value: pg.Result.State.Value,
            oid: i32,
        ) !Self {
            if (value.is_null) {
                const info = @typeInfo(T);
                if (info == .optional) {
                    return .{ .value = null };
                }
                return .not_provided;
            }
            return .{ .value = try pg.types.decodeScalar(.safe, Unwrap(T), value.data, oid) };
        }

        pub fn pgzMoveOwner(self: Self, alloc: std.mem.Allocator) !Self {
            const info = @typeInfo(T);
            if (info == .optional) @compileError("Do not wrap nullable in an Optional");
            if (comptime (T == []u8 or T == []const u8)) {
                if (self == .value) {
                    return .{ .value = try alloc.dupe(u8, self.value) };
                }
            } else if (info == .pointer) {
                @compileError("Optional does not support slices, except strings");
            }
            return self;
        }
    };
}

test "Unwrap: concrete type returns unchanged" {
    try expectEqual(i32, Unwrap(i32));
    try expectEqual([]const u8, Unwrap([]const u8));

    const E = enum { a, b };
    try expectEqual(E, Unwrap(E));
}

test "Unwrap: unwraps native ?T" {
    try expectEqual(i32, Unwrap(?i32));
    try expectEqual([]const u8, Unwrap(?[]const u8));
}

test "Unwrap: unwraps nested native optionals" {
    try expectEqual(i32, Unwrap(??i32));
    try expectEqual([]const u8, Unwrap(???[]const u8));
}

test "Unwrap: unwraps Optional wrapper" {
    try expectEqual(i32, Unwrap(Optional(i32)));
    try expectEqual([]const u8, Unwrap(Optional([]const u8)));
}

test "Unwrap: unwraps Optional wrapping native optional" {
    try expectEqual(i32, Unwrap(Optional(?i32)));
    try expectEqual([]const u8, Unwrap(Optional(??[]const u8)));
}

test "Unwrap: unwraps nested Optional wrappers with native optionals" {
    try expectEqual(i32, Unwrap(Optional(Optional(?i32))));
}

test "Unwrap: preserves struct and union types" {
    const S = struct { x: i32 };
    const U = union(enum) { a: i32, b: []const u8 };
    try expectEqual(S, Unwrap(S));
    try expectEqual(U, Unwrap(U));
    try expectEqual(S, Unwrap(?S));
    try expectEqual(U, Unwrap(Optional(?U)));
}

test "Optional.getPtr" {
    {
        var opt: Optional(i32) = .to(123);
        opt.getPtr().?.* = 456;
        try expectEqual(456, opt.value);
    }
    {
        var opt: Optional(?i32) = .to(null);
        try expectEqual(null, opt.getPtr());
    }
}

test "Optional.to" {
    const Foo = struct {
        foo: i32,
    };

    {
        const foo: Foo = .{ .foo = 123 };
        const opt: Optional(Foo) = .to(foo);

        const got = opt.get();

        try expect(@TypeOf(got) == ?Foo);
        try expect(got.?.foo == 123);
    }

    {
        const foo: ?Foo = .{ .foo = 123 };
        const opt: Optional(Foo) = .to(foo);

        const got = opt.get();

        try expect(@TypeOf(got) == ?Foo);
        try expect(got.?.foo == 123);
    }

    {
        // Support anonymous structs
        const opt: Optional(Foo) = Optional(Foo).to(.{ .foo = 123 });
        try expectEqual(123, opt.value.foo);
    }
}

test "Optional.fromPgzRow" {
    _ = Optional(i32).fromPgzRow(.{ .is_null = false, .data = "123" }, 0) catch {};
    const o = try Optional(?i32).fromPgzRow(.{ .is_null = true, .data = "" }, 0);
    try expect(o.value == null);
}

test "parse json Optional" {
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
            try expect(v == 123);
        } else try expect(false);
    }

    {
        const inner = foo.bar.get();
        if (inner) |_| {
            try expect(false);
        } else try expect(inner == null);
    }

    if (foo.foo.get()) |v| {
        try expect(v == 123);
    }

    try expect(foo.foo.value == 123);
    try expect(foo.bar.value == null);
    try expect(foo.baz == .not_provided);

    if (foo.foo.value) |val| {
        try expect(val == 123);
    } else try expect(false);

    if (foo.bar.value) |_| try expect(false);
}

test "stringify json Optional" {
    const stringify = @import("json.zig").stringify;
    const alloc = std.testing.allocator;

    {
        const opt: Optional(i32) = .{ .value = 1 };
        const str = try stringify(alloc, opt, .{});
        defer alloc.free(str);
        try expectEqualStrings(str, "1");
    }

    {
        const opt: Optional(i32) = .not_provided;
        const strOrError = stringify(alloc, opt, .{});
        try expectError(json.Stringify.Error.WriteFailed, strOrError);
    }

    const Foo = struct {
        data: Optional(i32),
    };

    {
        const foo: Foo = .{
            .data = .not_provided,
        };
        const strOrError = stringify(alloc, foo, .{});
        try expectError(json.Stringify.Error.WriteFailed, strOrError);
    }
    {
        const foo: Foo = .{
            .data = .{ .value = 123 },
        };
        const str = try stringify(alloc, foo, .{});
        defer alloc.free(str);
        try expectEqualStrings(str, "{\"data\":123}");
    }
}

pub fn JsonObject(comptime T: type) type {
    const info = @typeInfo(T);
    const S = switch (info) {
        .@"struct" => |S| S,
        else => @compileError("JsonObject must wrap a struct: " ++ @typeName(T)),
    };
    return struct {
        const Self = @This();

        obj: T,

        pub fn init(obj: T) Self {
            return .{ .obj = obj };
        }

        pub fn jsonStringify(self: *const Self, jws: anytype) !void {
            try jws.beginObject();
            inline for (S.fields) |Field| {
                if (Field.type == void) continue;

                var emit_field: bool = true;
                if (comptime isOptional(Field.type)) {
                    if (@field(self.obj, Field.name) == .not_provided) {
                        emit_field = false;
                    }
                }

                if (emit_field) {
                    try jws.objectField(Field.name);
                    const j: Json(Field.type) = .init(@field(self.obj, Field.name));
                    try jws.write(j);
                }
            }
            try jws.endObject();
        }
    };
}

test "stringify JsonObject" {
    const stringify = @import("json.zig").stringify;
    const alloc = std.testing.allocator;

    const Foo = JsonObject(struct {
        data: Optional(i32),
    });

    {
        const foo: Foo = .{
            .obj = .{ .data = .not_provided },
        };
        const str = try stringify(alloc, foo, .{});
        defer alloc.free(str);
        try expectEqualStrings(str, "{}");
    }
    {
        const foo: Foo = .{
            .obj = .{ .data = .{ .value = 123 } },
        };
        const str = try stringify(alloc, foo, .{});
        defer alloc.free(str);
        try expectEqualStrings(str, "{\"data\":123}");
    }
}

pub fn JsonArray(comptime T: type) type {
    return struct {
        const Self = @This();

        list: std.ArrayList(T) = .empty,

        pub fn deinit(self: *Self, alloc: std.mem.Allocator) void {
            self.list.deinit(alloc);
        }

        pub fn jsonParse(
            allocator: std.mem.Allocator,
            source: anytype,
            options: json.ParseOptions,
        ) json.ParseError(@TypeOf(source.*))!Self {
            const slice: []T = try json.innerParse([]T, allocator, source, options);
            return .{ .list = .fromOwnedSlice(slice) };
        }

        pub fn jsonParseFromValue(
            allocator: std.mem.Allocator,
            source: json.Value,
            options: json.ParseOptions,
        ) json.ParseError(@TypeOf(allocator))!Self {
            const slice: []T = try json.parseFromValueLeaky(T, allocator, source, options);
            return .{ .list = .fromOwnedSlice(slice) };
        }

        pub fn jsonStringify(self: *const Self, jws: anytype) !void {
            const j: JsonSlice(T) = .init(self.list.items);
            try jws.write(j);
        }
    };
}

test "parse json array" {
    const alloc = std.testing.allocator;

    const Foo = struct {
        data: JsonArray(i32),
    };

    const payload =
        \\ { "data": [1, 2, 3] }
    ;

    const parsed = try json.parseFromSlice(Foo, alloc, payload, .{});
    defer parsed.deinit();

    const foo = parsed.value;

    try std.testing.expect(foo.data.list.items[0] == 1);
    try std.testing.expect(foo.data.list.items[1] == 2);
    try std.testing.expect(foo.data.list.items[2] == 3);
}

test "stringify json array" {
    const stringify = @import("json.zig").stringify;
    const alloc = std.testing.allocator;

    const Foo = struct {
        data: JsonArray(i32),
    };

    var foo: Foo = .{
        .data = .{},
    };
    defer foo.data.list.deinit(alloc);
    try foo.data.list.append(alloc, 1);
    try foo.data.list.append(alloc, 2);
    try foo.data.list.append(alloc, 3);

    const str = try stringify(alloc, foo, .{});
    defer alloc.free(str);

    try expectEqualStrings(str, "{\"data\":[1,2,3]}");
}

pub fn JsonSlice(comptime T: type) type {
    return struct {
        const Self = @This();

        slice: []const T,

        pub fn init(slice: []const T) Self {
            return .{ .slice = slice };
        }

        pub fn jsonStringify(self: *const Self, jws: anytype) !void {
            try jws.beginArray();
            for (self.slice) |x| {
                const j: Json(T) = .init(x);
                try jws.write(j);
            }
            try jws.endArray();
        }
    };
}

pub fn JsonNullable(comptime T: type) type {
    return struct {
        const Self = @This();

        value: ?T,

        pub fn init(value: ?T) Self {
            return .{ .value = value };
        }

        pub fn jsonStringify(self: *const Self, jws: anytype) !void {
            if (self.value) |v| {
                const j: Json(T) = .init(v);
                try jws.write(j);
            } else {
                try jws.write(null);
            }
        }
    };
}

pub fn JsonUnion(comptime T: type) type {
    return struct {
        const Self = @This();

        value: T,

        pub fn init(value: T) Self {
            return .{ .value = value };
        }

        pub fn jsonStringify(self: *const Self, jws: anytype) !void {
            switch (self.value) {
                inline else => |v| {
                    const V = @TypeOf(v);
                    if (V != void) {
                        const j: Json(V) = .init(v);
                        try jws.write(j);
                    }
                },
            }
        }
    };
}

pub fn JsonPrimitive(comptime T: type) type {
    return struct {
        const Self = @This();

        value: T,

        pub fn init(value: T) Self {
            return .{ .value = value };
        }

        pub fn jsonStringify(self: *const Self, jws: anytype) !void {
            try jws.write(self.value);
        }
    };
}

pub fn Json(comptime T: type) type {
    const info = @typeInfo(T);
    switch (info) {
        .@"struct" => {
            if (!std.meta.hasFn(T, "jsonStringify")) return JsonObject(T);
        },
        .pointer => |p| {
            if (p.child != u8) return JsonSlice(p.child);
        },
        .optional => |O| return JsonNullable(O.child),
        .@"union" => return JsonUnion(T),
        .array => @compileError("array not supported"),
        .vector => @compileError("vector not supported"),
        else => {},
    }
    return JsonPrimitive(T);
}

test "Json(JsonArray(T))" {
    const stringify = @import("json.zig").stringify;
    const alloc = std.testing.allocator;

    const Foo = struct {
        data: JsonArray(i32),
    };

    var arr: JsonArray(i32) = .{};
    defer arr.deinit(alloc);

    try arr.list.append(alloc, 1);
    try arr.list.append(alloc, 2);
    try arr.list.append(alloc, 3);

    const foo: Json(Foo) = .init(.{ .data = arr });

    const str = try stringify(alloc, foo, .{});
    defer alloc.free(str);
    try expectEqualStrings(str, "{\"data\":[1,2,3]}");
}

/// Declarative presence constraints for a request `body` or `query_params` struct.
/// Attach as a decl inside the struct:
///
///   pub const constraints: Constraints = .{
///       .any_of = true,
///   };
///
/// Use tagged unions for distinct shapes, e.g. "a & b" OR "c & d".
pub const Constraints = struct {
    any_of: bool = false,
};

/// Returns whether a (possibly `Optional` wrapped) field carries a provided value.
/// Non-Optional fields are always considered present once parsing succeeds, since they are mandatory.
pub fn fieldPresent(value: anytype, comptime name: []const u8) bool {
    const F = @FieldType(@TypeOf(value), name);
    if (comptime isOptional(F)) {
        return @field(value, name) != .not_provided;
    }
    return true;
}

/// Validates the `constraints` of struct type `T` against a parsed value.
/// Returns a human readable error message for the first violated rule,
/// or `null` when all constraints are satisfied.
pub fn validateConstraints(comptime T: type, value: T) ?[]const u8 {
    // If it's not a struct, or if it is but doesn't have constraints, skip the checks.
    if (comptime @typeInfo(T) != .@"struct" or !@hasDecl(T, "constraints")) return null;

    const c: Constraints = T.constraints;
    if (comptime c.any_of) {
        var any_present = false;
        inline for (@typeInfo(T).@"struct".fields) |f| {
            if (fieldPresent(value, f.name)) {
                any_present = true;
                break;
            }
        }
        if (!any_present) return "At least one field must be provided";
    }

    return null;
}

test "validateConstraints any_of" {
    const Body = struct {
        a: Optional(i32) = .not_provided,
        b: Optional(?i32) = .not_provided,

        pub const constraints: Constraints = .{ .any_of = true };
    };

    var error_msg: ?[]const u8 = null;

    error_msg = validateConstraints(Body, .{ .a = .{ .value = 1 } });
    try expectEqual(null, error_msg);

    error_msg = validateConstraints(Body, .{ .b = .{ .value = null } });
    try expectEqual(null, error_msg);

    error_msg = validateConstraints(Body, .{});
    try std.testing.expect(error_msg != null);
    try expectEqualStrings("At least one field must be provided", error_msg.?);
}
