const std = @import("std");
const Allocator = std.mem.Allocator;

const jolt = @import("jolt");
const types = jolt.types;
const Response = jolt.Response;
const Optional = types.Optional;

const GetContext = struct {
    query_params: struct {
        page: u32 = 1,
        filter: Optional(struct {
            category: Optional([]const u8) = .not_provided,
            status: Optional([]const u8) = .not_provided,
        }) = .not_provided,
    },
};

pub const NestedOptionalQpDetails = struct {
    page: u32,
    category: ?[]const u8,
    status: ?[]const u8,
};

pub fn get(ctx: *GetContext, _: Allocator) !Response(NestedOptionalQpDetails) {
    const qp = ctx.query_params;
    if (qp.filter.get()) |filter| {
        return .{ .body = .{
            .page = qp.page,
            .category = filter.category.get(),
            .status = filter.status.get(),
        } };
    }
    return .{ .body = .{
        .page = qp.page,
        .category = null,
        .status = null,
    } };
}
