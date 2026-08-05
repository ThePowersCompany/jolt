const std = @import("std");
const Allocator = std.mem.Allocator;
const Type = std.builtin.Type;

const types_module = @import("./types.zig");
const Optional = types_module.Optional;
const isOptional = types_module.isOptional;
const Unwrap = types_module.Unwrap;

/// How a container type (a struct or union) is resolved from query params.
///
/// - composite: flattened or key-selected into its own leaf keys.
///     A plain struct (no `paramParse`) is flattened into its fields,
///     and a union with at least one composite variant is key-selected
///     from which variant keys are present (`parseCompositeUnion`),
///     so variant declaration order is irrelevant.
/// - strong_scalar: parsed from a single query value, with an explicit absent form,
///     namely a union carrying a `void` variant.
/// - weak_scalar: parsed from a single query value, namely a `paramParse` type,
///     a union with no `void` variant, or any leaf type (int, enum, pointer, etc).
pub const ContainerKind = enum {
    composite, // can only ever be used as a composite
    strong_scalar, // can only ever be used as a scalar
    weak_scalar, // can be used as scalar or composite depending on if there is a parent key name
};

pub fn containerKind(comptime T: type) ContainerKind {
    comptime {
        // A type with a custom `paramParse` is always parsed from a single value.
        if (std.meta.hasFn(T, "paramParse")) return .strong_scalar;

        return switch (@typeInfo(T)) {
            .void => @compileError("Void is nothing"),
            .optional => |o| containerKind(o.child),
            // A plain struct is flattened into its leaf keys.
            .@"struct" => .composite,
            .@"union" => |u| {
                if (isOptional(T)) {
                    return containerKind(T.childType());
                }

                // Composite when any variant is itself composite.
                const is_comp: bool = for (u.fields) |f| {
                    // A `void` variant carries no keys, so it is never composite.
                    // The `is_strong` check below is what detects it.
                    if (f.type == void) continue;
                    if (containerKind(f.type) == .composite) break true;
                } else false;
                // Otherwise a scalar union, strong when it can represent a `void` case.
                const is_strong: bool = for (u.fields) |f| {
                    if (f.type == void) break true;
                } else false;

                if (is_comp) {
                    if (is_strong) {
                        @compileError("Container is both composite and a strong scalar: " ++ @typeName(T));
                    }
                    return .composite;
                } else if (is_strong) {
                    return .strong_scalar;
                }
                return .weak_scalar;
            },
            // Leaf types (int, float, enum, pointer, void, etc) are scalars.
            else => .strong_scalar,
        };
    }
}

/// Returns if a leaf/field is not required
pub fn isNotRequired(comptime field: Type.StructField) bool {
    return comptime isOptional(field.type) or
        field.defaultValue() != null;
}

/// Number of required leaf keys of the type,
/// recursing into nested types (which are flattened into their leaf keys).
pub fn getRequiredKeyCount(comptime T: type) usize {
    comptime {
        const kind = containerKind(T);
        if (kind == .strong_scalar) return 1;

        switch (@typeInfo(T)) {
            .@"struct" => |S| {
                var count: usize = 0;
                for (S.fields) |f| {
                    // Doesn't matter if fields nested further down are required
                    if (isNotRequired(f)) continue;
                    count += getRequiredKeyCount(f.type);
                }
                return count;
            },
            .@"union" => |U| {
                if (isOptional(T)) @compileError("No optionals in unions!");

                var count: usize = std.math.maxInt(usize);
                if (U.fields.len == 0) @compileError("Union must have at least one variant");
                for (U.fields) |f| {
                    count = @min(count, getRequiredKeyCount(f.type));
                }
                return count;
            },
            else => return false,
        }
    }
}

test "getRequiredKeyCount: weak scalar" {
    const Weak = union(enum) {
        foo: i32,
    };

    // forced to be composite because it's directly assigned to the QP
    const QP1 = Weak;
    try std.testing.expectEqual(1, comptime getRequiredKeyCount(QP1));

    // forced to be scalar because of the parent struct
    const QP2 = struct { value: Weak };
    try std.testing.expectEqual(1, comptime getRequiredKeyCount(QP2));
}

const Basic = struct {
    start_date: []const u8,
};

const Detailed = struct {
    start_date: []const u8,
    end_date: []const u8,
};

const RangeUnion = union(enum) {
    basic: Basic,
    detailed: Detailed,
    all: struct {
        page: ?u32 = null,
    },
};

const CustomParam = struct {
    n: u32,
    pub fn paramParse(_: Allocator, _: []const u8) !@This() {
        return null;
    }
};

test "getRequiredKeyCount: scores by required key count" {
    const fields = @typeInfo(RangeUnion).@"union".fields;
    // basic
    try std.testing.expectEqual(1, comptime getRequiredKeyCount(fields[0].type));
    // detailed
    try std.testing.expectEqual(2, comptime getRequiredKeyCount(fields[1].type));
    // fallback (all optional leaves = score of 0)
    try std.testing.expectEqual(0, comptime getRequiredKeyCount(fields[2].type));
}

test "getRequiredKeyCount: flattened nested keys, single leaf variants score 1" {
    const U = union(enum) {
        nested: struct {
            range: struct {
                start: []const u8,
                end: []const u8,
            },
            opt: ?u32 = null,
        },
        custom: CustomParam,
    };

    const fields = @typeInfo(U).@"union".fields;
    // Nested flattens to { start, end }, and `opt` is optional and excluded
    try std.testing.expectEqual(2, comptime getRequiredKeyCount(fields[0].type));
    // Structs using `paramParse` resolve to a single key
    try std.testing.expectEqual(1, comptime getRequiredKeyCount(fields[1].type));
}

test "getRequiredKeyCount: nested union fallback" {
    const T = union(enum) {
        foo: union(enum) {
            bar: struct {
                baz: ?i32 = null,
            },
        },
    };

    const fields = @typeInfo(T).@"union".fields;
    try std.testing.expectEqual(0, comptime getRequiredKeyCount(fields[0].type));
}

/// Whether a type carries a custom `paramParse` function.
pub fn hasParamParse(comptime T: type) bool {
    return comptime std.meta.hasFn(T, "paramParse");
}
