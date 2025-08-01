const std = @import("std");
const Allocator = std.mem.Allocator;

const task_utils = @import("../main.zig").task_utils;
const scheduleArenaTask = task_utils.scheduleArenaTask;
const logFmt = task_utils.logFmt;
const log = task_utils.log;

pub fn submit(alloc: Allocator) !void {
    try scheduleArenaTask(alloc, 1000, 0, true, task, null);
}

const try_count = 3;

pub fn task(arena_alloc: *Allocator) void {
    const alloc = arena_alloc.*;
    for (0..try_count) |i| {
        _task() catch |err| {
            if (i == try_count - 1) {
                logFmt(alloc, .err, "{} - Giving up!", .{err}) catch |e| {
                    std.log.err("Failed to set users as offline: {}", .{e});
                };
            } else {
                logFmt(alloc, .err, "{} - Retry #{}...", .{ err, i + 1 }) catch |e| {
                    std.log.err("Failed to set users as offline: {}", .{e});
                };
            }
            continue;
        };
        break;
    }
}

fn _task() !void {
    std.log.err("Running task!", .{});
}
