const std = @import("std");
const Allocator = std.mem.Allocator;
const assert = std.debug.assert;
const eql = std.mem.eql;

const builtin = @import("builtin");

const zap = @import("../zap/zap.zig");
const MiddlewareContext = zap.Endpoint.MiddlewareContext;
const MiddlewareFn = zap.Endpoint.MiddlewareFn;
const Request = zap.Request;
const HttpError = zap.HttpError;
const StatusCode = zap.StatusCode;

// Auto middleware
const parseQueryParams = @import("parse-query-params.zig").parseQueryParams;
const parseBody = @import("parse-body.zig").parseBody;
const cors = @import("cors.zig").cors;

// NOTE: When adding new auto middleware,
// add the name of the checked Context field (e.g. "body") to this struct.
// Then, update the `auto` function.
const AutoMiddleware = struct {
    // pub const _
    decls: struct {
        cors: bool = false,
        middleware: bool = false,
    } = .{},
    // Normal fields within a Context object
    fields: struct {
        req: bool = false,
        body: bool = false,
        query_params: bool = false,
    } = .{},
};

/// Populates AutoMiddleware based on the given context.
fn determine_middleware(comptime Context: type) AutoMiddleware {
    const ctx_info = @typeInfo(Context);
    assert(ctx_info == .@"struct");

    comptime var auto_middleware: AutoMiddleware = .{};

    comptime {
        const auto_middleware_info = @typeInfo(AutoMiddleware);
        assert(auto_middleware_info == .@"struct");

        // Declarations (pub const)

        if (@hasDecl(Context, "cors")) {
            if (@TypeOf(Context.cors) != bool) {
                @compileError(@typeName(Context) ++ " \"cors\" field is not a bool");
            }
            if (Context.cors) {
                auto_middleware.decls.cors = true;
            }
        }

        if (@hasDecl(Context, "middleware")) {
            for (@field(Context, "middleware")) |f| {
                if ((@TypeOf(f) != MiddlewareFn(Context))) {
                    @compileError("Invalid middleware on: " ++ @typeName(Context));
                }
            }
            auto_middleware.decls.middleware = true;
        }

        // Fields
        auto_middleware.fields.req = @hasField(Context, "req");
        auto_middleware.fields.body = @hasField(Context, "body");
        auto_middleware.fields.query_params = @hasField(Context, "query_params");
    }
    return auto_middleware;
}

pub fn auto(comptime Context: type) MiddlewareFn(Context) {
    const auto_middleware: AutoMiddleware = comptime determine_middleware(Context);

    return struct {
        fn auto(ctx: *MiddlewareContext(Context)) anyerror!void {
            if (comptime auto_middleware.fields.req) {
                @field(ctx, "req") = ctx.req;
            }

            if (comptime auto_middleware.decls.cors) {
                try cors(Context)(ctx);
                if (ctx.req.isFinished()) return;
            }

            if (comptime auto_middleware.fields.query_params) {
                try parseQueryParams(Context)(ctx);
                if (ctx.req.isFinished()) return;
            }

            if (comptime auto_middleware.fields.body) {
                try parseBody(Context)(ctx);
                if (ctx.req.isFinished()) return;
            }

            if (comptime auto_middleware.decls.middleware) {
                inline for (@field(Context, "middleware")) |middleware| {
                    try middleware(ctx);
                    if (ctx.req.isFinished()) return;
                }
            }
        }
    }.auto;
}

test "AutoMiddleware is false for everything with an empty context" {
    const EmptyContext = struct {};
    const auto_middleware = determine_middleware(EmptyContext);

    const decls = @typeInfo(@FieldType(AutoMiddleware, "decls"));
    inline for (decls.@"struct".fields) |field| {
        assert(@field(@field(auto_middleware, "decls"), field.name) == false);
    }

    const fields = @typeInfo(@FieldType(AutoMiddleware, "fields"));
    inline for (fields.@"struct".fields) |field| {
        assert(@field(@field(auto_middleware, "fields"), field.name) == false);
    }
}

test "AutoMiddleware is true for everything with a \"full\" context" {
    const FullContext = struct {
        pub const cors = true;

        // Example of providing extra middleware
        pub const Self = @This();
        pub const middleware = &.{parseBody(Self)};

        req: Request,
        body: struct {},
        query_params: struct {},
    };

    const auto_middleware = determine_middleware(FullContext);

    // The following checks should auto-fail if anything is added to the AutoMiddleware type.

    const decls = @typeInfo(@FieldType(AutoMiddleware, "decls"));
    inline for (decls.@"struct".fields) |field| {
        assert(@field(@field(auto_middleware, "decls"), field.name) == true);
    }

    const fields = @typeInfo(@FieldType(AutoMiddleware, "fields"));
    inline for (fields.@"struct".fields) |field| {
        assert(@field(@field(auto_middleware, "fields"), field.name) == true);
    }
}
