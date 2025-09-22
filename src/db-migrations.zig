const std = @import("std");
const builtin = @import("builtin");
const Allocator = std.mem.Allocator;
const ArrayList = std.ArrayList;

const Dir = std.fs.Dir;
const Entry = Dir.Entry;

const pg = @import("pg");
const db = @import("db/database.zig");

const DateTime = @import("utils/time.zig").DateTime;

const UUID = @import("utils/uuid.zig").UUID;

pub const DbInfo = struct {
    host: []const u8,
    port: u16,
    database: []const u8,
    username: []const u8,
    password: []const u8,
    migrations_dir: []const u8,
    migrations_table: []const u8 = "_migrations",
};

const MigrationEntry = struct {
    id: []const u8,
    checksum: []const u8,
    migration_name: []const u8,
};

const MigrationDir = struct { dir: Dir, entries: []Entry };

fn strEql(s1: []const u8, s2: []const u8) bool {
    return std.mem.eql(u8, s1, s2);
}

fn findMigrationDir(path: []const u8) !Dir {
    if (std.fs.path.isAbsolute(path)) {
        return std.fs.openDirAbsolute(path, .{ .iterate = true }) catch |err| {
            if (err == error.FileNotFound) {
                try std.fs.makeDirAbsolute(path);
                return try std.fs.openDirAbsolute(path, .{ .iterate = true });
            }
            return err;
        };
    }

    const cwd = std.fs.cwd();
    return cwd.openDir(path, .{ .iterate = true }) catch |err| {
        if (err == error.FileNotFound) {
            try cwd.makeDir(path);
            return try cwd.openDir(path, .{ .iterate = true });
        }
        return err;
    };
}

fn loadMigrationDir(alloc: Allocator, path: []const u8) !MigrationDir {
    const dir = try findMigrationDir(path);

    var entries: ArrayList(Entry) = .empty;
    defer entries.deinit(alloc);

    var iterator = dir.iterate();
    while (try iterator.next()) |entry| {
        if (entry.kind != .directory) continue;

        try entries.append(alloc, .{
            .kind = .directory,
            .name = try alloc.dupe(u8, entry.name),
        });
    }
    std.sort.pdq(Entry, entries.items, {}, compareEntries);
    return .{
        .dir = dir,
        .entries = try entries.toOwnedSlice(alloc),
    };
}

fn compareEntries(_: void, a: std.fs.Dir.Entry, b: std.fs.Dir.Entry) bool {
    return std.mem.order(u8, a.name, b.name) == .lt;
}

fn dropMigrationsTable(alloc: Allocator, conn: *pg.Conn) !void {
    try executeSql(alloc, conn, "DROP SCHEMA IF EXISTS public CASCADE;");
}

fn ensureMigrationsTable(alloc: Allocator, conn: *pg.Conn, info: DbInfo) !void {
    const sql = try std.fmt.allocPrint(
        alloc,
        \\ CREATE SCHEMA IF NOT EXISTS public;
        \\ GRANT ALL ON SCHEMA public TO {s};
        \\ CREATE TABLE IF NOT EXISTS public.{s} (
        \\   id character varying(36) NOT NULL PRIMARY KEY,
        \\   checksum character varying(64) NOT NULL,
        \\   finished_at timestamp with time zone,
        \\   migration_name character varying(255) NOT NULL,
        \\   logs text,
        \\   rolled_back_at timestamp with time zone,
        \\   started_at timestamp with time zone NOT NULL DEFAULT NOW(),
        \\   applied_steps_count integer NOT NULL DEFAULT 0
        \\ )
        \\ TABLESPACE pg_default;
        \\ ALTER TABLE IF EXISTS public.{s} OWNER TO {s};
    ,
        .{
            info.username,
            info.migrations_table,
            info.migrations_table,
            info.username,
        },
    );
    defer alloc.free(sql);

    try executeSql(alloc, conn, sql);
}

pub fn migrateDatabase(alloc: Allocator, info: DbInfo) !void {
    const dir = try loadMigrationDir(alloc, info.migrations_dir);
    defer {
        for (dir.entries) |e| alloc.free(e.name);
        defer alloc.free(dir.entries);
    }

    try initDbConnectionPool(alloc, info);
    defer db.deinit();

    const conn = try db.acquireConnection();
    defer conn.release();
    try _migrate(alloc, conn, dir, info);
}

pub fn newDatabaseMigration(alloc: Allocator, file_name: []const u8, info: DbInfo) !void {
    const dir = try loadMigrationDir(alloc, info.migrations_dir);
    defer {
        for (dir.entries) |e| alloc.free(e.name);
        defer alloc.free(dir.entries);
    }

    // Construct path e.g. 20250618211026_foo_bar/migration.sql
    const now_str = try DateTime.now().formatAlloc(alloc, "YYYYMMDDHHmmss");
    defer alloc.free(now_str);

    const migration_dir_name = try std.fmt.allocPrint(alloc, "{s}_{s}", .{ now_str, file_name });
    defer alloc.free(migration_dir_name);

    try dir.dir.makeDir(migration_dir_name);

    const file_path = try std.fmt.allocPrint(
        alloc,
        "{s}{s}migration.sql",
        .{ migration_dir_name, std.fs.path.sep_str },
    );
    defer alloc.free(file_path);

    _ = try dir.dir.createFile(file_path, .{});

    std.log.info("Created file: {s}{s}{s}", .{ info.migrations_dir, std.fs.path.sep_str, file_path });
}

