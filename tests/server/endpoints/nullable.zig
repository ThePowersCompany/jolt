const std = @import("std");
const Allocator = std.mem.Allocator;

const jolt = @import("jolt");
const Response = jolt.Response;

// A bare `?T` field is both required and nullable
const GetContext = struct {
    query_params: struct {
        first: ?i32,
        second: ?i32,
    },
};

pub const NullableQpDetails = struct {
    first_is_null: bool,
    first_value: ?i32,
    second_is_null: bool,
    second_value: ?i32,
};

pub fn get(ctx: *GetContext, _: Allocator) !Response(NullableQpDetails) {
    const a = ctx.query_params.first;
    const b = ctx.query_params.second;
    return .{ .body = .{
        .first_is_null = a == null,
        .first_value = a,
        .second_is_null = b == null,
        .second_value = b,
    } };
}
