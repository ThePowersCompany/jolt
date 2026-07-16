const std = @import("std");
const Allocator = std.mem.Allocator;

const Request = @import("../zap/zap.zig").Request;

pub fn isEnabled(comptime Context: type) bool {
    if (@hasDecl(Context, "cors")) {
        if (@TypeOf(Context.cors) != bool) {
            @compileError(@typeName(Context) ++ " \"cors\" field is not a bool");
        }
        return Context.cors;
    }
    return false;
}

pub fn setCors(req: *const Request) !void {
    _setHeaders(req) catch |err| {
        std.log.err("CORS error: {}\n", .{err});
        return try req.respondWithError(
            .internal_server_error,
            "Failed to set CORS headers",
        );
    };
}

fn _setHeaders(req: *const Request) !void {
    try req.setHeader("Access-Control-Allow-Origin", "*");
    try req.setHeader("Access-Control-Allow-Methods", "GET, POST, PUT, PATCH, DELETE, OPTIONS");
    try req.setHeader("Access-Control-Allow-Headers", "Content-Type, Authorization");
}
