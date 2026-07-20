const std = @import("std");
const dotenv = @import("dotenv");

const jolt = @import("jolt");
const JoltServer = jolt.JoltServer;
const EndpointDef = jolt.EndpointDef;

const builtin = @import("builtin");
const Allocator = std.mem.Allocator;
const ArenaAllocator = std.heap.ArenaAllocator;

pub const endpoints = [_]EndpointDef{
    .{ "/ping", @import("endpoints/ping.zig") },

    // Test endpoints
    .{ "/any-of", @import("endpoints/any-of.zig") },
    .{ "/gated-group", @import("endpoints/gated-group.zig") },
    .{ "/nested-optional", @import("endpoints/nested-optional.zig") },
    .{ "/union-variant", @import("endpoints/union-variant.zig") },
    .{ "/nullable", @import("endpoints/nullable.zig") },
    .{ "/arrays", @import("endpoints/arrays.zig") },
    .{ "/param-parse", @import("endpoints/param-parse.zig") },
};

const tasks = [_]type{};

fn healthCheck(alloc: Allocator) !void {
    var env_map = try std.process.getEnvMap(alloc);
    defer env_map.deinit();

    const port = env_map.get("SERVER_PORT") orelse "3333";

    var client = std.http.Client{ .allocator = alloc };
    defer client.deinit();

    const url = try std.fmt.allocPrint(alloc, "http://127.0.0.1:{s}/ping", .{port});
    defer alloc.free(url);

    const res = try client.fetch(.{
        .location = std.http.Client.FetchOptions.Location{
            .url = url,
        },
    });

    if (res.status != .ok) {
        return error.Unhealthy;
    }
}

pub fn main() !void {
    const args = std.os.argv;
    if (args.len > 1) {
        if (std.mem.eql(u8, std.mem.sliceTo(args[1], 0), "--health")) {
            return try healthCheck(std.heap.c_allocator);
        }
    }

    std.log.info("Starting server in {} mode\n", .{builtin.mode});

    if (builtin.mode == .Debug) {
        var gpa: std.heap.DebugAllocator(.{
            .thread_safe = true,
            .stack_trace_frames = 100,
        }) = .init;

        const alloc = gpa.allocator();

        std.log.info("Generating TypeScript types...", .{});
        try jolt.generateTypesFile(alloc, "types.generated.d.ts", &endpoints);
        std.log.info("Done.\n", .{});

        try runServer(alloc);

        // Show potential memory leaks when Zap shuts down
        const memory_leak = gpa.detectLeaks();
        if (memory_leak) {
            std.log.err("Memory leak!\n", .{});
        }
    } else {
        // Release/Production mode
        try runServer(std.heap.c_allocator);
    }

    std.log.info("Server shutdown.\n", .{});
}

fn runServer(alloc: Allocator) !void {
    dotenv.loadFrom(alloc, ".env", .{}) catch {
        // Ignore if .env file doesn't exist
    };

    var server: JoltServer = try JoltServer.init(alloc, .{
        .port = 3333,
        .threads = 32,
    });
    defer server.deinit();

    // Default = Same Origin
    server.cors = server.getEnvBool("ENABLE_CORS", false);

    try server.run(
        &endpoints,
        &tasks,
        jolt.middleware.auto,
    );
}

test {
    // Required for `zig build test` to find all tests in src
    std.testing.refAllDecls(@This());
}
