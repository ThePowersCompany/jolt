const std = @import("std");
const Allocator = std.mem.Allocator;

const Response = @import("jolt").Response;

const GetContext = struct {};

pub fn get(_: *GetContext, _: Allocator) !Response(void) {
    return .{ .status = .ok };
}
