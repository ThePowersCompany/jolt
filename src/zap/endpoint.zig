const std = @import("std");
const ArenaAllocator = std.heap.ArenaAllocator;
const ThreadSafeAllocator = std.heap.ThreadSafeAllocator;
const Allocator = std.mem.Allocator;

const builtin = @import("builtin");

// zap types
const zap = @import("zap.zig");
const Request = zap.Request;
const ListenerSettings = zap.HttpListenerSettings;
const HttpListener = zap.HttpListener;
const StatusCode = zap.StatusCode;

const JoltServer = @import("../main.zig").JoltServer;

const sortByStringLengthDesc = @import("../utils/array_utils.zig").sortByStringLengthDesc;
const stringify = @import("../utils/json.zig").stringify;

pub fn MiddlewareContext(comptime Context: type) type {
    return struct {
        ctx: *Context,
        alloc: Allocator,
        server: *JoltServer,
        req: Request,
    };
}

pub fn MiddlewareFn(comptime Context: type) type {
    return fn (ctx: *MiddlewareContext(Context)) anyerror!void;
}

pub const EnabledContext = struct {
    env: *std.process.EnvMap,
    alloc: Allocator,

    /// Gets a key from the env map, or panics if it does not exist.
    pub fn getEnvOrPanic(env: *std.process.EnvMap, comptime key: []const u8) []const u8 {
        const optValue = env.get(key);
        if (optValue) |v| return v;
        @panic(key ++ " is required");
    }
};

pub const EnabledFn = *const fn (EnabledContext) anyerror!bool;

const DefaultOptionsContext = struct {
    pub const cors = builtin.mode == .Debug;
};

pub fn defaultOptionsHandler(_: *DefaultOptionsContext, _: Allocator) !Response(void) {
    return .{};
}

pub const ErrorHandlerFn = *const fn (Request, anyerror) anyerror!void;

pub fn defaultErrorHandler(req: Request, err: anyerror) !void {
    std.log.err("Unhandled internal server error: {}\n", .{err});
    try req.respondWithStatus(.internal_server_error);
}

pub fn Response(comptime ReturnType: type) type {
    return struct {
        body: ?ReturnType = null,
        err: ?[]const u8 = null,
        content_type: ?[]const u8 = null,
        opts: std.json.Stringify.Options = .{},
        status: ?StatusCode = null,
        finished: bool = false, // supports WebSockets
    };
}

