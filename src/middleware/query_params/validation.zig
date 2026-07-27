const std = @import("std");
const Allocator = std.mem.Allocator;
const Type = std.builtin.Type;

const types_module = @import("../../utils/types.zig");
const Optional = types_module.Optional;
const isOptional = types_module.isOptional;
const Unwrap = types_module.Unwrap;

const UnionRepr = @import("../../utils/unions.zig").UnionRepr;
const expect = std.testing.expect;
const expectEqual = std.testing.expectEqual;
const expectEqualStrings = std.testing.expectEqualStrings;

const containsString = @import("../../utils/array_utils.zig").containsString;

/// Whether a struct type carries a custom `paramParse`
/// and therefore stays a single query key (its value parsed from a string),
/// rather than being flattened into leaf keys.
pub fn hasParamParse(comptime T: type) bool {
    return comptime std.meta.hasFn(T, "paramParse");
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

/// Whether `T` is a plain struct that flattens into leaf keys,
/// as opposed to a `paramParse` struct (an opaque single-key leaf).
fn isFlattenedStruct(comptime T: type) bool {
    return comptime @typeInfo(T) == .@"struct" and !hasParamParse(T);
}

/// Returns if `str` appears in `list`.
fn contains(comptime list: []const []const u8, comptime str: []const u8) bool {
    comptime {
        for (list) |x| if (std.mem.eql(u8, x, str)) return true;
        return false;
    }
}

/// Returns the flattened leaf key names of a plain struct,
/// hoisting nested plain structs and skipping lifted union fields (handled by the caller).
pub fn getFlatLeafNames(comptime T: type) []const []const u8 {
    comptime {
        var names: []const []const u8 = &.{};
        for (@typeInfo(T).@"struct".fields) |field| {
            if (isLiftableUnion(field.type)) continue;
            const Child = if (isOptional(field.type)) field.type.childType() else field.type;
            if (isFlattenedStruct(Child)) {
                names = names ++ getFlatLeafNames(Child);
            } else {
                names = names ++ &[_][]const u8{field.name};
            }
        }
        return names;
    }
}

/// Returns the flat key names contributed by a single union variant.
pub fn getVariantKeyNames(comptime variant: Type.UnionField) []const []const u8 {
    comptime {
        if (isFlattenedStruct(variant.type)) return getFlatLeafNames(variant.type);
        return &[_][]const u8{variant.name};
    }
}

/// Like `getFlatLeafNames`, but only the required leaves.
/// Fields that are optional (`Optional` wrapper or a native `?T`) or have a default, are skipped.
pub fn getRequiredLeafNames(comptime T: type) []const []const u8 {
    comptime {
        var names: []const []const u8 = &.{};
        for (@typeInfo(T).@"struct".fields) |field| {
            if (isLiftableUnion(field.type)) continue;

            const not_required = isOptional(field.type) or
                @typeInfo(field.type) == .optional or
                field.defaultValue() != null;
            if (not_required) continue;

            if (isFlattenedStruct(field.type)) {
                names = names ++ getRequiredLeafNames(field.type);
            } else {
                names = names ++ &[_][]const u8{field.name};
            }
        }
        return names;
    }
}

/// Returns the required key names a variant needs in order to be selected.
pub fn getVariantRequiredKeyNames(comptime variant: Type.UnionField) []const []const u8 {
    comptime {
        if (isFlattenedStruct(variant.type)) return getRequiredLeafNames(variant.type);
        return &[_][]const u8{variant.name};
    }
}

/// Returns if a leaf/field is not required
fn isNotRequired(comptime field: Type.StructField) bool {
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
        if (hasParamParse(T)) return .strong_scalar;

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

/// Returns if every required leaf key of struct `T` is in `present_keys`.
pub fn requiredKeysPresent(comptime T: type, present_keys: []const []const u8) bool {
    inline for (@typeInfo(T).@"struct".fields) |field| {
        if (comptime isNotRequired(field)) continue;

        if (comptime @typeInfo(field.type) == .@"struct" and containerKind(field.type) == .composite) {
            if (!requiredKeysPresent(field.type, present_keys)) return false;
        } else if (!containsString(present_keys, field.name)) {
            return false;
        }
    }
    return true;
}

/// Returns if any leaf key of struct `T` is in `present_keys`.
pub fn anyStructLeafKeyPresent(comptime T: type, present_keys: []const []const u8) bool {
    inline for (@typeInfo(T).@"struct".fields) |field| {
        if (comptime @typeInfo(field.type) == .@"struct" and containerKind(field.type) == .composite) {
            if (anyStructLeafKeyPresent(field.type, present_keys)) return true;
        } else if (containsString(present_keys, field.name)) {
            return true;
        }
    }
    return false;
}

/// Field index of union `T`'s all-optional fallback variant (zero required keys),
/// or null if it has none.
/// At most one may exist (enforced here at compile time).
pub fn findFallbackVariant(comptime T: type) ?std.meta.Tag(T) {
    comptime {
        var found: ?std.meta.Tag(T) = null;
        for (@typeInfo(T).@"union".fields) |f| {
            if (getRequiredKeyCount(f.type) == 0) {
                if (found != null) {
                    @compileError("Union " ++ @typeName(T) ++
                        " has more than one all-optional variant;" ++
                        " at most one may act as the fallback.");
                }
                found = @field(T, f.name);
            }
        }
        return found;
    }
}

/// Returns the first string that appears in both lists.
pub fn findFirstCommonString(
    comptime listA: []const []const u8,
    comptime listB: []const []const u8,
) ?[]const u8 {
    comptime {
        for (listA) |x| if (contains(listB, x)) return x;
        return null;
    }
}

/// Returns the first string appearing more than once in the list.
pub fn findFirstDuplicate(comptime list: []const []const u8) ?[]const u8 {
    comptime {
        for (list, 0..) |x, i| {
            if (contains(list[i + 1 ..], x)) return x;
        }
        return null;
    }
}

/// Returns if every string in `listA` also appears in `listB`.
pub fn isSubset(
    comptime listA: []const []const u8,
    comptime listB: []const []const u8,
) bool {
    comptime {
        for (listA) |str| if (!contains(listB, str)) return false;
        return true;
    }
}

/// Rejects any two union variants with identical required key sets.
pub fn assertUnambiguousUnion(comptime U: Type.Union, comptime label: []const u8) void {
    comptime {
        for (U.fields, 0..) |a, i| {
            const a_keys = getVariantRequiredKeyNames(a);
            for (U.fields[i + 1 ..]) |b| {
                const b_keys = getVariantRequiredKeyNames(b);
                if (isSubset(a_keys, b_keys) and isSubset(b_keys, a_keys)) {
                    @compileError(
                        "Variants '" ++ a.name ++ "' and '" ++ b.name ++ "' of " ++ label ++
                            " have identical required keys and cannot be told apart. " ++
                            "Make their required keys distinct (e.g. mark a distinguishing key as required).",
                    );
                }
            }
        }
    }
}

/// Query params share a flat key structure,
/// i.e. one value can't be assigned to two different fields in the zig query_params structure.
/// These collisions are ambiguous, so we reject here at compile time.
pub fn assertNoQueryKeyCollisions(comptime T: type) void {
    comptime {
        assertNoReprUnionInQuery(T);

        const info = @typeInfo(T);

        // Check that a variant's own keys do not conflict with themselves
        if (isLiftableUnion(T)) {
            for (info.@"union".fields) |variant| {
                if (findFirstDuplicate(getVariantKeyNames(variant))) |key| {
                    @compileError(
                        "Duplicate query key '" ++ key ++ "' within variant '" ++
                            variant.name ++ "' of union " ++ @typeName(T),
                    );
                }
            }
            assertUnambiguousUnion(info.@"union", "union " ++ @typeName(T));
            return;
        }

        if (info != .@"struct") return;

        // Base keys first, checked for internal duplicates.
        const base_keys = getFlatLeafNames(T);
        if (findFirstDuplicate(base_keys)) |key| {
            @compileError("Duplicate query key '" ++ key ++ "' in " ++ @typeName(T));
        }

        // List of keys that all exist in the flat key space
        var seen: []const []const u8 = base_keys;
        for (info.@"struct".fields) |field| {
            if (!isLiftableUnion(field.type)) continue;

            assertUnambiguousUnion(
                @typeInfo(field.type).@"union",
                "union field '" ++ field.name ++ "' in " ++ @typeName(T),
            );

            var union_names: []const []const u8 = &.{};
            for (@typeInfo(field.type).@"union".fields) |variant| {
                const variant_names = getVariantKeyNames(variant);

                if (findFirstDuplicate(variant_names)) |key| {
                    @compileError(
                        "Duplicate query key '" ++ key ++ "' within variant '" ++
                            variant.name ++ "' of union field '" ++ field.name ++
                            "' in " ++ @typeName(T),
                    );
                }

                if (findFirstCommonString(variant_names, seen)) |key| {
                    @compileError(
                        "Query key '" ++ key ++ "' from variant '" ++ variant.name ++
                            "' of union field '" ++ field.name ++
                            "' collides with another query key in " ++
                            @typeName(T) ++
                            ". Rename or restructure so each coexisting key is unique.",
                    );
                }

                // Variants of the same union may share names, so dedupe before merging into `seen`.
                for (variant_names) |n| {
                    if (!contains(union_names, n)) {
                        union_names = union_names ++ &[_][]const u8{n};
                    }
                }
            }
            seen = seen ++ union_names;
        }
    }
}

/// Searches `T` for a `_repr` union at any depth,
/// returning a describing message (including the field/variant path) if found, else `null`.
///
/// A tagged union requires a field to disambiguate the variant,
/// which cannot be represented in the flat query param key space.
/// Untagged unions are allowed since they infer the variant from the value itself.
pub fn findReprUnionInQuery(comptime T: type, comptime label: []const u8) ?[]const u8 {
    comptime {
        const U = Unwrap(T);
        switch (@typeInfo(U)) {
            .@"union" => |info| {
                if (hasTaggedRepr(U)) {
                    return "Union '" ++ @typeName(U) ++ //
                        "' with discriminator `_repr` cannot be a query param (" ++ label ++ ")";
                }
                // A liftable union flattens its fields into the key space,
                // so its variant payloads must be checked too.
                if (!hasParamParse(U)) for (info.fields) |variant| {
                    const variant_label = "variant '" ++ variant.name ++ "' of " ++ label;
                    if (findReprUnionInQuery(variant.type, variant_label)) |msg| {
                        return msg;
                    }
                };
            },
            .@"struct" => |info| {
                // A `paramParse` struct is a single opaque leaf, so we don't recurse.
                if (hasParamParse(U)) return null;

                for (info.fields) |field| {
                    const field_label = "field '" ++ field.name ++ "' of " ++ label;
                    if (findReprUnionInQuery(field.type, field_label)) |msg| {
                        return msg;
                    }
                }
            },
            else => {},
        }
        return null;
    }
}

/// Rejects at compile time if any union with `_repr` is used inside a `query_params` type.
pub fn assertNoReprUnionInQuery(comptime T: type) void {
    if (comptime findReprUnionInQuery(T, @typeName(T))) |msg| {
        @compileError(msg);
    }
}

const ReprUnion = union(enum) {
    a: struct { x: i32 },
    b: struct { y: i32 },
    pub const _repr: UnionRepr = .{ .adjacently = .{ .discriminator = "kind" } };
};

const LiftableUnion = union(enum) {
    by_id: struct { id: i32 },
    by_name: struct { name: []const u8 },
};

// A `paramParse` struct is parsed from a single string, so it is opaque.
// Its internal `_repr` union is never serialized as a tagged union and must be ignored by findReprUnionInQuery.
const ParamParseHidingRepr = struct {
    inner: ReprUnion,
    pub fn paramParse() void {}
};

// An untagged union with `_repr`, which should be allowed in query params since it needs no discriminator.
const UntaggedReprUnion = union(enum) {
    id: i32,
    auto: void,
    pub const _repr: UnionRepr = .untagged;
};

test "findReprUnionInQuery: valid query types return null" {
    try expectEqual(null, comptime findReprUnionInQuery(i32, "t"));
    try expectEqual(null, comptime findReprUnionInQuery(?i32, "t"));
    try expectEqual(null, comptime findReprUnionInQuery(enum { a, b }, "t"));
    try expectEqual(null, comptime findReprUnionInQuery(struct { a: i32, b: []const u8 }, "t"));
    try expectEqual(null, comptime findReprUnionInQuery(struct { a: i32, nested: struct { b: i32 } }, "t"));
    try expectEqual(null, comptime findReprUnionInQuery(LiftableUnion, "t"));
}

test "findReprUnionInQuery: a paramParse leaf is opaque, so its inner _repr union is ignored" {
    try expectEqual(null, comptime findReprUnionInQuery(ParamParseHidingRepr, "t"));
}

test "findReprUnionInQuery: detects a direct _repr union" {
    const msg = comptime findReprUnionInQuery(ReprUnion, "root");
    try expect(msg != null);
}

test "findReprUnionInQuery: allows untagged unions" {
    try expectEqual(null, comptime findReprUnionInQuery(UntaggedReprUnion, "root"));
    try expectEqual(null, comptime findReprUnionInQuery(struct { u: UntaggedReprUnion }, "root"));
}

test "findReprUnionInQuery: detects a _repr union as a struct field, reporting the path" {
    const S = struct {
        page: i32,
        payload: ReprUnion,
    };
    try expect(comptime findReprUnionInQuery(S, "root") != null);
}

test "findReprUnionInQuery: detects a _repr union through native and wrapper optionals" {
    {
        const msg = comptime findReprUnionInQuery(struct { p: ?ReprUnion }, "t");
        try expect(msg != null);
    }

    {
        const msg = comptime findReprUnionInQuery(struct { p: Optional(?ReprUnion) }, "t");
        try expect(msg != null);
    }
}

test "findReprUnionInQuery: detects a nested _repr union" {
    const Nested = struct { a: struct { b: struct { c: ReprUnion } } };
    const msg = comptime findReprUnionInQuery(Nested, "root");
    try expect(msg != null);
}

test "findReprUnionInQuery: detects a _repr union inside a liftable union variant" {
    const Q = union(enum) {
        simple: struct { id: i32 },
        complex: struct { payload: ReprUnion },
    };
    const msg = comptime findReprUnionInQuery(Q, "root");
    try expect(msg != null);
}
