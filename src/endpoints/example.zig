const std = @import("std");
const Allocator = std.mem.Allocator;

const Response = @import("../main.zig").Endpoint.Response;

const CreateUserRequest = struct {
    is_active: bool,
    first_name: []u8,
    last_name: []u8,
    company_id: i32,
    site_id: i32,
    role: []u8,
    email: []u8,
    username: []u8,
};

const PostContext = struct {
    body: CreateUserRequest,
};

pub fn post(ctx: *PostContext, arena_alloc: Allocator) !Response(void) {
    _ = ctx;
    _ = arena_alloc;
    return .{};
}
