const std = @import("std");
const Allocator = std.mem.Allocator;
const allocPrint = std.fmt.allocPrint;

const pg = @import("pg");
const db = @import("../db/database.zig");

const types = @import("types.zig");
const Optional = types.Optional;
const isOptional = types.isOptional;

/// Generates a dynamic UPDATE query for fields marked as .value in the params.
/// Only fields where the parameter is not .not_provided are included in the SET clause.
/// The query must be completed by the caller with a WHERE clause and executed via bindAndExecute.
///
/// @param alloc - Allocator for SQL string construction
/// @param table - The table name to update
/// @param fields - Tuple of .{key, expr, param} triples
///   (e.g. .{.{"name", "TRIM($$)", name_param}, .{"age", "$$", age_param}})
/// @param param_offset - Starting parameter number for WHERE clauses (e.g. 1 or 2)
/// @param conn - The database connection
///
/// @returns A PatchQuery that can be extended with WHERE clauses and executed
pub fn patchQuery(
    alloc: Allocator,
    table: []const u8,
    fields: anytype,
    param_offset: usize,
    conn: anytype,
) !PatchQuery(@TypeOf(fields), @TypeOf(conn)) {
    var parameter_count: usize = 0;
    inline for (fields) |f| {
        const param = f[2];
        if (comptime isOptional(@TypeOf(param))) {
            if (param == .value) parameter_count += 1;
        } else {
            parameter_count += 1;
        }
    }
    if (parameter_count == 0) return error.NoParams;

    var sql: std.ArrayList(u8) = .empty;
    errdefer sql.deinit(alloc);

    try sql.print(alloc, "UPDATE {s} SET ", .{table});

    var param_num: usize = 1;
    inline for (fields) |f| {
        const key = f[0];
        const expr = f[1];
        const param = f[2];
        if ((comptime !isOptional(@TypeOf(param))) or param == .value) {
            const subbed = try subPlaceholders(alloc, expr, param_offset + param_num);
            defer alloc.free(subbed);
            try sql.print(alloc, "{s} = {s}", .{ key, subbed });
            if (param_num < parameter_count) try sql.appendSlice(alloc, ", ");
            param_num += 1;
        }
    }

    return .{
        ._alloc = alloc,
        ._conn = conn,
        .sql = sql,
        .params = fields,
    };
}

