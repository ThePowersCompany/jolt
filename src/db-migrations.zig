const std = @import("std");
const builtin = @import("builtin");
const Allocator = std.mem.Allocator;
const ArrayList = std.ArrayList;

const Dir = std.Io.Dir;

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

const MigrationDir = struct { dir: Dir, entries: []const []const u8 };

fn strEql(s1: []const u8, s2: []const u8) bool {
    return std.mem.eql(u8, s1, s2);
}

fn findMigrationDir(io: std.Io, path: []const u8) !Dir {
    if (std.fs.path.isAbsolute(path)) {
        return Dir.openDirAbsolute(io, path, .{ .iterate = true }) catch |err| {
            if (err == error.FileNotFound) {
                try Dir.createDirAbsolute(io, path, .default_dir);
                return try Dir.openDirAbsolute(io, path, .{ .iterate = true });
            }
            return err;
        };
    }

    const cwd = Dir.cwd();
    return cwd.openDir(io, path, .{ .iterate = true }) catch |err| {
        if (err == error.FileNotFound) {
            try cwd.createDir(io, path, .default_dir);
            return try cwd.openDir(io, path, .{ .iterate = true });
        }
        return err;
    };
}

fn loadMigrationDir(alloc: Allocator, io: std.Io, path: []const u8) !MigrationDir {
    const dir = try findMigrationDir(io, path);

    var entries: ArrayList([]const u8) = .empty;
    defer entries.deinit(alloc);

    var iterator = dir.iterate();
    while (try iterator.next(io)) |entry| {
        if (entry.kind != .directory) continue;

        try entries.append(alloc, try alloc.dupe(u8, entry.name));
    }
    std.sort.pdq([]const u8, entries.items, {}, compareEntries);
    return .{
        .dir = dir,
        .entries = try entries.toOwnedSlice(alloc),
    };
}

fn compareEntries(_: void, a: []const u8, b: []const u8) bool {
    return std.mem.order(u8, a, b) == .lt;
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

pub fn migrateDatabase(alloc: Allocator, io: std.Io, info: DbInfo) !void {
    const dir = try loadMigrationDir(alloc, io, info.migrations_dir);
    defer {
        for (dir.entries) |name| alloc.free(name);
        defer alloc.free(dir.entries);
    }

    try initDbConnectionPool(alloc, io, info);
    defer db.deinit();

    const conn = try db.acquireConnection();
    defer conn.release();
    try _migrate(alloc, io, conn, dir, info);
}

pub fn newDatabaseMigration(alloc: Allocator, io: std.Io, file_name: []const u8, info: DbInfo) !void {
    const dir = try loadMigrationDir(alloc, io, info.migrations_dir);
    defer {
        for (dir.entries) |name| alloc.free(name);
        defer alloc.free(dir.entries);
    }

    // Construct path e.g. 20250618211026_foo_bar/migration.sql
    const now_str = try DateTime.now().formatAlloc(alloc, "YYYYMMDDHHmmss");
    defer alloc.free(now_str);

    const migration_dir_name = try std.fmt.allocPrint(alloc, "{s}_{s}", .{ now_str, file_name });
    defer alloc.free(migration_dir_name);

    try dir.dir.createDir(io, migration_dir_name, .default_dir);

    const file_path = try std.fmt.allocPrint(
        alloc,
        "{s}{s}migration.sql",
        .{ migration_dir_name, std.fs.path.sep_str },
    );
    defer alloc.free(file_path);

    const file = try dir.dir.createFile(io, file_path, .{});
    file.close(io);

    std.log.info("Created file: {s}{s}{s}", .{ info.migrations_dir, std.fs.path.sep_str, file_path });
}

pub fn resetDatabase(alloc: Allocator, io: std.Io, info: DbInfo) !void {
    const dir = try loadMigrationDir(alloc, io, info.migrations_dir);
    defer {
        for (dir.entries) |name| alloc.free(name);
        defer alloc.free(dir.entries);
    }

    try initDbConnectionPool(alloc, io, info);
    defer db.deinit();

    const conn: *pg.Conn = try db.acquireConnection();
    defer conn.release();

    try executeSql(alloc, conn, "DROP SCHEMA IF EXISTS public CASCADE;");
    try _migrate(alloc, io, conn, dir, info);
}

fn initDbConnectionPool(alloc: Allocator, io: std.Io, info: DbInfo) !void {
    try db.init(io, alloc, .{
        .host = info.host,
        .port = info.port,
        .database = info.database,
        .username = info.username,
        .password = info.password,
        .pool_size = 1,
    });
}

fn _migrate(alloc: Allocator, io: std.Io, conn: *pg.Conn, dir: MigrationDir, info: DbInfo) !void {
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

    for (dir.entries) |dir_name| {
        std.debug.print("Checking {s}...\n", .{dir_name});

        // Construct path e.g. 20250618211026_foo_bar/migration.sql
        const file_path = try std.fmt.allocPrint(
            alloc,
            "{s}{s}migration.sql",
            .{ dir_name, std.fs.path.sep_str },
        );
        defer alloc.free(file_path);

        const sql = try dir.dir.readFileAlloc(io, file_path, alloc, .unlimited);
        defer alloc.free(sql);

        // Insert new row in migrations table
        const checksum = hash(sql);
        if (findMigrationEntry(migration_entries, dir_name)) |migration| {
            std.debug.print("Verifying checksum...", .{});
            if (!strEql(migration.checksum, &checksum)) {
                std.debug.print("\nChecksum mismatch for {s}!\n", .{migration.checksum});
                return error.ChecksumMismatch;
            }
        } else {
            // New migration to apply
            std.debug.print("New migration found, applying...", .{});
            try executeSql(alloc, conn, sql);
            try insertMigrationRow(alloc, conn, info.migrations_table, dir_name, checksum);
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
