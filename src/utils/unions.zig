const std = @import("std");
const Allocator = std.mem.Allocator;
const ParseOptions = std.json.ParseOptions;
const Value = std.json.Value;
const ObjectMap = std.json.ObjectMap;
const Type = std.builtin.Type;

const types_mod = @import("../utils/types.zig");
const Optional = types_mod.Optional;
const isOptional = types_mod.isOptional;

const containers_module = @import("./containers.zig");
const hasParamParse = containers_module.hasParamParse;
const getRequiredKeyCount = containers_module.getRequiredKeyCount;

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

/// Drop-in helpers for a tagged union's custom json parsing.
///
/// `std.json` has two parse paths that dispatch to different methods:
/// parsing from a slice looks for `jsonParse`, while parsing a nested value looks for `jsonParseFromValue`.
/// A union that only defines `jsonParse` is parsed with std's default (external) logic
/// whenever it is nested inside another value, silently ignoring its `_repr`.
/// Define both so the repr is honored at the top level and when nested:
///
///  pub fn jsonParse(alloc: Allocator, source: anytype, opts: std.json.ParseOptions) !@This() {
///    return try jsonParseUnion(@This(), alloc, source, opts, _repr);
///  }
///  pub fn jsonParseFromValue(alloc: Allocator, source: std.json.Value, opts: std.json.ParseOptions) !@This() {
///    return try jsonParseUnionFromValue(@This(), alloc, source, opts, _repr);
///  }
///
pub fn jsonParseUnion(
    comptime T: type,
    allocator: Allocator,
    source: anytype,
    options: ParseOptions,
    comptime repr: UnionRepr,
) std.json.ParseError(@TypeOf(source.*))!T {
    comptime validateUnionRepr(T, repr);

    // Simple inferred unions (scalar/void variants) are resolved by the first token's JSON type,
    // so it does not need to buffer into a Value and does not allocate.
    switch (repr) {
        .untagged, .adjacently => if (comptime isSimpleInferableUnion(T)) {
            return parseInferredFromSource(T, allocator, source, options);
        },
        .external, .internal => {},
    }

    // Buffer the token source into a Value, then share the from-value dispatch below.
    const value = try Value.jsonParse(allocator, source, options);
    return dispatchByRepr(T, allocator, value, options, repr);
}

/// From-value counterpart of `jsonParseUnion`,
/// used by std.json when the union is nested inside another value (and internally by this helper).
/// Wire it up as `jsonParseFromValue`.
pub fn jsonParseUnionFromValue(
    comptime T: type,
    allocator: Allocator,
    source: Value,
    options: ParseOptions,
    comptime repr: UnionRepr,
) std.json.ParseFromValueError!T {
    comptime validateUnionRepr(T, repr);
    return dispatchByRepr(T, allocator, source, options, repr);
}

fn validateUnionRepr(comptime T: type, comptime repr: UnionRepr) void {
    comptime {
        const info = @typeInfo(T);
        if (info != .@"union")
            @compileError("jsonParseUnion requires a union type, got " ++ @typeName(T));

        if (info.@"union".tag_type == null)
            @compileError("jsonParseUnion requires a tagged union, got " ++ @typeName(T));

        // Inferred reprs resolve the variant purely from the value's shape,
        // so more than one zero-key fallback would be a silent, order-dependent ambiguity.
        switch (repr) {
            .adjacently, .untagged => assertAtMostOneFallbackVariant(T),
            .external, .internal => {},
        }
    }
}

