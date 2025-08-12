const std = @import("std");
const builtin = @import("builtin");
const Allocator = std.mem.Allocator;

const Dir = std.fs.Dir;

const stdout = std.io.getStdOut().writer();

pub const DbInfo = struct {
    host: []const u8,
    port: u16,
    database: []const u8,
    username: []const u8,
    password: []const u8,
    migrations_dir: []const u8,
};

fn openMigrationsDir(path: []const u8) !Dir {
    if (std.fs.path.isAbsolute(path)) {
        const dir = std.fs.openDirAbsolute(path, .{}) catch |err| {
            if (err == error.FileNotFound) {
                try std.fs.makeDirAbsolute(path);
                return try std.fs.openDirAbsolute(path, .{});
            }
            return err;
        };
        return dir;
    }

    const cwd = std.fs.cwd();
    const dir = cwd.openDir(path, .{}) catch |err| {
        if (err == error.FileNotFound) {
            try cwd.makeDir(path);
            return try cwd.openDir(path, .{});
        }
        return err;
    };
    return dir;
}

pub fn migrateDatabase(info: DbInfo) !void {
    const dir = try openMigrationsDir(info.migrations_dir);
    try _migrate(dir);
}

pub fn newDatabaseMigration(info: DbInfo, file_name: []const u8) !void {
    const dir = try openMigrationsDir(info.migrations_dir);
    _ = dir;
    _ = file_name;
}

pub fn resetDatabase(info: DbInfo) !void {
    const dir = try openMigrationsDir(info.migrations_dir);
    // TODO:
    // 1. Check if we can connect to the database
    // 2. Check if the database exists
    // 3. Confirm with user
    // 4. Delete everything from the database

    // 5. Run try _migrate(dir);
    try _migrate(dir);
}

fn _migrate(dir: Dir) !void {
    _ = dir;
}

test {
    try migrateDatabase(.{
        .database = "postgres",
        .host = "localhost",
        .username = "username",
        .password = "password",
        .migrations_dir = ".migrations",
        .port = 3333,
    });
}
