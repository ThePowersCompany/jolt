const std = @import("std");
const Allocator = std.mem.Allocator;
const allocPrint = std.fmt.allocPrint;

const pg = @import("pg");
const db = @import("../db/database.zig");

const Optional = @import("types.zig").Optional;

pub const PatchQueryOpts = struct {};

pub fn patchQuery(
    alloc: Allocator,
    table: []const u8,
    keys: anytype,
    exprs: anytype,
    params: anytype,
    param_offset: usize,
    conn: anytype,
    opts: PatchQueryOpts,
) !PatchQuery(@TypeOf(params), @TypeOf(conn)) {
    var parameter_count: usize = 0;
    inline for (params) |p| {
        if (p != .not_provided) parameter_count += 1;
    }
    if (parameter_count == 0) return error.NoParams;

    var sql: std.ArrayList(u8) = .empty;
    errdefer sql.deinit(alloc);

    const update_prefix = try allocPrint(alloc, "UPDATE {s} SET ", .{table});
    defer alloc.free(update_prefix);
    try sql.appendSlice(alloc, update_prefix);

    var param_num: usize = 1; // 1-based
    inline for (keys, exprs, params) |k, e, v| {
        if (v != .not_provided) {
            const set_clause = try std.fmt.allocPrint(alloc, k ++ " = " ++ e, .{param_offset + param_num});
            defer alloc.free(set_clause);
            try sql.appendSlice(alloc, set_clause);
            if (param_num < parameter_count) {
                try sql.appendSlice(alloc, ", ");
            }
            param_num += 1;
        }
    }

    return .{
        ._alloc = alloc,
        ._conn = conn,
        ._opts = opts,
        .sql = sql,
        .params = params,
    };
}

