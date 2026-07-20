const std = @import("std");
const Allocator = std.mem.Allocator;

const jolt = @import("jolt");
const types = jolt.types;
const Response = jolt.Response;
const Optional = types.Optional;

const Tag = enum { red, green, blue };

// Comma separated arrays: `?ids=1,2,3` parses into a slice of ints,
// and `?tags=red,blue` parses into a slice of enums.
const GetContext = struct {
    query_params: struct {
        ids: Optional([]const u32) = .not_provided,
        tags: Optional([]const Tag) = .not_provided,
    },
};

pub const ArraysQpDetails = struct {
    ids: []const u32,
    tags: []const Tag,
};

pub fn get(ctx: *GetContext, _: Allocator) !Response(ArraysQpDetails) {
    const qp = ctx.query_params;
    return .{ .body = .{
        .ids = qp.ids.get() orelse &.{},
        .tags = qp.tags.get() orelse &.{},
    } };
}