/// Generates an INSERT ... ON CONFLICT DO UPDATE SET (upsert) query.
/// Automatically distinguishes fixed vs dynamic fields based on whether params are Optional types.
/// Non-Optional params are fixed (always included), Optional params are dynamic (included only if .value).
/// The ON CONFLICT UPDATE SET clause automatically includes all fields except those in conflict_columns.
///
/// @param alloc - Allocator for SQL string construction
/// @param table - The table name to upsert into
/// @param fields - Tuple of .{key, expr, param} triples
///   (e.g. .{.{"user_id", "$$", user_id}, .{"status", "$$", status_param}})
/// @param conflict_columns - Columns that trigger the conflict (e.g. .{"user_id"}).
///   These columns are excluded from the ON CONFLICT UPDATE SET clause.
/// @param conn - The database connection
///
/// @returns A PatchQuery that generates the full INSERT ... ON CONFLICT DO UPDATE statement
pub fn upsertQuery(
    alloc: Allocator,
    table: []const u8,
    fields: anytype,
    conflict_columns: anytype,
    conn: anytype,
) !PatchQuery(@TypeOf(fields), @TypeOf(conn)) {
    var parameter_count: usize = 0;
    inline for (fields) |f| {
        const param = f[2];
        if (comptime isOptional(@TypeOf(param))) {
            if (param == .value) parameter_count += 1;
        } else {
            parameter_count += 1;
        }
    }
    if (parameter_count == 0) return error.NoParams;

    var sql: std.ArrayList(u8) = .empty;
    errdefer sql.deinit(alloc);

    try sql.print(alloc, "INSERT INTO {s} (", .{table});

    var first = true;
    inline for (fields) |f| {
        const key = f[0];
        const param = f[2];
        if ((comptime !isOptional(@TypeOf(param))) or param == .value) {
            if (!first) try sql.appendSlice(alloc, ", ");
            try sql.appendSlice(alloc, key);
            first = false;
        }
    }

    try sql.appendSlice(alloc, ") VALUES (");

    var param_num: usize = 1;
    first = true;
    inline for (fields) |f| {
        const expr = f[1];
        const param = f[2];
        if ((comptime !isOptional(@TypeOf(param))) or param == .value) {
            if (!first) try sql.appendSlice(alloc, ", ");
            const subbed = try subPlaceholders(alloc, expr, param_num);
            defer alloc.free(subbed);
            try sql.print(alloc, "{s}", .{subbed});
            param_num += 1;
            first = false;
        }
    }

    try sql.appendSlice(alloc, ") ON CONFLICT (");
    inline for (conflict_columns, 0..) |col, i| {
        if (i > 0) try sql.appendSlice(alloc, ", ");
        try sql.appendSlice(alloc, col);
    }
    try sql.appendSlice(alloc, ") DO UPDATE SET ");

    first = true;
    outer: inline for (fields) |f| {
        const key = f[0];
        const param = f[2];

        // Skip columns that are in conflict_columns
        inline for (conflict_columns) |col| {
            if (comptime std.mem.eql(u8, key, col)) continue :outer;
        }

        // Only include fields that are provided
        if ((comptime !isOptional(@TypeOf(param))) or param == .value) {
            if (!first) try sql.appendSlice(alloc, ", ");

            const expr = f[1];
            const col_param_num = getColParamNum(fields, key);
            const subbed = try subPlaceholders(alloc, expr, col_param_num);
            defer alloc.free(subbed);
            try sql.print(alloc, "{s} = {s}", .{ key, subbed });
            first = false;
        }
    }

    return .{
        ._alloc = alloc,
        ._conn = conn,
        .sql = sql,
        .params = fields,
    };
}

fn subPlaceholders(alloc: Allocator, expr: []const u8, num: usize) ![]const u8 {
    const replacement = try allocPrint(alloc, "${d}", .{num});
    defer alloc.free(replacement);
    return try std.mem.replaceOwned(u8, alloc, expr, "$$", replacement);
}

/// Returns the parameter number for a field by col name
fn getColParamNum(fields: anytype, col: []const u8) usize {
    var param_num: usize = 1;
    inline for (fields) |f| {
        const field_key = f[0];
        const param = f[2];
        if ((comptime !isOptional(@TypeOf(param))) or param == .value) {
            if (std.mem.eql(u8, field_key, col)) return param_num;
            param_num += 1;
        }
    }
    return 0;
}

pub fn PatchQuery(P: type, ConnType: type) type {
    return struct {
        pub const Self = @This();

        _alloc: Allocator,
        _conn: ConnType,
        sql: std.ArrayList(u8),
        params: P,

        pub fn deinit(self: *Self) void {
            self.sql.deinit(self._alloc);
        }

        /// Binds parameters and executes the prepared UPDATE statement.
        ///
        /// @param self - The PatchQuery instance containing the SQL and parameters
        /// @param fixed_params - A tuple of values to bind first (e.g. WHERE clause values).
        ///                       These are bound in order before the Optional params.
        ///
        /// @returns A pointer to pg.Result on success (or mock result for tests).
        ///          Errors if statement preparation or binding fails (production only).
        pub fn bindAndExecute(self: *Self, fixed_params: anytype) !*pg.Result {
            switch (ConnType) {
                *pg.Conn => {
                    var stmt = try pg.Stmt.init(self._conn, .{ .allocator = self._alloc });
                    errdefer stmt.deinit();

                    try stmt.prepare(self.sql.items, null);
                    inline for (fixed_params) |p| try stmt.bind(p);
                    inline for (self.params) |f| {
                        const param = f[2];
                        if (comptime isOptional(@TypeOf(param))) {
                            if (param == .value) try stmt.bind(param.value);
                        } else {
                            try stmt.bind(param);
                        }
                    }

                    return try stmt.execute();
                },
                *MockConnection => {
                    if (self._conn.captured_sql.len > 0) self._conn.alloc.free(self._conn.captured_sql);
                    self._conn.captured_sql = try self._conn.alloc.dupe(u8, self.sql.items);
                    return &self._conn.result;
                },
                else => return error.UnsupportedConnectionType,
            }
        }
    };
}