pub fn resetDatabase(alloc: Allocator, info: DbInfo) !void {
    const dir = try loadMigrationDir(alloc, info.migrations_dir);
    defer {
        for (dir.entries) |e| alloc.free(e.name);
        defer alloc.free(dir.entries);
    }

    try initDbConnectionPool(alloc, info);
    defer db.deinit();

    const conn: *pg.Conn = try db.acquireConnection();
    defer conn.release();

    try dropMigrationsTable(alloc, conn, info);
    try _migrate(alloc, conn, dir, info);
}

fn initDbConnectionPool(alloc: Allocator, info: DbInfo) !void {
    try db.init(alloc, .{
        .host = info.host,
        .port = info.port,
        .database = info.database,
        .username = info.username,
        .password = info.password,
        .pool_size = 1,
    });
}

fn _migrate(alloc: Allocator, conn: *pg.Conn, dir: MigrationDir, info: DbInfo) !void {
    try ensureMigrationsTable(alloc, conn, info);

    const migration_entries = try queryMigrationsTable(alloc, conn, info);
    defer {
        for (migration_entries) |entry| {
            alloc.free(entry.id);
            alloc.free(entry.checksum);
            alloc.free(entry.migration_name);
        }
        alloc.free(migration_entries);
    }

    std.debug.print("Found {} migrations already applied\n", .{migration_entries.len});

    for (dir.entries) |dir_entry| {
        std.debug.print("Checking {s}...\n", .{dir_entry.name});

        // Construct path e.g. 20250618211026_foo_bar/migration.sql
        const file_path = try std.fmt.allocPrint(
            alloc,
            "{s}{s}migration.sql",
            .{ dir_entry.name, std.fs.path.sep_str },
        );
        defer alloc.free(file_path);

        const file = try dir.dir.openFile(file_path, .{});
        defer file.close();

        const sql = try alloc.alloc(u8, (try file.stat()).size);
        defer alloc.free(sql);
        _ = try file.read(sql);

        // Insert new row in migrations table
        const checksum = hash(sql);
        if (findMigrationEntry(migration_entries, dir_entry.name)) |migration| {
            std.debug.print("Verifying checksum...", .{});
            if (!strEql(migration.checksum, &checksum)) {
                std.debug.print("\nChecksum mismatch for {s}!\n", .{migration.checksum});
                return error.ChecksumMismatch;
            }
        } else {
            // New migration to apply
            std.debug.print("New migration found, applying...", .{});
            try executeSql(alloc, conn, sql);
            try insertMigrationRow(alloc, conn, info.migrations_table, dir_entry.name, checksum);
        }
        std.debug.print("Done.\n\n", .{});
    }
}

/// Creates a 64 character hexadecimal string of a checksum of the provided string.
fn hash(sql: []const u8) [64]u8 {
    var hasher = std.crypto.hash.sha2.Sha256.init(.{});
    hasher.update(sql);

    var checksum: [32]u8 = undefined;
    hasher.final(&checksum);
    var hex_string: [64]u8 = undefined;
    _ = std.fmt.bufPrint(&hex_string, "{x}", .{checksum}) catch unreachable;
    return hex_string;
}

fn insertMigrationRow(
    alloc: Allocator,
    conn: *pg.Conn,
    migrations_table: []const u8,
    migration_name: []const u8,
    file_checksum: [64]u8,
) !void {
    const sql = try std.fmt.allocPrint(alloc,
        \\ INSERT INTO {s} (id, migration_name, checksum, applied_steps_count, started_at, finished_at)
        \\ VALUES ($1, $2, $3, 1, NOW(), NOW());
    , .{migrations_table});
    defer alloc.free(sql);

    const id: [36]u8 = UUID.v7().toHex(.lower);
    _ = conn.execOpts(
        sql,
        .{ id, migration_name, file_checksum },
        .{ .allocator = alloc },
    ) catch |err| return db.logError(err, conn);
}

fn executeSql(alloc: Allocator, conn: *pg.Conn, sql: []const u8) !void {
    _ = conn.execOpts(
        sql,
        .{},
        .{ .allocator = alloc },
    ) catch |err| return db.logError(err, conn);
}

fn queryMigrationsTable(alloc: Allocator, conn: *pg.Conn, info: DbInfo) ![]MigrationEntry {
    const sql = try std.fmt.allocPrint(
        alloc,
        "SELECT id, checksum, migration_name FROM {s};",
        .{info.migrations_table},
    );
    defer alloc.free(sql);

    var result = conn.queryOpts(sql, .{}, .{ .allocator = alloc }) catch |err| return db.logError(err, conn);
    defer result.deinit();

    var migration_entries: ArrayList(MigrationEntry) = .empty;
    defer migration_entries.deinit(alloc);

    while (try result.next()) |row| {
        const entry = try row.to(MigrationEntry, .{ .allocator = alloc });
        try migration_entries.append(alloc, entry);
    }
    return try migration_entries.toOwnedSlice(alloc);
}

fn findMigrationEntry(entries: []MigrationEntry, file_name: []const u8) ?MigrationEntry {
    for (entries) |migration| {
        if (strEql(migration.migration_name, file_name)) return migration;
    }
    return null;
}
