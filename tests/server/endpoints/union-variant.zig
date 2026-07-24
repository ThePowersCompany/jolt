const std = @import("std");
const Allocator = std.mem.Allocator;

const jolt = @import("jolt");
const types = jolt.types;
const Response = jolt.Response;
const Optional = types.Optional;
const Constraints = types.Constraints;

// A top-level tagged union as query_params: the active variant is inferred from which keys are
// present, ordered most-specific-first. Mirrors the "matched specific variant parse failure is
// preferred over fallback" and "orderedVariants" unit tests:
//   - `?start_date=..`            -> .date_range (most specific)
//   - `?id=..`                    -> .id
//   - `?line=..` / empty          -> .all (all-optional fallback, last)
const GetContext = struct {
    query_params: union(enum) {
        id: i32,
        date_range: struct {
            start_date: i32,
            end_date: Optional(i32) = .not_provided,
            line: Optional(i32) = .not_provided,
            shift: Optional(i32) = .not_provided,
        },
        all: struct {
            line: Optional(i32) = .not_provided,
            shift: Optional(i32) = .not_provided,
        },
    },
};

// Details about which variant was selected plus its key fields,
// so tests can assert variant selection by specificity (not declaration order).
pub const UnionVariantQpDetails = struct {
    variant: []const u8,
    id: ?i32 = null,
    start_date: ?i32 = null,
    end_date: ?i32 = null,
    line: ?i32 = null,
    shift: ?i32 = null,
};

pub fn get(ctx: *GetContext, _: Allocator) !Response(UnionVariantQpDetails) {
    const body: UnionVariantQpDetails = switch (ctx.query_params) {
        .id => |v| .{
            .variant = "id",
            .id = v,
        },
        .date_range => |v| .{
            .variant = "date_range",
            .start_date = v.start_date,
            .end_date = v.end_date.get(),
            .line = v.line.get(),
            .shift = v.shift.get(),
        },
        .all => |v| .{
            .variant = "all",
            .line = v.line.get(),
            .shift = v.shift.get(),
        },
    };

    return .{ .body = body };
}
