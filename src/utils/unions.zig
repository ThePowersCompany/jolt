const std = @import("std");
const Allocator = std.mem.Allocator;
const ParseOptions = std.json.ParseOptions;
const Value = std.json.Value;
const ObjectMap = std.json.ObjectMap;

/// Various ways to represent tagged unions in JSON
/// Reference (Rust): https://serde.rs/enum-representations.html
pub const UnionRepr = union(enum) {
    external,
    internal: struct {
        discriminator: []const u8,
    },
    adjacently: struct {
        discriminator: []const u8,
    },
    untagged,
};

/// Drop-in helper for a tagged union's `jsonParse`.
///
///  pub fn jsonParse(alloc: Allocator, source: anytype, opts: std.json.ParseOptions) !@This() {
///    return try jsonParseUnion(@This(), alloc, source, opts, _repr);
///  }
///
pub fn jsonParseUnion(
    comptime T: type,
    allocator: Allocator,
    source: anytype,
    options: ParseOptions,
    comptime repr: UnionRepr,
) std.json.ParseError(@TypeOf(source.*))!T {
    comptime {
        const info = @typeInfo(T);
        if (info != .@"union")
            @compileError("jsonParseUnion requires a union type, got " ++ @typeName(T));

        if (info.@"union".tag_type == null)
            @compileError("jsonParseUnion requires a tagged union, got " ++ @typeName(T));
    }

    const value = try Value.jsonParse(allocator, source, options);
    return switch (repr) {
        .external => parseExternal(T, allocator, value, options),
        .internal => |i| parseInternal(T, allocator, value, options, i.discriminator),
        // The adjacent discriminator is consumed by the parent object,
        // so from here it is indistinguishable from untagged.
        .adjacently, .untagged => parseByInference(T, allocator, value, options),
    };
}

/// `{ "Variant": <payload> }`
fn parseExternal(
    comptime T: type,
    allocator: Allocator,
    value: Value,
    options: ParseOptions,
) std.json.ParseFromValueError!T {
    const info = @typeInfo(T).@"union";
    if (value != .object) return error.UnexpectedToken;
    if (value.object.count() != 1) return error.UnexpectedToken;

    var it = value.object.iterator();
    const kv = it.next().?;
    const name = kv.key_ptr.*;

    inline for (info.fields) |field| {
        if (std.mem.eql(u8, field.name, name)) {
            const payload = try parsePayload(field.type, allocator, kv.value_ptr.*, options);
            return @unionInit(T, field.name, payload);
        }
    }
    return error.UnknownField;
}

/// Parse a single variant payload
fn parsePayload(
    comptime P: type,
    allocator: Allocator,
    value: Value,
    options: ParseOptions,
) std.json.ParseFromValueError!P {
    if (P == void) {
        if (value == .object and value.object.count() == 0) return {};
        return error.UnexpectedToken;
    }
    return std.json.innerParseFromValue(P, allocator, value, options);
}

/// `{ "<discriminator>": "Variant", ...payload fields }`
fn parseInternal(
    comptime T: type,
    allocator: Allocator,
    value: Value,
    options: ParseOptions,
    discriminator: []const u8,
) std.json.ParseFromValueError!T {
    if (value != .object) return error.UnexpectedToken;

    const tag_value = value.object.get(discriminator) orelse return error.MissingField;
    const name = switch (tag_value) {
        .string => |s| s,
        else => return error.UnexpectedToken,
    };

    inline for (@typeInfo(T).@"union".fields) |field| {
        if (std.mem.eql(u8, field.name, name)) {
            if (field.type == void) {
                // Void variant: the object should hold nothing but the discriminator.
                if (!options.ignore_unknown_fields and value.object.count() != 1) {
                    return error.UnknownField;
                }
                return @unionInit(T, field.name, {});
            }
            // Recreate the object without the discriminator key,
            // then parse the remainder as the variant's payload struct.
            const obj_map = try cloneObjectExcluding(allocator, value.object, discriminator);
            const val = try std.json.innerParseFromValue(
                field.type,
                allocator,
                Value{ .object = obj_map },
                options,
            );
            return @unionInit(T, field.name, val);
        }
    }
    return error.UnknownField;
}

/// Shallow copy of `obj` with `exclude` removed.
/// Keys/values are shared with the original (both live in the same arena during a normal parse).
fn cloneObjectExcluding(
    allocator: Allocator,
    obj: ObjectMap,
    exclude: []const u8,
) Allocator.Error!ObjectMap {
    var out = ObjectMap.init(allocator);
    var it = obj.iterator();
    while (it.next()) |kv| {
        if (std.mem.eql(u8, kv.key_ptr.*, exclude)) continue;
        try out.put(kv.key_ptr.*, kv.value_ptr.*);
    }
    return out;
}