const MockConnection = struct {
    pub const Self = @This();

    captured_sql: []u8 = &[_]u8{},
    alloc: Allocator,
    result: pg.Result = undefined,

    fn init(alloc: Allocator) Self {
        return .{ .alloc = alloc };
    }

    fn deinit(self: *Self) void {
        if (self.captured_sql.len > 0) self.alloc.free(self.captured_sql);
    }

    fn execOpts(self: *Self, sql: []const u8, _: anytype, _: anytype) !?i64 {
        if (self.captured_sql.len > 0) self.alloc.free(self.captured_sql);
        self.captured_sql = try self.alloc.dupe(u8, sql);
        return 1;
    }

    fn begin(_: *Self) !void {}
    fn rollback(_: *Self) !void {}
    fn release(_: *Self) void {}
};

test "patchQuery - basic test" {
    const alloc = std.testing.allocator;

    const fields = .{
        .{ "name", "trim($$)", Optional([]u8){ .value = try alloc.alloc(u8, 4) } },
        .{ "is_universal", "$$", Optional([]bool){ .value = try alloc.alloc(bool, 1) } },
    };

    defer inline for (fields) |f| {
        if (f[2] == .value) alloc.free(f[2].value);
    };

    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    var query = try patchQuery(alloc, "skill", fields, 2, &mock);
    defer query.deinit();

    try query.sql.appendSlice(alloc, " WHERE site = $1 AND id = $2 RETURNING id;");

    _ = try query.bindAndExecute(.{ 1, 2 });
}

test "patchQuery - with partial fields" {
    const alloc = std.testing.allocator;

    const not_provided: Optional([]const u8) = .not_provided;
    const fields = .{
        .{ "name", "trim($$)", Optional([]const u8){ .value = "updated_name" } },
        .{ "is_universal", "$$", not_provided },
    };

    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    var query = try patchQuery(alloc, "skill", fields, 1, &mock);
    defer query.deinit();

    try query.sql.appendSlice(alloc, " WHERE id = $1;");

    _ = try query.bindAndExecute(.{123});

    const expected =
        "UPDATE skill SET name = trim($2) WHERE id = $1;";
    try std.testing.expectEqualStrings(expected, mock.captured_sql);
}

test "upsertQuery - simple upsert" {
    const alloc = std.testing.allocator;

    const fields = .{
        .{ "job_title", "$$", Optional([]const u8){ .value = "Engineer" } },
        .{ "phone_number", "$$", Optional([]const u8){ .value = "555-1234" } },
        .{ "about_me", "$$", Optional([]const u8){ .value = "Hello" } },
    };

    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    var query = try upsertQuery(
        alloc,
        "user_profile",
        fields,
        .{"user_id"},
        &mock,
    );
    defer query.deinit();

    try query.sql.appendSlice(alloc, ";");

    _ = try query.bindAndExecute(.{});

    const expected =
        "INSERT INTO user_profile (job_title, phone_number, about_me) " ++
        "VALUES ($1, $2, $3) " ++
        "ON CONFLICT (user_id) DO UPDATE SET " ++
        "job_title = $1, phone_number = $2, about_me = $3;";
    try std.testing.expectEqualStrings(expected, mock.captured_sql);
}