pub const RequestHandler = struct {
    handle_fn: *const fn (Allocator, *JoltServer, Request, ErrorHandlerFn) anyerror!void,

    pub fn init(comptime auto: anytype, comptime last_fn: anytype) !RequestHandler {
        const info: std.builtin.Type = @typeInfo(@TypeOf(last_fn));

        const ContextPtr = info.@"fn".params[0].type orelse @compileError("Null Context type!");
        const Context = @typeInfo(ContextPtr).pointer.child;
        if (@typeInfo(Context) != .@"struct") {
            @compileError("Provided Context is not a struct: " ++ @typeName(Context));
        }

        const return_type = info.@"fn".return_type orelse @compileError("Null return type!");
        const return_type_info = @typeInfo(return_type);
        const ResponseType: type = switch (return_type_info) {
            .void => @compileError("Handler return type cannot be void!"),
            .error_union => return_type_info.error_union.payload,
            else => return_type,
        };
        const ReturnType: type = switch (@typeInfo(ResponseType)) {
            .@"struct" => @typeInfo(std.meta.fieldInfo(ResponseType, .body).type).optional.child,
            else => @compileError("Handler function must return a Response!"),
        };
        if (ResponseType != Response(ReturnType)) {
            @compileError("Handler function must return a Response!");
        }
        return _init(Context, ReturnType, auto, last_fn);
    }

    fn _init(
        comptime Context: type,
        comptime ReturnType: type,
        comptime auto: anytype,
        comptime last_fn: *const fn (*Context, Allocator) anyerror!Response(ReturnType),
    ) !RequestHandler {
        const Wrapper = struct {
            pub fn handle(alloc: Allocator, server: *JoltServer, req: Request, sendErrorResponse: ErrorHandlerFn) !void {
                var context: Context = undefined;
                var middleware_context: MiddlewareContext(Context) = .{
                    .ctx = &context,
                    .alloc = alloc,
                    .server = server,
                    .req = req,
                };

                auto(Context)(&middleware_context) catch |err| {
                    std.log.err("Middleware error - {}\n", .{err});
                    return req.respondWithStatus(StatusCode.internal_server_error) catch |failed| {
                        std.log.err("Failed to send error to client: {}\n", .{failed});
                    };
                };

                if (req.isFinished()) {
                    return;
                }

                const response: Response(ReturnType) = last_fn(&context, alloc) catch |err| {
                    std.log.err("Endpoint fn error - {}\n", .{err});
                    return sendErrorResponse(req, err) catch |failed| {
                        std.log.err("Failed to send error to client: {}\n", .{failed});
                    };
                };

                // Exit early if the handler explicitly finished the response
                // This is required for WebSockets
                if (response.finished) {
                    return;
                }

                if (response.content_type) |c| {
                    try req.setHeader("content-type", c);
                }

                if (response.body) |body| outer: {
                    req.setStatus(response.status orelse StatusCode.ok);
                    const data = switch (@TypeOf(body)) {
                        []const u8 => blk: {
                            if (response.content_type == null) {
                                try req.setHeader("content-type", "text/plain");
                            }
                            break :blk body;
                        },
                        void => break :outer,
                        else => blk: {
                            if (response.content_type == null) {
                                try req.setHeader("content-type", "application/json");
                            }
                            break :blk try stringify(alloc, body, response.opts);
                        },
                    };
                    try req.sendBody(data);
                } else if (response.err) |err| {
                    const statusCode: StatusCode = response.status orelse StatusCode.internal_server_error;
                    const enumValue = @intFromEnum(statusCode);
                    if (enumValue < 300) {
                        std.log.err(
                            \\{s} is not an error response!
                            \\  With message:
                            \\  {s}
                        ,
                            .{ statusCode.toString(), err },
                        );
                    }
                    req.setStatus(statusCode);
                    try req.sendBody(err);
                } else {
                    req.setStatus(response.status orelse StatusCode.no_content);
                }

                req.markAsFinished(true);
            }
        };

        return RequestHandler{
            .handle_fn = &Wrapper.handle,
        };
    }

    pub fn handle(
        self: RequestHandler,
        allocator: Allocator,
        server: *JoltServer,
        req: Request,
        sendErrorResponse: ErrorHandlerFn,
    ) void {
        self.handle_fn(allocator, server, req, sendErrorResponse) catch |err| {
            std.log.err("Failed to generate response: {}", .{err});
            req.respondWithStatus(StatusCode.internal_server_error) catch |e| {
                std.log.err("Failed to fail: {}", .{e});
            };
        };
    }
};

pub const RequestHandlers = struct {
    getHandler: ?RequestHandler = null,
    postHandler: ?RequestHandler = null,
    putHandler: ?RequestHandler = null,
    patchHandler: ?RequestHandler = null,
    deleteHandler: ?RequestHandler = null,
    optionsHandler: ?RequestHandler = null,
    wsHandler: ?RequestHandler = null,
};

pub const Endpoint = struct {
    server: *JoltServer,
    path: []const u8,
    handlers: RequestHandlers,
    sendErrorResponse: ErrorHandlerFn,

    pub fn init(server: *JoltServer, path: []const u8, sendErrorResponse: ErrorHandlerFn, handlers: RequestHandlers) Endpoint {
        return .{
            .server = server,
            .path = path,
            .sendErrorResponse = sendErrorResponse,
            .handlers = handlers,
        };
    }

    pub fn onRequest(self: *const Endpoint, arena: *ArenaAllocator, req: Request) void {
        defer _ = arena.reset(.retain_capacity);
        switch (req.methodAsEnum()) {
            .GET => if (self.handlers.getHandler) |handler| {
                return handler.handle(arena.allocator(), self.server, req, self.sendErrorResponse);
            },
            .POST => if (self.handlers.postHandler) |handler| {
                return handler.handle(arena.allocator(), self.server, req, self.sendErrorResponse);
            },
            .PUT => if (self.handlers.putHandler) |handler| {
                return handler.handle(arena.allocator(), self.server, req, self.sendErrorResponse);
            },
            .PATCH => if (self.handlers.patchHandler) |handler| {
                return handler.handle(arena.allocator(), self.server, req, self.sendErrorResponse);
            },
            .DELETE => if (self.handlers.deleteHandler) |handler| {
                return handler.handle(arena.allocator(), self.server, req, self.sendErrorResponse);
            },
            .OPTIONS => if (self.handlers.optionsHandler) |handler| {
                return handler.handle(arena.allocator(), self.server, req, self.sendErrorResponse);
            },
            .UNKNOWN => {
                return;
            },
        }
        req.respondWithStatus(.not_found) catch |err| {
            std.log.err("Failed to respond to request: {}", .{err});
        };
    }

    pub fn onWebSocket(self: *const Endpoint, arena: *ArenaAllocator, req: Request) void {
        defer _ = arena.reset(.retain_capacity);
        // Pass WebSocket connection to handler:
        // It's the handler's responsibility to finish the upgrade of the request
        // because the receiver defines the connection state/context to be used.
        if (self.handlers.wsHandler) |handler| {
            handler.handle(arena.allocator(), self.server, req, self.sendErrorResponse);
        }
    }
};

