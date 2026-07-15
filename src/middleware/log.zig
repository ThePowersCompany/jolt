const std = @import("std");
const Allocator = std.mem.Allocator;
const zap = @import("../zap/zap.zig");
const Response = zap.Endpoint.Response;
const Request = zap.Request;

/// Logs the given string as a debug statement.
pub fn log(comptime Context: type, str: []const u8) Response(Context) {
    return struct {
        fn log(_: *Context, _: Allocator, _: Request) anyerror!void {
            std.log.debug("{s}", .{str});
        }
    }.log;
}
