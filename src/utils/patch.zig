const std = @import("std");
const Allocator = std.mem.Allocator;

const pg = @import("pg");

const Optional = @import("types.zig").Optional;

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

const MockConnection = struct {
    captured_sql: []u8 = &[_]u8{},
    alloc: Allocator,

    fn init(alloc: Allocator) MockConnection {
        return .{ .alloc = alloc };
    }

    fn deinit(self: *MockConnection) void {
        if (self.captured_sql.len > 0) self.alloc.free(self.captured_sql);
    }

    fn execOpts(self: *MockConnection, sql: []const u8, _: anytype, _: anytype) !?i64 {
        if (self.captured_sql.len > 0) self.alloc.free(self.captured_sql);
        self.captured_sql = try self.alloc.dupe(u8, sql);
        return 1;
    }
};

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

    // null_value has .value set (to null), so it should be included
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
