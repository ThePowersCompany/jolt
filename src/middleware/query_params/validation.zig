const std = @import("std");
const Type = std.builtin.Type;
const isOptional = @import("../../utils/types.zig").isOptional;

/// Whether a struct type carries a custom `paramParse`
/// and therefore stays a single query key (its value parsed from a string),
/// rather than being flattened into leaf keys.
pub fn hasParamParse(comptime T: type) bool {
    return comptime std.meta.hasFn(T, "paramParse");
}

/// Returns if `T` is a tagged union that should be lifted into the flat query key space.
/// This ignores the `Optional` wrapper,
/// and unions with a `paramParse` function (which is parsed as a single leaf key).
pub fn isLiftableUnion(comptime T: type) bool {
    return comptime @typeInfo(T) == .@"union" and !isOptional(T) and !hasParamParse(T);
}

/// Whether `T` is a struct with at least one field
/// that is a non-Optional tagged union (without `paramParse`),
/// i.e. a union to be lifted into the flat query key space.
pub fn isStructContainingUnionField(comptime T: type) bool {
    if (@typeInfo(T) != .@"struct") return false;
    inline for (@typeInfo(T).@"struct".fields) |field| {
        if (comptime isLiftableUnion(field.type)) return true;
    }
    return false;
}

/// Returns the flattened leaf key names of a plain struct,
/// hoisting nested plain structs and skipping lifted union fields (handled by the caller).
pub fn getFlatLeafNames(comptime T: type) []const []const u8 {
    comptime {
        var names: []const []const u8 = &.{};
        for (@typeInfo(T).@"struct".fields) |field| {
            if (isLiftableUnion(field.type)) continue;
            const Child = if (isOptional(field.type)) field.type.childType() else field.type;
            if (@typeInfo(Child) == .@"struct" and !hasParamParse(Child)) {
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
        const V = variant.type;
        if (@typeInfo(V) == .@"struct" and !hasParamParse(V)) return getFlatLeafNames(V);
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

            if (@typeInfo(field.type) == .@"struct" and !hasParamParse(field.type)) {
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
        const V = variant.type;
        if (@typeInfo(V) == .@"struct" and !hasParamParse(V)) return getRequiredLeafNames(V);
        return &[_][]const u8{variant.name};
    }
}

/// Returns the first string that appears in both lists.
pub fn findFirstCommonString(
    comptime listA: []const []const u8,
    comptime listB: []const []const u8,
) ?[]const u8 {
    comptime {
        for (listA) |x| for (listB) |y| {
            if (std.mem.eql(u8, x, y)) return x;
        };
        return null;
    }
}

/// Returns the first string appearing more than once in the list.
pub fn findFirstDuplicate(comptime list: []const []const u8) ?[]const u8 {
    comptime {
        for (list, 0..) |x, i| {
            for (list[i + 1 ..]) |y| {
                if (std.mem.eql(u8, x, y)) return x;
            }
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
        for (listA) |str| {
            // TODO: Can this be simplified?
            if (findFirstCommonString(&[_][]const u8{str}, listB) == null) return false;
        }
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
                    if (findFirstCommonString(&[_][]const u8{n}, union_names) == null) {
                        union_names = union_names ++ &[_][]const u8{n};
                    }
                }
            }
            seen = seen ++ union_names;
        }
    }
}
