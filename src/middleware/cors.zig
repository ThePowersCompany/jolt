const std = @import("std");
const Allocator = std.mem.Allocator;

const zap = @import("../zap/zap.zig");
const MiddlewareContext = zap.Endpoint.MiddlewareContext;
const MiddlewareFn = zap.Endpoint.MiddlewareFn;
const Request = zap.Request;

pub fn cors(comptime Context: type) MiddlewareFn(Context) {
    return struct {
        fn cors(ctx: *MiddlewareContext(Context)) anyerror!void {
            if (!ctx.server.cors) return;
            _setHeaders(ctx.req) catch |err| {
                std.log.err("CORS error: {}\n", .{err});
                return try ctx.req.respondWithError(
                    .internal_server_error,
                    "Failed to set CORS headers",
                );
            };
        }
    }.cors;
}

fn _setHeaders(req: Request) !void {
    try req.setHeader("Access-Control-Allow-Origin", "*");
    try req.setHeader("Access-Control-Allow-Methods", "GET, POST, PUT, PATCH, DELETE, OPTIONS");
    try req.setHeader("Access-Control-Allow-Headers", "Content-Type, Authorization");
}
