const std = @import("std");
const Allocator = std.mem.Allocator;
const zap = @import("../zap/zap.zig");
const MiddlewareFn = zap.Endpoint.MiddlewareFn;
const Request = zap.Request;
const HttpError = zap.HttpError;
const StatusCode = zap.StatusCode;

pub fn cors(comptime Context: type) MiddlewareFn(Context) {
    return struct {
        fn cors(_: *Context, _: Allocator, req: Request) anyerror!void {
            try _cors(req);
        }
    }.cors;
}

pub fn _cors(req: Request) anyerror!void {
    _setHeaders(req) catch |err| {
        std.log.err("CORS error: {}\n", .{err});
        return try req.respondWithError(
            StatusCode.internal_server_error,
            "Failed to set CORS headers",
        );
    };
}

pub fn _setHeaders(req: Request) !void {
    try req.setHeader("Access-Control-Allow-Origin", "*");
    try req.setHeader("Access-Control-Allow-Methods", "GET, POST, PUT, PATCH, DELETE, OPTIONS");
    try req.setHeader("Access-Control-Allow-Headers", "Content-Type, Authorization");
}
