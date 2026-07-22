const std = @import("std");
const Type = std.builtin.Type;

const zap = @import("../zap/zap.zig");
const MiddlewareContext = zap.Endpoint.MiddlewareContext;
const MiddlewareResult = zap.Endpoint.MiddlewareResult;
const Request = zap.Request;

const types = @import("../utils/types.zig");

// Built-in middleware
const cors = @import("cors.zig");
const parseQueryParams = @import("parse-query-params.zig").parseQueryParams;
const parseBody = @import("parse-body.zig").parseBody;

pub fn auto(comptime Context: type, ctx: *MiddlewareContext(Context)) !void {
    if (@typeInfo(Context) != .@"struct") {
        @compileError("Endpoint context must be a struct");
    }

    if (cors.isEnabled(Context) or ctx.server.cors) {
        try cors.setCors(&ctx.req);
    }
    if (@hasField(Context, "req")) {
        @field(ctx.deps, "req") = ctx.req;
    }

    try execute(Context, ctx);

    if (@hasField(Context, "query_params")) {
        try parseQueryParams(Context, ctx);
    }

    if (@hasField(Context, "body")) {
        try parseBody(Context, ctx);
    }
}

fn execute(comptime Context: type, ctx: *MiddlewareContext(Context)) !void {
    inline for (analyze(Context, &.{})) |M| {
        // Prepare middleware dependencies
        const D = dependencies(M);
        const deps: D = deps: {
            // Loop through each dependency
            var d: D = undefined;
            inline for (dependencyFields(D)) |f| {
                const F: type = extractDependency(f.type, false);
                const p: ?*F = ptr(Context, F, &ctx.deps);
                // Copy the middleware result from context memory (p) into dependency field
                @field(d, f.name) = switch (@typeInfo(f.type)) {
                    .pointer => (
                        // Middleware dependencies can be pointers
                        p orelse {
                            // Middleware result is null in the context
                            return error.MiddlewareError;
                        }),
                    .optional => |O| (
                        // Middleware dependencies can be optional (if dependency fails, middleware can proceed with null dependency result)
                        switch (@typeInfo(O.child)) {
                            .pointer => p,
                            else => if (p) |pp| pp.* else null,
                        }),
                    else => (p orelse {
                        // Middleware result is null in the context
                        return error.MiddlewareError;
                    }).*,
                };
            }
            break :deps d;
        };
        // Execute middleware with dependencies
        const result: MiddlewareResult(M) = try M.middleware(&.{
            .deps = deps,
            .alloc = ctx.alloc,
            .server = ctx.server,
            .req = ctx.req,
        });
        // Note: If middleware produces a Zig error, the endpoint is immediately aborted
        // Store middleware result in top-level context
        const ctx_field = field(Context, M);
        @field(ctx.deps, ctx_field.name) = switch (@typeInfo(ctx_field.type)) {
            .optional => switch (result) {
                .ok => |v| v,
                .err => null,
            },
            else => switch (result) {
                .ok => |v| v,
                .err => |e| {
                    try ctx.req.respondWithError(e.status, e.msg);
                    return error.MiddlewareError;
                },
            },
        };
    }
}

fn extractDependency(comptime T: type, comptime top_level: bool) type {
    return switch (@typeInfo(T)) {
        .pointer => |P| {
            if (@typeInfo(P.child) == .optional) @compileError("Middleware dependency cannot be a pointer to optional");
            if (top_level) @compileError("Pointers are not supported in endpoint context");
            return P.child;
        },
        .optional => |O| {
            if (@typeInfo(O.child) == .optional) @compileError("Middleware dependency cannot be double optional");
            return extractDependency(O.child, top_level);
        },
        else => T,
    };
}

fn extractMiddleware(comptime T: type, comptime top_level: bool) ?type {
    const B: type = extractDependency(T, top_level);
    return if (std.meta.hasFn(B, "middleware")) B else null;
}

/// Recursively processes an endpoint context struct and extracts all of the middleware types into a flat list.
/// The resulting flat list is a topological sort (reverse postorder traversal) of the middleware dependency graph.
/// If the middleware is executed in this order, dependencies will always be resolved before their dependents require their result.
fn analyze(comptime Context: type, comptime stack: []const type) []const type {
    comptime {
        var middlewares: []const type = &.{};
        for (dependencyFields(Context)) |f| {
            if (extractMiddleware(f.type, stack.len == 0)) |M| {
                // Check if middleware recursive dependencies form a cycle
                for (stack) |C| {
                    if (C == M) @compileError("Middleware contains recursive dependency cycle");
                }
                // Check if middleware has already been discovered
                if (std.mem.indexOfScalar(type, middlewares, M) == null) {
                    // Recursively analyze dependencies
                    for (analyze(dependencies(M), stack ++ .{M})) |A| {
                        if (std.mem.indexOfScalar(type, middlewares, A) == null) {
                            middlewares = middlewares ++ .{A};
                        }
                    }
                    // Add middleware
                    middlewares = middlewares ++ .{M};
                }
            }
        }
        return middlewares;
    }
}

