const std = @import("std");
const builtin = @import("builtin");
const Allocator = std.mem.Allocator;
const ArenaAllocator = std.heap.ArenaAllocator;
const dotenv = @import("dotenv");
const zap = @import("./zap/zap.zig");
const auth = @import("middleware/auth.zig");
const RequestHandler = Endpoint.RequestHandler;

const getEnvOrPanic = Endpoint.EnabledContext.getEnvOrPanic;

pub const Request = zap.Request;
pub const Endpoint = zap.Endpoint;
pub const Response = Endpoint.Response;
pub const MiddlewareFn = Endpoint.MiddlewareFn;
pub const StatusCode = zap.StatusCode;

pub const middleware = .{
    @import("./middleware/cors.zig").cors,
    @import("./middleware/parse-body.zig").parseBody,
    @import("./middleware/parse-query-params.zig").parseQueryParams,
};

pub const ServerOpts = struct {
    port: u16,
    threads: i16,
    workers: i16,
};

pub const EndpointDef = struct { []const u8, type };

pub const PowersServer = struct {
    const Self = @This();

    alloc: Allocator,
    opts: ServerOpts,
    env_map: std.process.EnvMap,

    pub fn init(alloc: Allocator, opts: ServerOpts) !Self {
        dotenv.loadFrom(alloc, ".env", .{}) catch {
            // Ignore if .env file doesn't exist
        };

        return .{
            .alloc = alloc,
            .opts = opts,
            .env_map = try std.process.getEnvMap(alloc),
        };
    }

    pub fn deinit(self: *Self) void {
        self.env_map.deinit();
    }

    pub fn run(self: *Self, endpoints: []const EndpointDef, tasks: []type, auto: anytype) !void {
        var global_arena = ArenaAllocator.init(self.alloc);
        defer global_arena.deinit();
        var thread_safe_alloc = std.heap.ThreadSafeAllocator{ .child_allocator = self.alloc };
        var listener = try createListener(thread_safe_alloc.allocator(), self.opts.port);
        defer listener.deinit();

        var deinitFns = std.ArrayList(*const fn () void).init(global_arena.allocator());
        defer {
            for (deinitFns.items) |f| f();
            deinitFns.deinit();
        }

        inline for (endpoints) |def| blk: {
            const path, const typ = def;

            if (std.meta.hasFn(typ, "enabled")) {
                // Runtime enabled check
                const enabled_func: Endpoint.EnabledFn = typ.enabled;
                if (!try enabled_func(.{ .env = &self.env_map, .alloc = global_arena.allocator() })) {
                    // Not enabled, skip
                    break :blk;
                }
            } else if (@hasField(typ, "enabled")) {
                // Compile-time enabled check
                if (!@field(typ, "enabled")) {
                    // Not enabled, skip
                    continue;
                }
            }

            // Automatic endpoint discovery
            var handlers: Endpoint.RequestHandlers = .{};
            if (std.meta.hasFn(typ, "get")) {
                handlers.getHandler = try RequestHandler.init(auto, @field(typ, "get"));
            }
            if (std.meta.hasFn(typ, "post")) {
                handlers.postHandler = try RequestHandler.init(auto, @field(typ, "post"));
            }
            if (std.meta.hasFn(typ, "put")) {
                handlers.putHandler = try RequestHandler.init(auto, @field(typ, "put"));
            }
            if (std.meta.hasFn(typ, "patch")) {
                handlers.patchHandler = try RequestHandler.init(auto, @field(typ, "patch"));
            }
            if (std.meta.hasFn(typ, "delete")) {
                handlers.deleteHandler = try RequestHandler.init(auto, @field(typ, "delete"));
            }
            if (std.meta.hasFn(typ, "options")) {
                handlers.optionsHandler = try RequestHandler.init(auto, @field(typ, "options"));
            } else if (!@hasField(typ, "options") or @field(typ, "options")) {
                // Default `options` handler with CORS
                handlers.optionsHandler = try RequestHandler.init(auto, Endpoint.defaultOptionsHandler);
            }

            const error_handler: Endpoint.ErrorHandlerFn = if (std.meta.hasFn(typ, "sendErrorResponse")) @field(typ, "sendErrorResponse") else Endpoint.defaultErrorHandler;

            const ep = Endpoint.Endpoint.init(path, error_handler, handlers);
            try listener.register(&ep);
        }

        try listener.listen();
        std.log.info("Listening on 0.0.0.0:{}\n", .{listener.listener.settings.port});

        // Schedule any tasks before starting zap server

        // Created from a thread-safe allocator.
        const task_alloc = thread_safe_alloc.allocator();
        inline for (tasks) |task| blk: {
            if (std.meta.hasFn(task, "enabled")) {
                // Runtime enabled check
                const enabled_func: Endpoint.EnabledFn = task.enabled;
                if (!try enabled_func(.{ .env = &self.env_map, .alloc = global_arena.allocator() })) {
                    // Not enabled, skip
                    break :blk;
                }
            } else if (@hasField(task, "enabled")) {
                // Compile-time enabled check
                if (!@field(task, "enabled")) {
                    // Not enabled, skip
                    continue;
                }
            }
            try task.submit(task_alloc);
        }

        zap.start(.{ .threads = self.opts.threads, .workers = self.opts.workers });
    }
};

fn createListener(alloc: Allocator, port: u16) !Endpoint.Listener {
    const settings = zap.HttpListenerSettings{
        .port = port,
        .on_request = null,
        .log = true,
        .max_clients = 100000,
        .max_body_size = 100 * 1024 * 1024,
    };
    return Endpoint.Listener.init(alloc, settings);
}

test {
    // Required for `zig build test` to find all tests in src
    std.testing.refAllDecls(@This());
}

pub fn main() !void {
    // Example auto middleware
    const auto = @import("./middleware/auto.zig").auto;

    const alloc = std.heap.raw_c_allocator;
    var server: PowersServer = try PowersServer.init(alloc, .{
        .port = 3333,
        .threads = 2,
        // This has to be 1 (workers are _additional_ processes).
        // Multiple processes don't play well with a global database connection pool (see database.zig).
        .workers = 1,
    });

    const endpoints = [_]EndpointDef{
        .{ "/example", @import("endpoints/example.zig") },
    };
    const tasks = [_]type{};

    try server.run(&endpoints, &tasks, auto);
}
