const std = @import("std");
const Allocator = std.mem.Allocator;
const ArenaAllocator = std.heap.ArenaAllocator;

const c = @cImport({
    @cInclude("fio.h");
});

pub fn cast(comptime T: type, ptr: ?*anyopaque) T {
    return @ptrCast(@alignCast(ptr));
}

fn wrap(T: type, comptime task: *const fn (t: *T) void) *const fn (?*anyopaque) callconv(.C) void {
    return struct {
        fn wrapped(arg: ?*anyopaque) callconv(.C) void {
            task(cast(*T, arg));
        }
    }.wrapped;
}

fn wrapArena(comptime task: *const fn (arena: *Allocator) void) *const fn (*Allocator) void {
    return struct {
        fn wrapped(arg: *Allocator) void {
            const alloc = cast(*Allocator, arg);
            var arena = ArenaAllocator.init(alloc.*);
            defer arena.deinit();
            var aa = arena.allocator();
            task(&aa);
        }
    }.wrapped;
}

fn wrapNoContext(comptime task: *const fn () void) *const fn (?*anyopaque) callconv(.C) void {
    return struct {
        fn wrapped(_: ?*anyopaque) callconv(.C) void {
            task();
        }
    }.wrapped;
}

fn wrap2(T: type, comptime task: *const fn (t: *T) void) *const fn (?*anyopaque, ?*anyopaque) callconv(.C) void {
    return struct {
        fn wrapped(arg: ?*anyopaque, _: ?*anyopaque) callconv(.C) void {
            task(cast(*T, arg));
        }
    }.wrapped;
}

fn wrap2NoContext(comptime task: *const fn () void) *const fn (?*anyopaque, ?*anyopaque) callconv(.C) void {
    return struct {
        fn wrapped(_: ?*anyopaque, _: ?*anyopaque) callconv(.C) void {
            task();
        }
    }.wrapped;
}

/// Schedules a task to be ran after `interval_millis`, `run_count` number of times.
/// `T` is a context type that may be passed to `task` and `on_finish`.
/// If `run_count` is `0`, the task will repeat forever.
/// `on_finish` is invoked when the last task is ran, or when the parent process exits.
pub fn scheduleTask(
    T: type,
    t: *T,
    interval_millis: u64,
    run_count: usize,
    run_immediately: bool,
    comptime task: *const fn (t: *T) void,
    comptime on_finish: ?*const fn (t: *T) void,
) !void {
    // See https://facil.io/0.7.x/fio#event-task-scheduling

    // If running immediately, subtract 1 from the number of times we'll run the task in a loop.
    const runs = if (run_immediately and run_count > 0) run_count - 1 else run_count;

    // Don't schedule the task loop if the user intends to run the task only once, immediately.
    if (!(run_immediately and run_count == 1)) {
        const result = c.fio_run_every(
            interval_millis,
            runs,
            wrap(T, task),
            t,
            if (on_finish) |f| wrap(T, f) else null,
        );
        if (result == -1) return error.FailedToScheduleTask;
    }

    if (run_immediately) {
        const result = c.fio_defer(wrap2(T, task), t, null);
        if (result == -1) return error.FailedToScheduleTask;
    }
}

/// Schedules a task to be ran after `interval_millis`, `run_count` number of times.
/// If `run_count` is `0`, the task will repeat forever.
/// `on_finish` is invoked when the last task is ran, or when the parent process exits.
pub fn scheduleArenaTask(
    alloc: Allocator,
    interval_millis: u64,
    run_count: usize,
    run_immediately: bool,
    comptime task: *const fn (arena: *Allocator) void,
    comptime on_finish: ?*const fn (arena: *Allocator) void,
) !void {
    try scheduleTask(
        Allocator,
        @ptrCast(@alignCast(alloc.ptr)),
        interval_millis,
        run_count,
        run_immediately,
        wrapArena(task),
        if (on_finish) |f| wrapArena(f) else null,
    );
}

/// Schedules a task to be ran after `interval_millis`, `run_count` number of times.
/// If `run_count` is `0`, the task will repeat forever.
/// `on_finish` is invoked when the last task is ran, or when the parent process exits.
pub fn scheduleSimpleTask(
    interval_millis: u64,
    run_count: usize,
    run_immediately: bool,
    comptime task: *const fn () void,
    comptime on_finish: ?*const fn () void,
) !void {
    // See https://facil.io/0.7.x/fio#event-task-scheduling

    // If running immediately, subtract 1 from the number of times we'll run the task in a loop.
    const runs = if (run_immediately and run_count > 0) run_count - 1 else run_count;

    // Don't schedule the task loop if the user intends to run the task only once, immediately.
    if (!(run_immediately and run_count == 1)) {
        const result = c.fio_run_every(
            interval_millis,
            runs,
            wrapNoContext(task),
            null,
            if (on_finish) |f| wrapNoContext(f) else null,
        );
        if (result == -1) return error.FailedToScheduleTask;
    }

    if (run_immediately) {
        const result = c.fio_defer(wrap2NoContext(task), null, null);
        if (result == -1) return error.FailedToScheduleTask;
    }
}
