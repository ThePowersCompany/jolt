// TODO: Can we put this file somewhere better?
const std = @import("std");
const DateTime = @import("./time.zig").DateTime;
const db = @import("../db/database.zig");
const Allocator = std.mem.Allocator;
const MICROS_PER_MILLI = @import("../utils/constants.zig").MICROS_PER_MILLI;
const events = @import("../endpoints/events.zig");
const PresenceStatus = @import("../endpoints/presence.zig").PresenceStatus;

const PresenceState = struct {
    prev_presence: PresenceStatus,
    presence: PresenceStatus,
    updated_at: DateTime,
};

pub const UserPresenceChange = struct {
    user: i32,
    status: PresenceStatus,
    updated_at: i64,
};

const duration = 5_000;

const PresenceMap = std.AutoHashMap(i32, PresenceState);
var alloc: Allocator = undefined;
var presence_map: PresenceMap = undefined;
var presence_map_lock: std.Thread.RwLock = .{};

pub fn init(allocator: Allocator) void {
    presence_map = PresenceMap.init(allocator);
    alloc = allocator;
}

pub fn deinit() void {
    presence_map.deinit();
}

pub fn setPresence(user_id: i32, new_presence: PresenceStatus) !void {
    const current_presence = p: {
        presence_map_lock.lockShared();
        defer presence_map_lock.unlockShared();
        break :p presence_map.get(user_id);
    };

    outer: switch (new_presence) {
        .ONLINE => {
            if (current_presence) |current| {
                if (current.prev_presence == .OFFLINE) {
                    const elapsed = DateTime.now().since(current.updated_at);
                    if (elapsed.ms < duration) {
                        // If they were offline for only a short time, we don't update their presence.
                        break :outer;
                    }
                }
            }
            try updateAndNotify(user_id, new_presence);
        },
        .OFFLINE => {},
        .IDLE => {
            try updateAndNotify(user_id, new_presence);
        },
    }
    try updateUserPresenceMap(user_id, new_presence);
}

fn updateAndNotify(user_id: i32, new_presence: PresenceStatus) !void {
    const updated_at = try updateUserPresenceTable(user_id, @tagName(new_presence));
    if (updated_at) |uat| {
        try events.notifyPresenceChanges(&.{.{
            .user = user_id,
            .status = new_presence,
            .updated_at = uat,
        }});
    }
}

fn updateUserPresenceMap(user_id: i32, new_presence: PresenceStatus) !void {
    presence_map_lock.lock();
    defer presence_map_lock.unlock();
    const old = presence_map.get(user_id);
    return presence_map.put(user_id, .{
        .prev_presence = if (old) |p| p.presence else .OFFLINE,
        .presence = new_presence,
        .updated_at = DateTime.now(),
    });
}

fn updateUserPresenceTable(user_id: i32, presence: []const u8) !?i64 {
    const sql =
        \\ MERGE INTO user_presence AS up
        \\ USING (VALUES
        \\   ($1::int, $2::user_activity_presence)
        \\ ) AS vals (user_id, presence)
        \\ ON up.user_id = vals.user_id
        \\ WHEN matched THEN
        \\   UPDATE SET
        \\     presence = vals.presence,
        \\     updated_at = NOW()
        \\ WHEN NOT matched THEN
        \\   INSERT (user_id, presence)
        \\   VALUES (vals.user_id, vals.presence)
        \\ RETURNING up.updated_at;
    ;

    const conn = try db.acquireConnection();
    defer conn.release();

    var result = conn.rowOpts(sql, .{
        user_id,
        presence,
    }, .{ .allocator = alloc }) catch |err| {
        return db.logError(err, conn);
    };
    if (result) |*row| {
        defer row.deinit() catch {};
        return @divTrunc(row.get(i64, 0), MICROS_PER_MILLI);
    } else {
        return null;
    }
}

/// Only to be called from an external task, which uses a different thread.
pub fn cleanupPresenceList() !void {
    // Calculate which users should be marked OFFLINE
    const new_offline_users = user_ids: {
        presence_map_lock.lock();
        defer presence_map_lock.unlock();

        if (presence_map.count() == 0) {
            return;
        }

        var user_ids = std.ArrayList(i32).init(alloc);
        errdefer user_ids.deinit();

        var iter = presence_map.iterator();
        while (iter.next()) |kv| {
            const presence = kv.value_ptr.*.presence;
            if (presence != .OFFLINE) continue;

            const updated_at = kv.value_ptr.*.updated_at;
            const elapsed = DateTime.now().since(updated_at);

            // Set user as offline if they've been disconnected for 5+ seconds.
            if (elapsed.ms >= duration) {
                try user_ids.append(kv.key_ptr.*);
            }
        }

        // Cleanup presence map
        for (user_ids.items) |user_id| {
            _ = presence_map.remove(user_id);
        }

        break :user_ids user_ids;
    };
    defer new_offline_users.deinit();

    if (new_offline_users.items.len == 0) return;

    const updated_at = try setUsersAsOffline(new_offline_users.items) orelse return;

    var changes = std.ArrayList(UserPresenceChange).init(alloc);
    defer changes.deinit();

    for (new_offline_users.items) |user_id| {
        try changes.append(.{
            .user = user_id,
            .status = .OFFLINE,
            .updated_at = updated_at,
        });
    }

    try events.notifyPresenceChanges(changes.items);
}

fn setUsersAsOffline(user_ids: []const i32) !?i64 {
    const sql =
        \\ UPDATE user_presence up
        \\ SET
        \\   presence = 'OFFLINE',
        \\   updated_at = NOW()
        \\ WHERE up.user_id = any($1)
        \\ RETURNING up.updated_at;
    ;

    const conn = try db.acquireConnection();
    defer conn.release();

    var result = conn.queryOpts(sql, .{user_ids}, .{ .allocator = alloc }) catch |err| {
        return db.logError(err, conn);
    };
    defer result.deinit();

    const row = try result.next() orelse return null;
    const updated_at = row.get(i64, 0);

    result.drain() catch {};

    return @divTrunc(updated_at, MICROS_PER_MILLI);
}