pub const EndpointListenerError = error{
    /// Since we use .startsWith to check for matching paths, you cannot use
    /// endpoint paths that overlap at the beginning. --> When trying to register
    /// an endpoint whose path would shadow an already registered one, you will
    /// receive this error.
    EndpointPathShadowError,
};

/// The listener with endpoint support
///
/// NOTE: It switches on path.startsWith -> so use endpoints with distinctly starting names!!
pub const Listener = struct {
    listener: HttpListener,

    const Self = @This();

    var arena: ArenaAllocator = undefined;
    var tsa: ThreadSafeAllocator = undefined;
    var alloc: Allocator = undefined;

    /// Internal static structs of member endpoints
    var endpoints: std.ArrayList(*const Endpoint) = .empty;

    threadlocal var arenas: []ArenaAllocator = &.{};

    /// Initialize a new endpoint listener. Note, if you pass an `on_request`
    /// callback in the provided ListenerSettings, this request callback will be
    /// called every time a request arrives that no endpoint matches.
    pub fn init(a: Allocator, l: ListenerSettings) Self {
        arena = ArenaAllocator.init(a);
        tsa = .{ .child_allocator = arena.allocator() };
        alloc = tsa.allocator();

        // take copy of listener settings so it's mutable
        var ls = l;

        // override the settings with our internal, actual callback function
        // so that "we" will be called on request
        ls.on_request = Listener.onRequest;

        // required for websockets
        ls.on_upgrade = Listener.onUpgrade;

        return .{
            .listener = HttpListener.init(ls),
        };
    }

    /// De-init the listener and free its resources.
    /// Registered endpoints will not be de-initialized automatically; just removed
    /// from the internal map.
    pub fn deinit(_: *Self) void {
        endpoints.deinit(alloc);
        arena.deinit();
    }

    /// Call this to start listening. After this, no more endpoints can be
    /// registered.
    pub fn listen(self: *Self) !void {
        sortByStringLengthDesc(*const Endpoint, endpoints.items, "path");
        try self.listener.listen();
    }

    /// Register an endpoint with this listener.
    /// NOTE: endpoint paths are matched with startsWith -> so use endpoints with distinctly starting names!!
    /// If you try to register an endpoint whose path would shadow an already registered one, you will
    /// receive an EndpointPathShadowError.
    pub fn register(_: *Self, endpoint: *const Endpoint) !void {
        for (endpoints.items) |other| {
            if (std.mem.eql(u8, other.path, endpoint.path)) {
                return EndpointListenerError.EndpointPathShadowError;
            }
        }
        try endpoints.append(alloc, endpoint);
    }

    fn delegateToEndpoint(r: Request, comptime f: *const fn (*const Endpoint, *ArenaAllocator, Request) void) void {
        if (r.path) |p| {
            for (endpoints.items, 0..) |e, i| {
                if (std.mem.startsWith(u8, p, e.path)) {
                    // Lookup thread-local arena allocator
                    // Note: This allocation must happen on each thread (can't be done during `register` or `listen`)
                    if (arenas.len == 0) {
                        const cap = endpoints.items.len;
                        arenas = alloc.alloc(ArenaAllocator, cap) catch {
                            r.setStatus(StatusCode.internal_server_error);
                            std.log.err("Failed to allocate arena for endpoint: {s}", .{p});
                            return;
                        };
                        for (0..cap) |a| {
                            arenas[a] = ArenaAllocator.init(alloc);
                        }
                    }
                    f(e, &arenas[i], r);
                    return;
                }
            }
        }
        r.setStatus(StatusCode.not_found);
    }

    fn onRequest(r: Request) void {
        delegateToEndpoint(r, Endpoint.onRequest);
    }

    fn onUpgrade(r: Request, target_protocol: []const u8) void {
        // Verify HTTP Upgrade protocol
        if (!std.mem.eql(u8, target_protocol, "websocket")) {
            std.log.warn("received illegal protocol: {s}", .{target_protocol});
            r.respondWithError(StatusCode.bad_request, "Unsupported HTTP Upgrade protocol") catch {
                std.log.err("Failed to respond with 400 error for HTTP Upgrade", .{});
            };
            return;
        }
        delegateToEndpoint(r, Endpoint.onWebSocket);
    }
};