test "upsertQuery - simple upsert with partial fields" {
    const alloc = std.testing.allocator;

    const not_provided: Optional([]const u8) = .not_provided;
    const fields = .{
        .{ "job_title", "$$", Optional([]const u8){ .value = "Engineer" } },
        .{ "phone_number", "$$", not_provided },
        .{ "about_me", "$$", Optional([]const u8){ .value = "Hello" } },
    };

    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    var query = try upsertQuery(
        alloc,
        "user_profile",
        fields,
        .{"user_id"},
        &mock,
    );
    defer query.deinit();

    try query.sql.appendSlice(alloc, ";");

    _ = try query.bindAndExecute(.{});

    const expected =
        "INSERT INTO user_profile (job_title, about_me) " ++
        "VALUES ($1, $2) " ++
        "ON CONFLICT (user_id) DO UPDATE SET " ++
        "job_title = $1, about_me = $2;";
    try std.testing.expectEqualStrings(expected, mock.captured_sql);
}

test "upsertQuery - with fixed and dynamic fields, auto-exclude conflict columns" {
    const alloc = std.testing.allocator;

    const not_provided: Optional([]const u8) = .not_provided;
    const fields = .{
        .{ "company_id", "$$", @as(i32, 5) },
        .{ "user_id", "$$", @as(i32, 123) },
        .{ "shift", "TRIM($$)", "morning" },
        .{ "status", "$$", Optional([]const u8){ .value = "active" } },
        .{ "notes", "TRIM($$)", not_provided },
    };

    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    var query = try upsertQuery(
        alloc,
        "work_assignment",
        fields,
        .{ "user_id", "shift" },
        &mock,
    );
    defer query.deinit();

    _ = try query.bindAndExecute(.{});

    const expected =
        "INSERT INTO work_assignment (company_id, user_id, shift, status) " ++
        "VALUES ($1, $2, TRIM($3), $4) " ++
        "ON CONFLICT (user_id, shift) DO UPDATE SET " ++
        "company_id = $1, status = $4";
    try std.testing.expectEqualStrings(expected, mock.captured_sql);
}

test "upsertQuery - multiple update columns with expressions" {
    const alloc = std.testing.allocator;

    const not_provided: Optional([]const u8) = .not_provided;
    const fields = .{
        .{ "user_id", "$$", @as(i32, 123) },
        .{ "name", "TRIM($$)", Optional([]const u8){ .value = "John" } },
        .{ "email", "LOWER($$)", Optional([]const u8){ .value = "john@example.com" } },
        .{ "notes", "$$", not_provided },
    };

    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    var query = try upsertQuery(
        alloc,
        "users",
        fields,
        .{"user_id"},
        &mock,
    );
    defer query.deinit();

    _ = try query.bindAndExecute(.{});

    const expected =
        "INSERT INTO users (user_id, name, email) " ++
        "VALUES ($1, TRIM($2), LOWER($3)) " ++
        "ON CONFLICT (user_id) DO UPDATE SET " ++
        "name = TRIM($2), email = LOWER($3)";
    try std.testing.expectEqualStrings(expected, mock.captured_sql);
}

test "upsertQuery - all optional fields with auto-excluded conflict" {
    const alloc = std.testing.allocator;

    const fields = .{
        .{ "metric_id", "$$", @as(i32, 1) },
        .{ "status", "$$", Optional([]const u8){ .value = "active" } },
        .{ "count", "$$", Optional(i32){ .value = 42 } },
    };

    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    var query = try upsertQuery(
        alloc,
        "metrics",
        fields,
        .{"metric_id"},
        &mock,
    );
    defer query.deinit();

    _ = try query.bindAndExecute(.{});

    const expected =
        "INSERT INTO metrics (metric_id, status, count) " ++
        "VALUES ($1, $2, $3) " ++
        "ON CONFLICT (metric_id) DO UPDATE SET " ++
        "status = $2, count = $3";
    try std.testing.expectEqualStrings(expected, mock.captured_sql);
}
