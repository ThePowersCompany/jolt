const std = @import("std");

const zap = @import("../zap/zap.zig");
const MiddlewareContext = zap.Endpoint.MiddlewareContext;
const Request = zap.Request;

// Built-in middleware
const cors = @import("cors.zig");
const parseQueryParams = @import("parse-query-params.zig").parseQueryParams;
const parseBody = @import("parse-body.zig").parseBody;

pub fn auto(comptime Context: type, ctx: *const MiddlewareContext(Context)) !void {
    if (@typeInfo(Context) != .@"struct") {
        @compileError("Endpoint context must be a struct");
    }

    if (cors.isEnabled(Context) or ctx.server.cors) {
        try cors.setCors(&ctx.req);
        if (ctx.req.isFinished()) return;
    }
    if (@hasField(Context, "req")) {
        @field(ctx.deps, "req") = ctx.req;
    }

    const graph: MiddlewareGraph(Context) = .init(&ctx.deps);
    graph.execute();

    if (@hasField(Context, "query_params")) {
        try parseQueryParams(Context, ctx);
        if (ctx.req.isFinished()) return;
    }

    if (@hasField(Context, "body")) {
        try parseBody(Context, ctx);
        if (ctx.req.isFinished()) return;
    }
}

/// Caches middleware results and allows them to be passed to other middleware that requires their results
const MiddlewareCache = struct {
    // TODO
};

const MiddlewareNode = struct {
    typ: type,
    ptr: *anyopaque,
};

fn analyze(comptime Context: type) []const type {
    // TODO
    unreachable;
}

/// A directed acyclic graph of middleware types
fn MiddlewareGraph(comptime Context: type) type {
    const V = analyze(Context);
    return struct {
        // TODO

        const Self = @This();

        pub fn init(ptr: *Context) Self {
            // TODO
            unreachable;
        }

        /// Add middleware to the graph and any recursive dependencies, if applicable.
        pub fn add(self: *Self, comptime Dependent: type, comptime Middleware: type) void {
            // TODO middleware to the graph with the Dependent node
            self.addAll(Middleware, dependencies(Middleware));
        }

        /// Add all middleware dependencies to the graph.
        pub fn addAll(self: *Self, comptime Dependent: type, comptime Dependencies: type, ptr: *Dependencies) void {
            if (Dependencies == void) return;
            inline for (@typeInfo(Dependencies).@"struct".fields) |f| {
                if (std.meta.hasFn(f.type, "middleware")) {
                    self.add(Dependent, f.type);
                }
            }
        }

        /// Execute middleware in order, de-duplicate middleware results, and provide results to dependents
        pub fn execute(self: *const Self) void {
            var cache: MiddlewareCache = .{};
            // TODO

            // Calculate final middleware order by resolving middleware dependencies
            inline for (self.toposort()) |M| {
                // TODO need to support optional middleware

                // const M = f.type;
                @field(ctx.deps, f.name) = try M.middleware(&.{
                    .ctx = &{}, // TODO need to recursively resolve dependencies
                    .alloc = ctx.alloc,
                    .server = ctx.server,
                    .req = ctx.req,
                });
            }
        }

        /// Topologically sort the middleware types in the graph.
        /// This is also known as a reverse postorder traversal.
        /// If the middleware is executed in this order, dependencies will always be resolved before their dependents require their result.
        fn toposort(self: *const Self) []const type {
            // TODO
            unreachable;
        }

        /// Get the dependencies of a middleware type.
        /// The dependencies are defined at comptime in a struct type (or void).
        pub fn dependencies(comptime M: type) type {
            if (!std.meta.hasFn(M, "middleware")) @compileError("Invalid middleware: Missing function");
            const fnInfo = @typeInfo(@TypeOf(M.middleware)).@"fn";
            if (fnInfo.params.len != 1) @compileError("Invalid middleware: Incorrect number of function params");
            const ctxPtrInfo = @typeInfo(fnInfo.params[0].type orelse @compileError("Null context type!"));
            if (ctxPtrInfo != .pointer) @compileError("Invalid middleware: First param is not a context pointer");
            const Ctx = ctxPtrInfo.pointer.child;
            if (@typeInfo(Ctx) != .@"struct" or !@hasDecl(Ctx, "Ctx")) @compileError("Invalid middleware: First param is not a context: " ++ @typeName(Ctx));
            const Deps = Ctx.Dependencies;
            if (@typeInfo(Deps) != .@"struct" and Deps != void) @compileError("Invalid middleware: Context dependencies are not a struct");
            return Deps;
        }
    };
}