pub fn PatchQuery(P: type, ConnType: type) type {
    return struct {
        pub const Self = @This();

        _alloc: Allocator,
        _conn: ConnType,
        _opts: PatchQueryOpts,
        sql: std.ArrayList(u8),
        params: P,

        pub fn deinit(self: *Self) void {
            self.sql.deinit(self._alloc);
        }

        /// Binds parameters and executes the prepared UPDATE statement.
        ///
        /// fixed_params:
        ///   A tuple of values to bind first (e.g., WHERE clause values).
        ///   These are bound in order before the Optional params.
        pub fn bindAndExecute(self: *Self, fixed_params: anytype) !*pg.Result {
            switch (ConnType) {
                *pg.Conn => {
                    var stmt = try pg.Stmt.init(self._conn, .{ .allocator = self._alloc });
                    errdefer stmt.deinit();

                    try stmt.prepare(self.sql.items, null);
                    inline for (fixed_params) |p| try stmt.bind(p);
                    inline for (self.params) |p| {
                        if (p != .not_provided) try stmt.bind(p.value);
                    }

                    return try stmt.execute();
                },
                // Testing path
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

    const keys = .{
        "name",
        "is_universal",
    };
    const exprs = .{
        "trim(${d})",
        "${d}",
    };

    const parameters = .{
        Optional([]u8){ .value = try alloc.alloc(u8, 4) },
        Optional([]bool){ .value = try alloc.alloc(bool, 1) },
    };

    defer inline for (parameters) |p| {
        if (p != .not_provided) alloc.free(p.value);
    };

    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    var query = try patchQuery(alloc, "skill", keys, exprs, parameters, 2, &mock, .{});
    defer query.deinit();

    try query.sql.appendSlice(alloc, " WHERE site = $1 AND id = $2 RETURNING id;");

    _ = try query.bindAndExecute(.{ 1, 2 });
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

    // Build SQL string with runtime presence checks, then build params and execute
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

test "execPatch - all fields provided" {
    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    const result = try execPatch(
        &mock,
        std.testing.allocator,
        "listing",
        .{
            .{ "name", .{ .value = "test" } },
            .{ "description", .{ .value = "desc" } },
        },
        .{
            .{ "id", 1 },
        },
    );

    try std.testing.expectEqualStrings(
        "UPDATE listing SET name = $1, description = $2 WHERE id = $3",
        mock.captured_sql,
    );
    try std.testing.expectEqual(1, result);
}

test "execPatch - first field not provided" {
    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    const not_provided: Optional([]const u8) = .not_provided;
    const result = try execPatch(
        &mock,
        std.testing.allocator,
        "listing",
        .{
            .{ "name", not_provided },
            .{ "description", .{ .value = "desc" } },
        },
        .{
            .{ "id", 1 },
        },
    );

    try std.testing.expectEqualStrings(
        "UPDATE listing SET description = $1 WHERE id = $2",
        mock.captured_sql,
    );
    try std.testing.expectEqual(1, result);
}

test "execPatch - second field not provided" {
    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    const not_provided: Optional([]const u8) = .not_provided;
    const result = try execPatch(
        &mock,
        std.testing.allocator,
        "listing",
        .{
            .{ "name", .{ .value = "test" } },
            .{ "description", not_provided },
        },
        .{
            .{ "id", 1 },
        },
    );

    try std.testing.expectEqualStrings(
        "UPDATE listing SET name = $1 WHERE id = $2",
        mock.captured_sql,
    );
    try std.testing.expectEqual(1, result);
}

test "execPatch - no fields provided" {
    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    const not_provided: Optional([]const u8) = .not_provided;
    const result = try execPatch(
        &mock,
        std.testing.allocator,
        "listing",
        .{
            .{ "name", not_provided },
            .{ "description", not_provided },
        },
        .{
            .{ "id", 1 },
        },
    );

    try std.testing.expectEqual(0, result);
}

test "execPatch - multiple where clauses" {
    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    _ = try execPatch(
        &mock,
        std.testing.allocator,
        "listing",
        .{
            .{ "name", .{ .value = "test" } },
        },
        .{
            .{ "id", 1 },
            .{ "user_id", 2 },
        },
    );

    try std.testing.expectEqualStrings(
        "UPDATE listing SET name = $1 WHERE id = $2 AND user_id = $3",
        mock.captured_sql,
    );
}

test "fieldIsPresent - Optional with value" {
    const val: Optional(i32) = .{ .value = 42 };
    try std.testing.expect(fieldIsPresent(val));
}

test "fieldIsPresent - Optional not provided" {
    const val: Optional(i32) = .not_provided;
    try std.testing.expect(!fieldIsPresent(val));
}

test "fieldIsPresent - Optional with null value" {
    const val: Optional(?i32) = .{ .value = null };
    try std.testing.expect(fieldIsPresent(val));
}

test "execPatch - middle field not provided" {
    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    const not_provided: Optional([]const u8) = .not_provided;
    _ = try execPatch(
        &mock,
        std.testing.allocator,
        "listing",
        .{
            .{ "name", .{ .value = "test" } },
            .{ "description", not_provided },
            .{ "price", .{ .value = 100 } },
        },
        .{
            .{ "id", 1 },
        },
    );

    try std.testing.expectEqualStrings(
        "UPDATE listing SET name = $1, price = $2 WHERE id = $3",
        mock.captured_sql,
    );
}

test "execPatch - nullable field explicitly set to null" {
    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    const null_value: Optional(?[]const u8) = .{ .value = null };
    _ = try execPatch(
        &mock,
        std.testing.allocator,
        "listing",
        .{
            .{ "name", .{ .value = "test" } },
            .{ "description", null_value },
        },
        .{
            .{ "id", 1 },
        },
    );

    try std.testing.expectEqualStrings(
        "UPDATE listing SET name = $1, description = $2 WHERE id = $3",
        mock.captured_sql,
    );
}

test "execPatch - single field" {
    var mock = MockConnection.init(std.testing.allocator);
    defer mock.deinit();

    _ = try execPatch(
        &mock,
        std.testing.allocator,
        "usr",
        .{
            .{ "email", .{ .value = "test@example.com" } },
        },
        .{
            .{ "id", 42 },
        },
    );

    try std.testing.expectEqualStrings(
        "UPDATE usr SET email = $1 WHERE id = $2",
        mock.captured_sql,
    );
}