/// Attempt to parse each variant in declaration order and keep the first that fully matches.
/// Unknown fields are rejected during inference
/// (regardless of the caller's `ignore_unknown_fields`)
/// so that an object with extra keys does not incorrectly match a narrower variant.
fn parseByInference(
    comptime T: type,
    allocator: Allocator,
    value: Value,
    options: ParseOptions,
) std.json.ParseFromValueError!T {
    // Don't skip unknown fields
    var strict = options;
    strict.ignore_unknown_fields = false;

    inline for (@typeInfo(T).@"union".fields) |field| {
        if (field.type == void) {
            if (value == .object and value.object.count() == 0) {
                return @unionInit(T, field.name, {});
            }
        } else {
            if (std.json.innerParseFromValue(field.type, allocator, value, strict)) |payload| {
                return @unionInit(T, field.name, payload);
            } else |_| {
                // Failed to parse into this variant, move onto the next.
            }
        }
    }
    return error.UnknownField;
}

const testing = std.testing;

// External

const External = union(enum) {
    ping,
    text: []const u8,
    point: struct { x: i32, y: i32 },

    const _repr: UnionRepr = .external;

    pub fn jsonParse(alloc: Allocator, source: anytype, opts: ParseOptions) !@This() {
        return try jsonParseUnion(@This(), alloc, source, opts, _repr);
    }
};

test "external: struct payload" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const v = try std.json.parseFromSliceLeaky(
        External,
        arena.allocator(),
        \\{
        \\  "point": { "x": 1, "y": 2 }
        \\}
    ,
        .{},
    );
    try testing.expect(v == .point);
    try testing.expectEqual(1, v.point.x);
    try testing.expectEqual(2, v.point.y);
}

test "external: string payload" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const v = try std.json.parseFromSliceLeaky(
        External,
        arena.allocator(),
        \\{
        \\  "text": "hi"
        \\}
    ,
        .{},
    );
    try testing.expect(v == .text);
    try testing.expectEqualStrings("hi", v.text);
}

test "external: void payload" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const v = try std.json.parseFromSliceLeaky(
        External,
        arena.allocator(),
        \\{
        \\  "ping": {}
        \\}
    ,
        .{},
    );
    try testing.expect(v == .ping);
}

test "external: unknown variant errors" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    try testing.expectError(
        error.UnknownField,
        std.json.parseFromSliceLeaky(
            External,
            arena.allocator(),
            \\{
            \\  "nope": 1
            \\}
        ,
            .{},
        ),
    );
}

// Internal

const Internal = union(enum) {
    request: struct { id: []const u8, method: []const u8 },
    response: struct { id: []const u8, result: i64 },
    ping,

    const _repr: UnionRepr = .{ .internal = .{ .discriminator = "type" } };

    pub fn jsonParse(alloc: Allocator, source: anytype, opts: ParseOptions) !@This() {
        return try jsonParseUnion(@This(), alloc, source, opts, _repr);
    }
};

test "internal: request variant" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const v = try std.json.parseFromSliceLeaky(
        Internal,
        arena.allocator(),
        \\{
        \\  "type": "request",
        \\  "id": "abc123",
        \\  "method": "GET"
        \\}
    ,
        .{},
    );
    try testing.expect(v == .request);
    try testing.expectEqualStrings("abc123", v.request.id);
    try testing.expectEqualStrings("GET", v.request.method);
}

test "internal: response variant" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const v = try std.json.parseFromSliceLeaky(
        Internal,
        arena.allocator(),
        \\{
        \\  "type": "response",
        \\  "id": "abc123",
        \\  "result": 32
        \\}
    ,
        .{},
    );
    try testing.expect(v == .response);
    try testing.expectEqual(32, v.response.result);
}

test "internal: void variant" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const v = try std.json.parseFromSliceLeaky(
        Internal,
        arena.allocator(),
        \\{
        \\  "type": "ping"
        \\}
    ,
        .{},
    );
    try testing.expect(v == .ping);
}

test "internal: missing discriminator errors" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    try testing.expectError(
        error.MissingField,
        std.json.parseFromSliceLeaky(
            Internal,
            arena.allocator(),
            \\{
            \\  "id": "abc123",
            \\  "method": "GET"
            \\}
        ,
            .{},
        ),
    );
}

// Adjacently

const AlertTopic = enum { downtime, lost_production };

const AlertPayload = union(AlertTopic) {
    downtime: struct { line: []const u8, minutes: f32 },
    lost_production: struct { line: []const u8, units: f64 },

    const _repr: UnionRepr = .{ .adjacently = .{ .discriminator = "topic" } };

    pub fn jsonParse(alloc: Allocator, source: anytype, opts: ParseOptions) !@This() {
        return try jsonParseUnion(@This(), alloc, source, opts, _repr);
    }
};

