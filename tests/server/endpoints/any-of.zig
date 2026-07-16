const std = @import("std");
const Allocator = std.mem.Allocator;

const jolt = @import("jolt");
const types = jolt.types;
const Response = jolt.Response;
const Optional = types.Optional;
const Constraints = types.Constraints;

const GetContext = struct {
    query_params: struct {
        query: Optional([]const u8) = .not_provided,
        email: Optional([]const u8) = .not_provided,
        username: Optional([]const u8) = .not_provided,
        role: Optional([]const u8) = .not_provided,

        pub const constraints: Constraints = .{ .any_of = true };
    },
};

pub const AnyOfQpDetails = struct {
    query: ?[]const u8,
    email: ?[]const u8,
    username: ?[]const u8,
    role: ?[]const u8,
};

pub fn get(ctx: *GetContext, _: Allocator) !Response(AnyOfQpDetails) {
    const qp = ctx.query_params;
    return .{ .body = .{
        .query = qp.query.get(),
        .email = qp.email.get(),
        .username = qp.username.get(),
        .role = qp.role.get(),
    } };
}
