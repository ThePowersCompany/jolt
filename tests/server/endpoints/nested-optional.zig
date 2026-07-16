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

pub fn get(_: *GetContext, _: Allocator) !Response(void) {
    return .{};
}
