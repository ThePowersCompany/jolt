const std = @import("std");

const zap = @import("../zap/zap.zig");
const MiddlewareContext = zap.Endpoint.MiddlewareContext;
const Request = zap.Request;

// Built-in middleware
const cors = @import("cors.zig").cors;
const parseQueryParams = @import("parse-query-params.zig").parseQueryParams;
const parseBody = @import("parse-body.zig").parseBody;

pub fn auto(comptime Context: type, ctx: *MiddlewareContext(Context)) !void {
    // Support CORS
    const cors_enabled: bool = if (@hasDecl(Context, "cors")) cors: {
        if (@TypeOf(Context.cors) != bool) {
            @compileError(@typeName(Context) ++ " \"cors\" field is not a bool");
        }
        break :cors Context.cors;
    } or ctx.server.cors;
    if (cors_enabled) {
        try cors(&ctx.req);
        if (ctx.req.isFinished()) return;
    }

    // Step 1: Scan Context and find all middleware
    inline for (@typeInfo(Context).@"struct".fields) |f| {
        if (f.type == Request) {
            @field(ctx.ctx, f.name) = ctx.req;
        }
        // TODO recursively scan context to find all middleware types
    }

    if (@hasField(Context, "query_params")) {
        try parseQueryParams(Context, ctx);
        if (ctx.req.isFinished()) return;
    }

    if (@hasField(Context, "body")) {
        try parseBody(Context, ctx);
        if (ctx.req.isFinished()) return;
    }

    // Step 2: Calculate final middleware order by resolving middleware dependencies
    // TODO

    // Step 3: Execute middleware in order, de-duplicate middleware results, and provide results to dependents
    // TODO
}
