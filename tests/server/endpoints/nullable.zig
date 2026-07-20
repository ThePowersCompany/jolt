const std = @import("std");
const Allocator = std.mem.Allocator;

const jolt = @import("jolt");
const Response = jolt.Response;

// A bare `?T` field is both required and nullable
const GetContext = struct {
    query_params: struct {
        required: ?i32,
    },
};

pub const NullableQpDetails = struct {
    is_null: bool,
    value: ?i32,
};

pub fn get(ctx: *GetContext, _: Allocator) !Response(NullableQpDetails) {
    const v = ctx.query_params.required;
    return .{ .body = .{
        .is_null = v == null,
        .value = v,
    } };
}
