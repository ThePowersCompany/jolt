const std = @import("std");
const builtin = @import("builtin");
const Allocator = std.mem.Allocator;
const ArenaAllocator = std.heap.ArenaAllocator;
const zap = @import("./zap/zap.zig");
const auth = @import("middleware/auth.zig");
const RequestHandler = Endpoint.RequestHandler;

const db_migrations = @import("db-migrations.zig");
const pg = @import("pg");

// Database
pub const database = @import("db/database.zig");
pub const migrateDatabase = db_migrations.migrateDatabase;
pub const newDatabaseMigration = db_migrations.newDatabaseMigration;
pub const resetDatabase = db_migrations.resetDatabase;
pub const DbInfo = db_migrations.DbInfo;
pub const DbConnection = pg.Conn;
pub const DbResult = pg.Result;
pub const DbRow = pg.Row;
pub const DbStatement = pg.Stmt;
pub const DbQueryRow = pg.QueryRow;
pub const DbListener = pg.Listener;

// Zap
pub const Request = zap.Request;
pub const Endpoint = zap.Endpoint;
pub const Response = Endpoint.Response;
pub const MiddlewareFn = Endpoint.MiddlewareFn;
pub const StatusCode = zap.StatusCode;
pub const mustache = @import("zap/mustache.zig");
pub const WebSockets = @import("zap/websockets.zig");
pub const util = @import("zap/util.zig");
pub const UnionRepr = @import("./middleware/parse-body.zig").UnionRepr;

// Utilities
pub const array = @import("./utils/array_utils.zig");
pub const datetime = @import("./utils/datetime.zig");
pub const email = @import("./utils/email.zig");
pub const err = @import("./utils/error.zig");
pub const json = @import("./utils/json.zig");
pub const jwt = @import("./utils/jwt.zig");
pub const mime = @import("./utils/mime.zig");
pub const password = @import("./utils/password.zig");
pub const task = @import("./utils/task.zig");
pub const time = @import("./utils/time.zig");
pub const types = @import("./utils/types.zig");
pub const uuid = @import("./utils/uuid.zig");

pub const generateTypesFile = @import("typegen.zig").generateTypesFile;

pub const middleware = struct {
    pub const cors = @import("./middleware/cors.zig").cors;
    pub const parseBody = @import("./middleware/parse-body.zig").parseBody;
    pub const parseQueryParams = @import("./middleware/parse-query-params.zig").parseQueryParams;
};

pub const ServerOpts = struct {
    port: u16,
    threads: u16,
};

pub const EndpointDef = struct { []const u8, type };

pub const JoltServer = struct {
    const Self = @This();

    alloc: Allocator,
    opts: ServerOpts,
    env_map: std.process.EnvMap,

    /// Enable Cross-Origin Requests
    /// By default, only requests from the same host/port are allowed.
    cors: bool = false,

    pub fn init(alloc: Allocator, opts: ServerOpts) !Self {
        return .{
            .alloc = alloc,
            .opts = opts,
            .env_map = try std.process.getEnvMap(alloc),
        };
    }

    pub fn deinit(self: *Self) void {
        self.env_map.deinit();
    }

    pub fn getEnv(self: *Self, comptime key: []const u8, comptime default: []const u8) []const u8 {
        return self.env_map.get(key) orelse default;
    }

    pub fn getEnvOrPanic(self: *Self, comptime key: []const u8) []const u8 {
        return Endpoint.EnabledContext.getEnvOrPanic(&self.env_map, key);
    }

    pub fn getEnvBool(self: *Self, comptime key: []const u8, comptime default: bool) bool {
        const val = self.env_map.get(key) orelse return default;
        return std.ascii.eqlIgnoreCase(val, "true") or std.ascii.eqlIgnoreCase(val, "yes") or std.ascii.eqlIgnoreCase(val, "1");
    }

    pub fn getEnvInt(self: *Self, comptime T: type, comptime key: []const u8, comptime default: T) !T {
        const val = self.env_map.get(key) orelse return default;
        return std.fmt.parseInt(T, val, 10);
    }

    pub fn run(
        self: *Self,
        endpoints: []const EndpointDef,
        tasks: []const type,
        auto: anytype,
    ) !void {
        @setEvalBranchQuota((endpoints.len + tasks.len) * 1000);
        var global_arena = ArenaAllocator.init(self.alloc);
        defer global_arena.deinit();
        var thread_safe_alloc = std.heap.ThreadSafeAllocator{ .child_allocator = self.alloc };

        var listener = Endpoint.Listener.init(thread_safe_alloc.allocator(), .{
            .port = self.opts.port,
            .on_request = null,
            .log = true,
            .max_clients = 100000,
            .max_body_size = 100 * 1024 * 1024,
        });
        defer listener.deinit();

        const alloc = global_arena.allocator();
        var deinitFns: std.ArrayList(*const fn () void) = .empty;
        defer {
            for (deinitFns.items) |f| f();
            deinitFns.deinit(alloc);
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

            if (std.meta.hasFn(typ, "init") and std.meta.hasFn(typ, "deinit")) {
                // Legacy init/deinit
                try typ.init(thread_safe_alloc.allocator(), self, &listener, path);
                try deinitFns.append(alloc, typ.deinit);
            } else {

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

                const ep = Endpoint.Endpoint.init(self, path, error_handler, handlers);
                try listener.register(&ep);
            }
        }

        try listener.listen();
        std.log.info("Listening on 0.0.0.0:{}\n", .{listener.listener.settings.port});

        // Schedule any tasks before starting zap server

        // Created from a thread-safe allocator.
        const task_alloc = thread_safe_alloc.allocator();

        inline for (tasks) |t| blk: {
            if (std.meta.hasFn(t, "enabled")) {
                // Runtime enabled check
                const enabled_func: Endpoint.EnabledFn = t.enabled;
                if (!try enabled_func(.{ .env = &self.env_map, .alloc = global_arena.allocator() })) {
                    // Not enabled, skip
                    break :blk;
                }
            } else if (@hasField(t, "enabled")) {
                // Compile-time enabled check
                if (!@field(t, "enabled")) {
                    // Not enabled, skip
                    continue;
                }
            }
            try t.submit(task_alloc);
        }

        // This has to be 1 (workers are _additional_ processes).
        // Multiple processes don't play well with a global database connection pool (see database.zig).
        zap.start(.{ .threads = @intCast(self.opts.threads), .workers = 1 });
    }
};

pub fn main() !void {

    // Example auto middleware
    const auto = @import("./middleware/auto.zig").auto;

    const alloc = std.heap.raw_c_allocator;
    var server: JoltServer = try JoltServer.init(alloc, .{
        .port = 3333,
        .threads = 2,
    });

    const endpoints = [_]EndpointDef{
        .{ "/example", @import("endpoints/example.zig") },
    };
    const tasks = [_]type{
        @import("tasks/example_task.zig"),
    };

    try generateTypesFile(alloc, "types.d.ts", &endpoints);

    try server.run(&endpoints, &tasks, auto);
}

test {
    // Required for `zig build test` to find all tests in src
    std.testing.refAllDecls(@This());
}
