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

// NOTE: Our largest migration file atm is 9.3k, this is fine I guess?
const max_migration_file_size = 4096 * 4;

pub const DbInfo = struct {
    host: []const u8,
    port: u16,
    database: []const u8,
    username: []const u8,
    password: []const u8,
    migrations_dir: []const u8,
    migrations_table: []const u8 = "_prisma_migrations",
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

    var entries = ArrayList(Entry).init(alloc);
    defer entries.deinit();

    var iterator = dir.iterate();
    while (try iterator.next()) |entry| {
        if (entry.kind != .directory) continue;

        try entries.append(.{
            .kind = .directory,
            .name = try alloc.dupe(u8, entry.name),
        });
    }
    std.sort.pdq(Entry, entries.items, {}, compareEntries);
    return .{
        .dir = dir,
        .entries = try entries.toOwnedSlice(),
    };
}

fn compareEntries(_: void, a: std.fs.Dir.Entry, b: std.fs.Dir.Entry) bool {
    return std.mem.order(u8, a.name, b.name) == .lt;
}

pub fn migrateDatabase(alloc: Allocator, info: DbInfo) !void {
    const dir = try loadMigrationDir(alloc, info.migrations_dir);
    defer {
        for (dir.entries) |e| alloc.free(e.name);
        defer alloc.free(dir.entries);
    }

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

    _ = conn.execOpts(
        \\ DROP SCHEMA if exists public CASCADE;
        \\ CREATE SCHEMA public;
        \\ GRANT ALL ON SCHEMA public TO postgres;
        \\ GRANT ALL ON SCHEMA public TO public;
        \\ CREATE TABLE IF NOT EXISTS public._prisma_migrations
        \\ (
        \\   id character varying(36) COLLATE pg_catalog."default" NOT NULL,
        \\   checksum character varying(64) COLLATE pg_catalog."default" NOT NULL,
        \\   finished_at timestamp with time zone,
        \\   migration_name character varying(255) COLLATE pg_catalog."default" NOT NULL,
        \\   logs text COLLATE pg_catalog."default",
        \\   rolled_back_at timestamp with time zone,
        \\   started_at timestamp with time zone NOT NULL DEFAULT now(),
        \\   applied_steps_count integer NOT NULL DEFAULT 0,
        \\   CONSTRAINT _prisma_migrations_pkey PRIMARY KEY (id)
        \\ )
        \\ TABLESPACE pg_default;
        \\ ALTER TABLE IF EXISTS public._prisma_migrations
        \\     OWNER to postgres;
    ,
        .{},
        .{ .allocator = alloc },
    ) catch |err| return db.logError(err, conn);

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
    const migration_entries = try queryMigrationsTable(alloc, conn, info);
    defer alloc.free(migration_entries);

    std.log.err("Found {} migrations already applied", .{migration_entries.len});

    for (dir.entries) |dir_entry| {
        std.log.err("Checking {s}...", .{dir_entry.name});

        // Construct path e.g. 20250618211026_foo_bar/migration.sql
        const file_path = try std.fmt.allocPrint(
            alloc,
            "{s}{s}migration.sql",
            .{ dir_entry.name, std.fs.path.sep_str },
        );
        defer alloc.free(file_path);

        const file = try dir.dir.openFile(file_path, .{});
        const sql = try file.readToEndAlloc(alloc, max_migration_file_size);
        defer alloc.free(sql);

        // Insert new row in migrations table
        const checksum = hash(sql);
        if (findMigrationEntry(migration_entries, dir_entry.name)) |migration| {
            std.log.err("Verifying checksum...", .{});
            if (!strEql(migration.checksum, &checksum)) {
                std.log.err("Checksum mismatch for {s}!", .{migration.checksum});
                return error.ChecksumMismatch;
            }
        } else {
            // New migration to apply
            std.log.err("New migration found, applying...", .{});
            try executeSql(alloc, conn, sql);
            try insertMigrationRow(alloc, conn, info.migrations_table, dir_entry.name, checksum);
        }
        std.log.err("Done.", .{});
    }
}

/// Creates a 64 character hexadecimal string of a checksum of the provided string.
fn hash(sql: []const u8) [64]u8 {
    var hasher = std.crypto.hash.sha2.Sha256.init(.{});
    hasher.update(sql);

    var checksum: [32]u8 = undefined;
    hasher.final(&checksum);
    var hex_string: [64]u8 = undefined;
    _ = std.fmt.bufPrint(&hex_string, "{}", .{std.fmt.fmtSliceHexLower(&checksum)}) catch unreachable;
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
    std.debug.print("Successfully executed SQL:\n{s}\n\n", .{sql});
}

fn queryMigrationsTable(alloc: Allocator, conn: *pg.Conn, info: DbInfo) ![]MigrationEntry {
    const sql = try std.fmt.allocPrint(alloc, "SELECT * FROM {s};", .{info.migrations_table});
    defer alloc.free(sql);

    var result = conn.queryOpts(sql, .{}, .{ .allocator = alloc }) catch |err| return db.logError(err, conn);
    defer result.deinit();

    var migration_entries = ArrayList(MigrationEntry).init(alloc);
    defer migration_entries.deinit();

    while (try result.next()) |row| {
        const entry = try row.to(MigrationEntry, .{ .allocator = alloc });
        try migration_entries.append(entry);
    }
    return try migration_entries.toOwnedSlice();
}

fn findMigrationEntry(entries: []MigrationEntry, file_name: []const u8) ?MigrationEntry {
    for (entries) |migration| {
        if (strEql(migration.migration_name, file_name)) return migration;
    }
    return null;
}