test "analyze and execute" {
    const M1 = struct {
        pub fn middleware(ctx: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            _ = ctx;
            return .{ .ok = .{} };
        }
    };
    const M2 = struct {
        const D = struct {
            m: M1,
        };

        pub fn middleware(ctx: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
            _ = ctx;
            return .{ .ok = .{} };
        }
    };
    const M3 = struct {
        const D = struct {
            m: M1,
            m2: *M2,
        };

        pub fn middleware(ctx: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
            _ = ctx;
            return .{ .ok = .{} };
        }
    };
    const C = struct {
        m: M3,
        m1: M1,
        m2: M2,
    };

    const t = analyze(C, &.{});
    try std.testing.expect(t[0] == M1);
    try std.testing.expect(t[1] == M2);
    try std.testing.expect(t[2] == M3);

    var ctx: MiddlewareContext(C) = .{
        .deps = undefined,
        .alloc = std.testing.allocator,
        .server = &undefined,
        .req = undefined,
    };
    try execute(C, &ctx);
}

/// Get the dependencies of a middleware type.
/// The dependencies are defined at comptime in a struct type (or void).
fn dependencies(comptime M: type) type {
    if (!std.meta.hasFn(M, "middleware")) @compileError("Invalid middleware: Missing function");
    const fnInfo = @typeInfo(@TypeOf(M.middleware)).@"fn";
    if (fnInfo.params.len != 1) @compileError("Invalid middleware: Incorrect number of function params");
    const ctxPtrInfo = @typeInfo(fnInfo.params[0].type orelse @compileError("Null context type!"));
    if (ctxPtrInfo != .pointer) @compileError("Invalid middleware: First param is not a context pointer");
    if (!ctxPtrInfo.pointer.is_const) @compileError("Invalid middleware: Context pointer must be const");
    const Ctx: type = ctxPtrInfo.pointer.child;
    if (@typeInfo(Ctx) != .@"struct" or !@hasField(Ctx, "deps")) @compileError("Invalid middleware: First param is not a context: " ++ @typeName(Ctx));
    const Deps: type = @FieldType(Ctx, "deps");
    if (@typeInfo(Deps) != .@"struct" and Deps != void) @compileError("Invalid middleware: Context dependencies are not a struct");
    return Deps;
}

fn dependencyFields(comptime D: type) []const Type.StructField {
    return switch (@typeInfo(D)) {
        .void => &.{},
        .@"struct" => |S| S.fields,
        else => @compileError("Middleware dependencies must be a struct or void"),
    };
}

/// Search context for middleware and return a read-only pointer to the middleware result memory.
fn ptr(comptime Context: type, comptime Middleware: type, ctx: *Context) ?*Middleware {
    const f = field(Context, Middleware);
    return types.unwrapPtr(f.type, &(@field(ctx, f.name)));
}

test "ptr" {
    const E = struct {
        i: i32,
    };
    const S = struct {
        e: E,
    };
    var s: S = .{ .e = .{ .i = 123 } };
    var p: ?*E = ptr(S, E, &s);
    p.?.i = 456;
    try std.testing.expectEqual(456, s.e.i);
}

fn field(comptime Context: type, comptime Middleware: type) Type.StructField {
    comptime {
        const ctxInfo = @typeInfo(Context);
        if (ctxInfo != .@"struct") @compileError("Context must be a struct");
        var found: ?Type.StructField = null;
        for (ctxInfo.@"struct".fields) |f| {
            if (types.Unwrap(f.type) == Middleware) {
                if (found != null) @compileError("Duplicate middleware defined in top-level endpoint context: " ++ @typeName(f.type));
                found = f;
            }
        }
        return found orelse @compileError("Unable to find middleware defined in top-level endpoint context: " ++ @typeName(Middleware));
    }
}

test "field" {
    const E = struct {
        i: i32,
    };
    const S = struct {
        e: E,
    };
    const T = struct {
        e: ?E,
    };
    const p: Type.StructField = field(S, E);
    try std.testing.expectEqualStrings("e", p.name);
    const q: Type.StructField = field(T, E);
    try std.testing.expectEqualStrings("e", q.name);
}
