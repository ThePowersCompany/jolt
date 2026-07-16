const std = @import("std");

const zap = @import("../zap/zap.zig");
const MiddlewareContext = zap.Endpoint.MiddlewareContext;
const Request = zap.Request;

// Built-in middleware
const cors = @import("cors.zig");
const parseQueryParams = @import("parse-query-params.zig").parseQueryParams;
const parseBody = @import("parse-body.zig").parseBody;

pub fn auto(comptime Context: type, ctx: *const MiddlewareContext(Context)) !void {
    if (cors.isEnabled(Context) or ctx.server.cors) {
        try cors.setCors(&ctx.req);
        if (ctx.req.isFinished()) return;
    }

    // Step 1: Scan Context and find all middleware
    if (@typeInfo(Context) != .@"struct") {
        @compileError("Endpoint context must be a struct");
    }
    inline for (@typeInfo(Context).@"struct".fields) |f| {
        if (f.type == Request) {
            @field(ctx.ctx, f.name) = ctx.req;
        } else if (std.meta.hasFn(f.type, "middleware")) {
            // TODO need to support optional middleware

            // Found middleware type
            const M = f.type;
            @field(ctx.ctx, f.name) = try M.middleware(&.{
                .ctx = &{}, // TODO need to recursively resolve dependencies
                .alloc = ctx.alloc,
                .server = ctx.server,
                .req = ctx.req,
            });
        }
    }

    // Step 2: Calculate final middleware order by resolving middleware dependencies
    // TODO

    // Step 3: Execute middleware in order, de-duplicate middleware results, and provide results to dependents
    // TODO

    if (@hasField(Context, "query_params")) {
        try parseQueryParams(Context, ctx);
        if (ctx.req.isFinished()) return;
    }

    if (@hasField(Context, "body")) {
        try parseBody(Context, ctx);
        if (ctx.req.isFinished()) return;
    }
}
