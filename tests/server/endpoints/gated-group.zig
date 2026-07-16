const std = @import("std");
const Allocator = std.mem.Allocator;

const jolt = @import("jolt");
const types = jolt.types;
const Response = jolt.Response;
const Optional = types.Optional;

const GetContext = struct {
    query_params: struct {
        room: i32,
        cursor: Optional(struct {
            cursor: i64,
            before: bool = false,
        }) = .not_provided,
        limit: u32 = 10,
    },
};

pub fn get(_: *GetContext, _: Allocator) !Response(void) {
    return .{};
}
