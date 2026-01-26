const std = @import("std");
const Allocator = std.mem.Allocator;
const allocPrint = std.fmt.allocPrint;

const pg = @import("pg");
const db = @import("../db/database.zig");

const Optional = @import("types.zig").Optional;

/// Generates a dynamic UPDATE query for fields marked as .value in the params.
/// Only fields where the parameter is not .not_provided are included in the SET clause.
/// The query must be completed by the caller with a WHERE clause and executed via bindAndExecute.
///
/// @param alloc - Allocator for SQL string construction
/// @param table - The table name to update
/// @param fields - Tuple of .{key, expr, param} triples
///   (e.g. .{.{"name", "TRIM(${d})", name_param}, .{"age", "${d}", age_param}})
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
        if (@typeInfo(@TypeOf(param)) == .@"union") {
            if (param != .not_provided) parameter_count += 1;
        } else {
            parameter_count += 1;
        }
    }
    if (parameter_count == 0) return error.NoParams;

    var sql: std.ArrayList(u8) = .empty;
    errdefer sql.deinit(alloc);

    const update_prefix = try allocPrint(alloc, "UPDATE {s} SET ", .{table});
    defer alloc.free(update_prefix);
    try sql.appendSlice(alloc, update_prefix);

    var param_num: usize = 1;
    inline for (fields) |f| {
        const key = f[0];
        const expr = f[1];
        const param = f[2];
        if (@typeInfo(@TypeOf(param)) != .@"union" or param != .not_provided) {
            const set_clause = try allocPrint(alloc, "{s} = " ++ expr, .{ key, param_offset + param_num });
            defer alloc.free(set_clause);

            try sql.appendSlice(alloc, set_clause);
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
///
/// @param alloc - Allocator for SQL string construction
/// @param table - The table name to upsert into
/// @param fields - Tuple of .{key, expr, param} triples
///   (e.g. .{.{"user_id", "${d}", user_id}, .{"status", "${d}", status_param}})
/// @param conflict_columns - Columns that trigger the conflict (e.g. .{"user_id"})
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
        if (@typeInfo(@TypeOf(param)) == .@"union") {
            if (param != .not_provided) parameter_count += 1;
        } else {
            parameter_count += 1;
        }
    }
    if (parameter_count == 0) return error.NoParams;

    var sql: std.ArrayList(u8) = .empty;
    errdefer sql.deinit(alloc);

    const insert_prefix = try allocPrint(alloc, "INSERT INTO {s} (", .{table});
    defer alloc.free(insert_prefix);
    try sql.appendSlice(alloc, insert_prefix);

    var first = true;
    inline for (fields) |f| {
        const key = f[0];
        const param = f[2];
        if (@typeInfo(@TypeOf(param)) != .@"union" or param != .not_provided) {
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
        if (@typeInfo(@TypeOf(param)) != .@"union" or param != .not_provided) {
            if (!first) try sql.appendSlice(alloc, ", ");
            const value_expr = try allocPrint(alloc, expr, .{param_num});
            defer alloc.free(value_expr);
            try sql.appendSlice(alloc, value_expr);
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

    param_num = 1;
    first = true;

    inline for (fields) |f| {
        const key = f[0];
        const expr = f[1];
        const param = f[2];
        if (@typeInfo(@TypeOf(param)) != .@"union" or param != .not_provided) {
            if (!first) try sql.appendSlice(alloc, ", ");
            const set_expr = try allocPrint(alloc, "{s} = " ++ expr, .{ key, param_num });
            defer alloc.free(set_expr);
            try sql.appendSlice(alloc, set_expr);
            param_num += 1;
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
                        switch (@typeInfo(@TypeOf(param))) {
                            .@"union" => if (param != .not_provided) try stmt.bind(param.value),
                            else => try stmt.bind(param),
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
        .{ "name", "trim(${d})", Optional([]u8){ .value = try alloc.alloc(u8, 4) } },
        .{ "is_universal", "${d}", Optional([]bool){ .value = try alloc.alloc(bool, 1) } },
    };

    defer inline for (fields) |f| {
        if (f[2] != .not_provided) alloc.free(f[2].value);
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
        .{ "name", "trim(${d})", Optional([]const u8){ .value = "updated_name" } },
        .{ "is_universal", "${d}", not_provided },
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
        .{ "job_title", "${d}", Optional([]const u8){ .value = "Engineer" } },
        .{ "phone_number", "${d}", Optional([]const u8){ .value = "555-1234" } },
        .{ "about_me", "${d}", Optional([]const u8){ .value = "Hello" } },
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
        .{ "job_title", "${d}", Optional([]const u8){ .value = "Engineer" } },
        .{ "phone_number", "${d}", not_provided },
        .{ "about_me", "${d}", Optional([]const u8){ .value = "Hello" } },
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

test "upsertQuery - with fixed and dynamic fields" {
    const alloc = std.testing.allocator;

    const not_provided: Optional([]const u8) = .not_provided;
    const fields = .{
        .{ "company_id", "${d}", @as(i32, 5) },
        .{ "user_id", "${d}", @as(i32, 123) },
        .{ "shift", "TRIM(${d})", "morning" },
        .{ "status", "${d}", Optional([]const u8){ .value = "active" } },
        .{ "notes", "TRIM(${d})", not_provided },
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
        "company_id = $1, user_id = $2, shift = TRIM($3), status = $4";
    try std.testing.expectEqualStrings(expected, mock.captured_sql);
}

/// Builds and executes a dynamic UPDATE query at runtime.
/// Fields are passed as tuples of {name, value} where value is Optional(T).
/// Only fields where value is present are included in the UPDATE.
pub fn execPatch(
    conn: anytype,
    alloc: Allocator,
    comptime table: []const u8,
    fields: anytype,
    where_clauses: anytype,
) !?i64 {
    var sql: std.ArrayList(u8) = .empty;
    defer sql.deinit(alloc);

    try sql.appendSlice(alloc, "UPDATE " ++ table ++ " SET ");

    const field_count = try buildSetClause(alloc, &sql, fields, 0, 1, true);
    if (field_count.count == 0) return 0;

    try buildWhereClause(alloc, &sql, where_clauses, field_count.next_param);

    return execWithParams(conn, alloc, sql.items, fields, where_clauses, 0, .{});
}

/// Result from building SET clause
const SetClauseResult = struct { count: usize, next_param: usize };

/// Builds the SET clause SQL, returns field count and next param index
fn buildSetClause(
    alloc: Allocator,
    sql: *std.ArrayList(u8),
    fields: anytype,
    comptime idx: usize,
    param: usize,
    first: bool,
) !SetClauseResult {
    if (idx >= fields.len) return .{ .count = 0, .next_param = param };

    const name = fields[idx][0];
    const value = fields[idx][1];

    if (fieldIsPresent(value)) {
        if (!first) try sql.appendSlice(alloc, ", ");
        try sql.appendSlice(alloc, name);
        try sql.appendSlice(alloc, " = $");
        try appendInt(alloc, sql, param);

        const rest = try buildSetClause(alloc, sql, fields, idx + 1, param + 1, false);
        return .{ .count = rest.count + 1, .next_param = rest.next_param };
    } else {
        return buildSetClause(alloc, sql, fields, idx + 1, param, first);
    }
}

/// Builds the WHERE clause SQL
fn buildWhereClause(alloc: Allocator, sql: *std.ArrayList(u8), clauses: anytype, start_param: usize) !void {
    inline for (clauses, 0..) |clause, i| {
        try sql.appendSlice(alloc, if (i == 0) " WHERE " else " AND ");
        try sql.appendSlice(alloc, clause[0]);
        try sql.appendSlice(alloc, " = $");
        try appendInt(alloc, sql, start_param + i);
    }
}

/// Recursively builds params tuple and executes query
fn execWithParams(
    conn: anytype,
    alloc: Allocator,
    sql: []const u8,
    fields: anytype,
    where_clauses: anytype,
    comptime field_idx: usize,
    params: anytype,
) !?i64 {
    if (field_idx >= fields.len) return addWhereParams(conn, alloc, sql, where_clauses, 0, params);

    const value = fields[field_idx][1];
    const info = @typeInfo(@TypeOf(value));

    if (info == .@"union") {
        return switch (value) {
            .not_provided => execWithParams(conn, alloc, sql, fields, where_clauses, field_idx + 1, params),
            .value => |v| execWithParams(conn, alloc, sql, fields, where_clauses, field_idx + 1, params ++ .{v}),
        };
    } else if (info == .optional) {
        return if (value) |v|
            execWithParams(conn, alloc, sql, fields, where_clauses, field_idx + 1, params ++ .{v})
        else
            execWithParams(conn, alloc, sql, fields, where_clauses, field_idx + 1, params);
    } else {
        return execWithParams(conn, alloc, sql, fields, where_clauses, field_idx + 1, params ++ .{value});
    }
}

/// Adds WHERE clause params and executes
fn addWhereParams(
    conn: anytype,
    alloc: Allocator,
    sql: []const u8,
    clauses: anytype,
    comptime idx: usize,
    params: anytype,
) !?i64 {
    if (idx >= clauses.len) return conn.execOpts(sql, params, .{ .allocator = alloc });
    return addWhereParams(conn, alloc, sql, clauses, idx + 1, params ++ .{clauses[idx][1]});
}

fn appendInt(alloc: Allocator, list: *std.ArrayList(u8), val: usize) !void {
    var buf: [20]u8 = undefined;
    const str = std.fmt.bufPrint(&buf, "{d}", .{val}) catch unreachable;
    try list.appendSlice(alloc, str);
}

fn fieldIsPresent(value: anytype) bool {
    const info = @typeInfo(@TypeOf(value));
    if (info == .@"union") return std.meta.activeTag(value) != .not_provided;
    if (info == .optional) return value != null;
    return true;
}
