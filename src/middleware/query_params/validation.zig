const std = @import("std");
const Allocator = std.mem.Allocator;
const Type = std.builtin.Type;

const types_module = @import("../../utils/types.zig");
const Optional = types_module.Optional;
const isOptional = types_module.isOptional;
const Unwrap = types_module.Unwrap;

const containers_module = @import("../../utils/containers.zig");
const hasParamParse = containers_module.hasParamParse;

const unions_module = @import("../../utils/unions.zig");
const hasTaggedRepr = unions_module.hasTaggedRepr;
const isLiftableUnion = unions_module.isLiftableUnion;

const UnionRepr = @import("../../utils/unions.zig").UnionRepr;
const expect = std.testing.expect;
const expectEqual = std.testing.expectEqual;
const expectEqualStrings = std.testing.expectEqualStrings;

const containsString = @import("../../utils/array_utils.zig").containsString;

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
fn getFlatLeafNames(comptime T: type) []const []const u8 {
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
fn getVariantKeyNames(comptime variant: Type.UnionField) []const []const u8 {
    comptime {
        if (isFlattenedStruct(variant.type)) return getFlatLeafNames(variant.type);
        return &[_][]const u8{variant.name};
    }
}

/// Like `getFlatLeafNames`, but only the required leaves.
/// Fields that are optional (`Optional` wrapper or a native `?T`) or have a default, are skipped.
fn getRequiredLeafNames(comptime T: type) []const []const u8 {
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
fn getVariantRequiredKeyNames(comptime variant: Type.UnionField) []const []const u8 {
    comptime {
        if (isFlattenedStruct(variant.type)) return getRequiredLeafNames(variant.type);
        return &[_][]const u8{variant.name};
    }
}

/// Returns the first string that appears in both lists.
fn findFirstCommonString(
    comptime listA: []const []const u8,
    comptime listB: []const []const u8,
) ?[]const u8 {
    comptime {
        for (listA) |x| if (contains(listB, x)) return x;
        return null;
    }
}

/// Returns the first string appearing more than once in the list.
fn findFirstDuplicate(comptime list: []const []const u8) ?[]const u8 {
    comptime {
        for (list, 0..) |x, i| {
            if (contains(list[i + 1 ..], x)) return x;
        }
        return null;
    }
}

/// Returns if every string in `listA` also appears in `listB`.
fn isSubset(
    comptime listA: []const []const u8,
    comptime listB: []const []const u8,
) bool {
    comptime {
        for (listA) |str| if (!contains(listB, str)) return false;
        return true;
    }
}

/// Rejects any two union variants with identical required key sets.
fn assertUnambiguousUnion(comptime U: Type.Union, comptime label: []const u8) void {
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
fn findReprUnionInQuery(comptime T: type, comptime label: []const u8) ?[]const u8 {
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
fn assertNoReprUnionInQuery(comptime T: type) void {
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
