const std = @import("std");
const Allocator = std.mem.Allocator;

const jolt = @import("jolt");
const types = jolt.types;
const Response = jolt.Response;
const Optional = types.Optional;

const GetContext = struct {
    query_params: struct {
        query: Optional([]const u8) = .not_provided,
        email: Optional([]const u8) = .not_provided,
        username: Optional([]const u8) = .not_provided,
        role: Optional([]const u8) = .not_provided,

        pub const constraints: types.Constraints = .{ .any_of = true };
    },
};

pub fn get(_: *GetContext, _: Allocator) !Response(void) {
    return .{};
}