fn dispatchByRepr(
    comptime T: type,
    allocator: Allocator,
    value: Value,
    options: ParseOptions,
    comptime repr: UnionRepr,
) std.json.ParseFromValueError!T {
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

pub const OrderedFieldsOptions = struct {
    /// When true, `void` variants count as zero required keys instead of being rejected.
    /// JSON inference needs this: a void variant is matched by its name string and sorts last.
    /// query-param parsing leaves it false so `void` variants remain a hard error.
    allow_void_fields: bool = false,
};

/// The fields of union `T`, ordered most specific first:
/// a field requiring more keys comes before one requiring fewer,
/// so the all-optional fallback (zero required keys) is always tried last.
/// The order is fixed at compile time and independent of the request,
/// so callers just walk it and build the first variant whose keys are present.
///
/// The sort is stable, so two fields requiring the same number of keys keep their declaration order.
/// Such a pair can only both match when the request mixes keys from both,
/// which is rejected later as unexpected query params,
/// so their relative order does not change the outcome of a valid request.
pub fn orderedFields(
    comptime T: type,
    comptime opts: OrderedFieldsOptions,
) [@typeInfo(T).@"union".fields.len]Type.UnionField {
    comptime {
        const Entry = struct {
            field: *const Type.UnionField,
            required_keys: usize,

            fn moreSpecific(_: void, a: @This(), b: @This()) bool {
                return a.required_keys > b.required_keys;
            }
        };

        const fields = @typeInfo(T).@"union".fields;
        var entries: [fields.len]Entry = undefined;
        for (fields, 0..) |f, i| {
            const count = if (opts.allow_void_fields and f.type == void) 0 else getRequiredKeyCount(f.type);
            entries[i] = .{
                .field = &f,
                .required_keys = count,
            };
        }

        std.sort.insertion(Entry, &entries, {}, Entry.moreSpecific);

        var sorted: [fields.len]Type.UnionField = undefined;
        for (entries, 0..) |e, i| sorted[i] = e.field.*;
        return sorted;
    }
}

/// Inferred unions allow at most one all-optional (zero required key) variant,
/// since two would both match `{}` and the winner would depend on declaration order.
///
/// Void variants are exempt: they are matched by their name string (e.g. `"auto"`).
fn assertAtMostOneFallbackVariant(comptime T: type) void {
    comptime {
        var fallback: ?[]const u8 = null;
        for (@typeInfo(T).@"union".fields) |f| {
            if (f.type == void) continue;
            if (getRequiredKeyCount(f.type) != 0) continue;
            if (fallback) |first| {
                @compileError(@typeName(T) ++ ": variants '" ++ first ++ "' and '" ++ f.name ++
                    "' are both all-optional, so the union is ambiguous.");
            }
            fallback = f.name;
        }
    }
}

/// Attempt to parse each variant and keep the first that fully matches,
/// trying the most specific variants first (see `orderedFields`).
/// Unknown fields are rejected during inference (regardless of the caller's `ignore_unknown_fields`)
/// so that an object with extra keys does not match a narrower variant.
///
/// Trying variants most-specific-first is what prevents a variant whose fields are all optional/defaulted
/// from greedily swallowing an object intended for a more specific variant.
/// The specific variant is attempted and succeeds before the fallback is ever reached.
fn parseByInference(
    comptime T: type,
    allocator: Allocator,
    value: Value,
    options: ParseOptions,
) std.json.ParseFromValueError!T {
    // Don't skip unknown fields
    var strict = options;
    strict.ignore_unknown_fields = false;

    inline for (comptime orderedFields(T, .{ .allow_void_fields = true })) |field| {
        if (field.type == void) {
            // A void variant is represented as the JSON string of its name, e.g. `"auto"` -> `.auto`
            if (value == .string and std.mem.eql(u8, value.string, field.name)) {
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

/// A union is "simple" when every variant is either `void` or a single-token scalar (numeric or bool).
/// Such unions can be inferred straight from the token stream.
/// The first token's JSON type uniquely selects a variant, so no Value buffering or retry is needed.
///
/// Because a consumed token cannot be replayed,
/// each JSON scalar type must map to at most one variant (at most one numeric and one bool variant).
fn isSimpleInferableUnion(comptime T: type) bool {
    comptime {
        var numeric_count: usize = 0;
        var bool_count: usize = 0;
        for (@typeInfo(T).@"union".fields) |f| {
            if (f.type == void) continue;
            switch (@typeInfo(f.type)) {
                .int, .comptime_int, .float, .comptime_float => numeric_count += 1,
                .bool => bool_count += 1,
                // Strings, structs, optionals, nested unions, etc. need shape-based inference.
                else => return false,
            }
        }
        return numeric_count <= 1 and bool_count <= 1;
    }
}

/// Allocation-free inference for a `isSimpleInferableUnion` union, reading directly from the token source.
/// A `void` variant is matched by its name string,
/// and a numeric/bool variant is matched by the corresponding JSON scalar token.
fn parseInferredFromSource(
    comptime T: type,
    allocator: Allocator,
    source: anytype,
    options: ParseOptions,
) std.json.ParseError(@TypeOf(source.*))!T {
    const fields = @typeInfo(T).@"union".fields;
    switch (try source.peekNextTokenType()) {
        .number => {
            inline for (fields) |f| {
                if (comptime isNumeric(f.type)) {
                    const payload = std.json.innerParse(f.type, allocator, source, options) catch
                        return error.UnknownField;
                    return @unionInit(T, f.name, payload);
                }
            }
            return error.UnknownField;
        },
        .true, .false => {
            inline for (fields) |f| {
                if (comptime f.type == bool) {
                    const payload = std.json.innerParse(f.type, allocator, source, options) catch
                        return error.UnknownField;
                    return @unionInit(T, f.name, payload);
                }
            }
            return error.UnknownField;
        },
        .string => {
            // A string can only match a void variant by name
            if (comptime !hasVoidField(T)) return error.UnknownField;

            const max_len = options.max_value_len orelse std.json.default_max_value_len;
            const token: std.json.Token = try source.nextAllocMax(allocator, .alloc_if_needed, max_len);
            defer if (token == .allocated_string) allocator.free(token.allocated_string);

            const slice = switch (token) {
                inline .string, .allocated_string => |s| s,
                else => return error.UnknownField,
            };

            inline for (fields) |f| {
                if (comptime f.type == void) {
                    if (std.mem.eql(u8, slice, f.name)) return @unionInit(T, f.name, {});
                }
            }
            return error.UnknownField;
        },
        else => return error.UnknownField,
    }
}

fn isNumeric(comptime P: type) bool {
    return switch (@typeInfo(P)) {
        .int, .comptime_int, .float, .comptime_float => true,
        else => false,
    };
}

fn hasVoidField(comptime T: type) bool {
    comptime {
        for (@typeInfo(T).@"union".fields) |f| {
            if (f.type == void) return true;
        }
        return false;
    }
}

/// Whether a union carries a `_repr` declaration,
/// i.e. it requires tagged serialization (external, internal, or adjacently tagged).
/// Such unions cannot be used in query params, which have no structure for a discriminator.
/// Untagged unions (`.untagged` mode) are allowed since they infer the variant from the value.
pub fn hasTaggedRepr(comptime T: type) bool {
    comptime {
        if (@typeInfo(T) != .@"union" or !@hasDecl(T, "_repr")) return false;
        const repr = T._repr;
        return repr != .untagged;
    }
}

/// Returns if `T` is a tagged union that should be lifted into the flat query key space.
/// This ignores the `Optional` wrapper,
/// excludes unions with a `paramParse` function (which are parsed as single leaf keys),
/// and excludes unions with a discriminator-requiring `_repr` (external/internal/adjacently tagged).
/// Untagged unions are allowed since they don't require a discriminator field.
pub fn isLiftableUnion(comptime T: type) bool {
    return comptime @typeInfo(T) == .@"union" and !isOptional(T) and !hasParamParse(T) and !hasTaggedRepr(T);
}

const testing = std.testing;
const no_alloc = @import("../utils/testing.zig").no_alloc;

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

test "external: non-object input errors" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    try testing.expectError(
        error.UnexpectedToken,
        std.json.parseFromSliceLeaky(External, arena.allocator(), "42", .{}),
    );
}

test "external: object with multiple keys errors" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    // More than one key is ambiguous about which variant is meant, so it is rejected.
    try testing.expectError(
        error.UnexpectedToken,
        std.json.parseFromSliceLeaky(
            External,
            arena.allocator(),
            \\{ "text": "hi", "ping": {} }
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

test "internal: unknown discriminator value errors" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    try testing.expectError(
        error.UnknownField,
        std.json.parseFromSliceLeaky(
            Internal,
            arena.allocator(),
            \\{ "type": "nope", "id": "x", "method": "GET" }
        ,
            .{},
        ),
    );
}

test "internal: non-string discriminator errors" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    try testing.expectError(
        error.UnexpectedToken,
        std.json.parseFromSliceLeaky(
            Internal,
            arena.allocator(),
            \\{ "type": 5 }
        ,
            .{},
        ),
    );
}

test "internal: void variant rejects extra fields" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    try testing.expectError(
        error.UnknownField,
        std.json.parseFromSliceLeaky(
            Internal,
            arena.allocator(),
            \\{ "type": "ping", "extra": 1 }
        ,
            .{},
        ),
    );
}

test "internal: void variant tolerates extra fields when ignore_unknown_fields" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const v = try std.json.parseFromSliceLeaky(
        Internal,
        arena.allocator(),
        \\{ "type": "ping", "extra": 1 }
    ,
        .{ .ignore_unknown_fields = true },
    );
    try testing.expect(v == .ping);
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
    const json =
        \\{
        \\  "id": 1,
        \\  "topic": "downtime",
        \\  "payload": { "line": "A", "minutes": 5.0 }
        \\}
    ;
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alert = try std.json.parseFromSliceLeaky(Alert, arena.allocator(), json, .{});
    try testing.expectEqual(1, alert.id);
    try testing.expectEqual(AlertTopic.downtime, alert.topic);
    try testing.expect(alert.payload == .downtime);
    try testing.expectEqualStrings("A", alert.payload.downtime.line);
    try testing.expectEqual(5.0, alert.payload.downtime.minutes);
}

test "adjacently: payload inferred from lost_production" {
    const json =
        \\{
        \\  "id": 2,
        \\  "topic": "lost_production",
        \\  "payload": { "line": "B", "units": 12.5 }
        \\}
    ;
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
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
    // { "x": 1, "y": 2 } has unknown keys for `maybe`,
    // so strict matching rejects it and inference falls through to `coords`.
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
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

/// Simple, mutually-exclusive, scalar type
/// This type should be parsable without allocation.
const IdOrAuto = union(enum) {
    id: i32,
    auto,

    pub const _repr: UnionRepr = .untagged;

    pub fn jsonParse(alloc: Allocator, source: anytype, opts: std.json.ParseOptions) !@This() {
        return try jsonParseUnion(@This(), alloc, source, opts, _repr);
    }
};

test "untagged: IdOrAuto - should not allocate memory for i32" {
    const v = try std.json.parseFromSliceLeaky(IdOrAuto, no_alloc, "123", .{});
    try testing.expect(v == .id);
    try testing.expectEqual(123, v.id);
}

test "untagged: IdOrAuto - should not allocate memory for 'auto'" {
    const v = try std.json.parseFromSliceLeaky(IdOrAuto, no_alloc, "\"auto\"", .{});
    try testing.expect(v == .auto);
}

test "untagged: IdOrAuto - should not allocate memory for invalid number" {
    try testing.expectError(
        error.UnknownField,
        std.json.parseFromSliceLeaky(IdOrAuto, no_alloc, "123456789000", .{}),
    );
}

test "untagged: IdOrAuto - should not allocate memory for invalid string" {
    try testing.expectError(
        error.UnknownField,
        std.json.parseFromSliceLeaky(IdOrAuto, no_alloc, "\"abc\"", .{}),
    );
}

test "untagged: IdOrAuto - should not allocate memory for invalid type" {
    try testing.expectError(
        error.UnknownField,
        std.json.parseFromSliceLeaky(IdOrAuto, no_alloc, "{}", .{}),
    );
}

/// Simple union-enum type (equivalent to normal enum)
/// This type should be parsable without allocation.
const TestAction = union(enum) {
    find,
    replace,
    trim,

    pub const _repr: UnionRepr = .untagged;

    pub fn jsonParse(alloc: Allocator, source: anytype, opts: std.json.ParseOptions) !@This() {
        return try jsonParseUnion(@This(), alloc, source, opts, _repr);
    }
};

test "untagged: TestAction - should not allocate memory for 'replace" {
    const v = try std.json.parseFromSliceLeaky(TestAction, no_alloc, "\"replace\"", .{});
    try testing.expect(v == .replace);
}

test "untagged: TestAction - should not allocate memory for invalid string" {
    try testing.expectError(
        error.UnknownField,
        std.json.parseFromSliceLeaky(TestAction, no_alloc, "\"auto\"", .{}),
    );
}

test "untagged: TestAction - should fail for number" {
    try testing.expectError(
        error.UnknownField,
        std.json.parseFromSliceLeaky(TestAction, no_alloc, "123", .{}),
    );
}

// One numeric, bool, and void variants: each JSON scalar type maps to exactly one variant,
// so this uses `isSimpleInferableUnion` and parses straight from the token stream without allocation.
const Val = union(enum) {
    count: i64,
    enabled: bool,
    auto,

    pub const _repr: UnionRepr = .untagged;

    pub fn jsonParse(alloc: Allocator, source: anytype, opts: std.json.ParseOptions) !@This() {
        return try jsonParseUnion(@This(), alloc, source, opts, _repr);
    }
};

test "untagged: Val - should not allocate for a number token" {
    const v = try std.json.parseFromSliceLeaky(Val, no_alloc, "42", .{});
    try testing.expect(v == .count);
    try testing.expectEqual(42, v.count);
}

test "untagged: Val - should not allocate for a bool token" {
    const v = try std.json.parseFromSliceLeaky(Val, no_alloc, "true", .{});
    try testing.expect(v == .enabled);
    try testing.expectEqual(true, v.enabled);
}

test "untagged: Val - should not allocate for a string token" {
    const v = try std.json.parseFromSliceLeaky(Val, no_alloc, "\"auto\"", .{});
    try testing.expect(v == .auto);
}

// Nested custom union: `Inner` uses an internal-tagged repr and is a field of another union's payload struct.
// When the outer union is reached through this helper,
// std.json parses `Inner` from a Value by calling `jsonParseFromValue`.
// A union that only defines `jsonParse` would be parsed with std's default (external) union logic here,
// silently ignoring its `_repr`.
const Inner = union(enum) {
    a: struct { x: i64 },
    b: struct { y: i64 },

    const _repr: UnionRepr = .{ .internal = .{ .discriminator = "kind" } };

    pub fn jsonParse(alloc: Allocator, source: anytype, opts: ParseOptions) !@This() {
        return try jsonParseUnion(@This(), alloc, source, opts, _repr);
    }
    pub fn jsonParseFromValue(alloc: Allocator, source: Value, opts: ParseOptions) !@This() {
        return try jsonParseUnionFromValue(@This(), alloc, source, opts, _repr);
    }
};

const Outer = union(enum) {
    wrap: struct { inner: Inner },
    other: struct { z: i64 },

    const _repr: UnionRepr = .untagged;

    pub fn jsonParse(alloc: Allocator, source: anytype, opts: ParseOptions) !@This() {
        return try jsonParseUnion(@This(), alloc, source, opts, _repr);
    }
    pub fn jsonParseFromValue(alloc: Allocator, source: Value, opts: ParseOptions) !@This() {
        return try jsonParseUnionFromValue(@This(), alloc, source, opts, _repr);
    }
};

test "nested: internal-tagged union nested inside an inferred union honors its repr" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const v = try std.json.parseFromSliceLeaky(
        Outer,
        arena.allocator(),
        \\{
        \\  "inner": { "kind": "b", "y": 9 }
        \\}
    ,
        .{},
    );
    try testing.expect(v == .wrap);
    try testing.expect(v.wrap.inner == .b);
    try testing.expectEqual(9, v.wrap.inner.b.y);
}

// Untagged union mixing a void variant with a struct variant,
// which exercises the void name matching inside `parseByInference`
const AutoOrPoint = union(enum) {
    point: struct { x: i64, y: i64 },
    auto,

    const _repr: UnionRepr = .untagged;

    pub fn jsonParse(alloc: Allocator, source: anytype, opts: ParseOptions) !@This() {
        return try jsonParseUnion(@This(), alloc, source, opts, _repr);
    }
};

test "untagged: void variant matched by name string" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const v = try std.json.parseFromSliceLeaky(AutoOrPoint, arena.allocator(), "\"auto\"", .{});
    try testing.expect(v == .auto);
}

test "untagged: struct variant inferred by shape" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const v = try std.json.parseFromSliceLeaky(
        AutoOrPoint,
        arena.allocator(),
        \\{ "x": 1, "y": 2 }
    ,
        .{},
    );
    try testing.expect(v == .point);
    try testing.expectEqual(1, v.point.x);
    try testing.expectEqual(2, v.point.y);
}

test "untagged: empty object does not match the void variant" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    try testing.expectError(
        error.UnknownField,
        std.json.parseFromSliceLeaky(AutoOrPoint, arena.allocator(), "{}", .{}),
    );
}

test "untagged: wrong string does not match the void variant" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    try testing.expectError(
        error.UnknownField,
        std.json.parseFromSliceLeaky(AutoOrPoint, arena.allocator(), "\"nope\"", .{}),
    );
}