const Alert = struct {
    id: i64,
    topic: AlertTopic,
    payload: AlertPayload,
};

test "adjacently: payload inferred from downtime" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const json =
        \\{
        \\  "id": 1,
        \\  "topic": "downtime",
        \\  "payload": { "line": "A", "minutes": 5.0 }
        \\}
    ;
    const alert = try std.json.parseFromSliceLeaky(Alert, arena.allocator(), json, .{});
    try testing.expectEqual(1, alert.id);
    try testing.expectEqual(AlertTopic.downtime, alert.topic);
    try testing.expect(alert.payload == .downtime);
    try testing.expectEqualStrings("A", alert.payload.downtime.line);
    try testing.expectEqual(5.0, alert.payload.downtime.minutes);
}

test "adjacently: payload inferred from lost_production" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const json =
        \\{
        \\  "id": 2,
        \\  "topic": "lost_production",
        \\  "payload": { "line": "B", "units": 12.5 }
        \\}
    ;
    const alert = try std.json.parseFromSliceLeaky(Alert, arena.allocator(), json, .{});
    try testing.expectEqual(2, alert.id);
    try testing.expect(alert.payload == .lost_production);
    try testing.expectEqualStrings("B", alert.payload.lost_production.line);
    try testing.expectEqual(12.5, alert.payload.lost_production.units);
}

// Untagged

const Untagged = union(enum) {
    number: i64,
    pair: struct { a: i64, b: i64 },
    named: struct { name: []const u8 },

    const _repr: UnionRepr = .untagged;

    pub fn jsonParse(alloc: Allocator, source: anytype, opts: ParseOptions) !@This() {
        return try jsonParseUnion(@This(), alloc, source, opts, _repr);
    }
};

test "untagged: scalar variant" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const v = try std.json.parseFromSliceLeaky(Untagged, arena.allocator(), "7", .{});
    try testing.expect(v == .number);
    try testing.expectEqual(7, v.number);
}

test "untagged: pair struct variant inferred by shape" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const v = try std.json.parseFromSliceLeaky(
        Untagged,
        arena.allocator(),
        \\{
        \\  "a": 1,
        \\  "b": 2
        \\}
    ,
        .{},
    );
    try testing.expect(v == .pair);
    try testing.expectEqual(2, v.pair.b);
}

test "untagged: named struct variant inferred by shape" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const v = try std.json.parseFromSliceLeaky(
        Untagged,
        arena.allocator(),
        \\{
        \\  "name": "zig"
        \\}
    ,
        .{},
    );
    try testing.expect(v == .named);
    try testing.expectEqualStrings("zig", v.named.name);
}

test "untagged: no matching variant errors" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    try testing.expectError(
        error.UnknownField,
        std.json.parseFromSliceLeaky(
            Untagged,
            arena.allocator(),
            \\{
            \\  "x": 1,
            \\  "y": 2,
            \\  "z": 3
            \\}
        ,
            .{},
        ),
    );
}

// Proves inference uses strict (ignore_unknown_fields = false) matching.
// The first variant is an all-optional struct
// that would greedily match any object under lenient parsing,
// but populated data must still parse into the correct variant found later in the union.
const OptionalFirst = union(enum) {
    maybe: struct { note: ?[]const u8 = null },
    coords: struct { x: i64, y: i64 },

    const _repr: UnionRepr = .untagged;

    pub fn jsonParse(alloc: Allocator, source: anytype, opts: ParseOptions) !@This() {
        return try jsonParseUnion(@This(), alloc, source, opts, _repr);
    }
};

test "untagged: optional-only first variant does not swallow a later variant" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    // { "x": 1, "y": 2 } has unknown keys for `maybe`,
    // so strict matching rejects it and inference falls through to `coords`.
    const v = try std.json.parseFromSliceLeaky(
        OptionalFirst,
        arena.allocator(),
        \\{
        \\  "x": 1,
        \\  "y": 2
        \\}
    ,
        .{},
    );
    try testing.expect(v == .coords);
    try testing.expectEqual(1, v.coords.x);
    try testing.expectEqual(2, v.coords.y);
}

test "untagged: optional-only first variant still matches its own shape" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const v = try std.json.parseFromSliceLeaky(
        OptionalFirst,
        arena.allocator(),
        \\{
        \\  "note": "hello"
        \\}
    ,
        .{},
    );
    try testing.expect(v == .maybe);
    try testing.expect(v.maybe.note != null);
    try testing.expectEqualStrings("hello", v.maybe.note.?);
}
