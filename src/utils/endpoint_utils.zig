const std = @import("std");
const Allocator = std.mem.Allocator;
const comptimePrint = std.fmt.comptimePrint;
const allocPrint = std.fmt.allocPrint;

const Response = @import("../zap/endpoint.zig").Response;

const pg = @import("pg");
const db = @import("../db/database.zig");

pub const OrderByOpts = struct {
    columnName: []const u8,
    ascending: bool = true,
};

const Table = enum {
    action_item,
    action_item_comment,
    action_item_category,
    area,
    category,
    company,
    downtime,
    dsc,
    dsc_row,
    equipment,
    line,
    product,
    product_config,
    profile,
    reason,
    shift,
    startup,
    lost_prod_category,
    lost_prod_reason,
    lost_production,
};

pub fn getTableName(comptime table: Table) []const u8 {
    return comptime switch (table) {
        .action_item => "action_item",
        .action_item_comment => "action_item_comment",
        .action_item_category => "action_item_category",
        .area => "area",
        .category => "category",
        .company => "company",
        .downtime => "downtime",
        .dsc => "dsc",
        .dsc_row => "dsc_row",
        .equipment => "equipment",
        .line => "line",
        .product => "product",
        .product_config => "product_config",
        .profile => "profile",
        .reason => "reason",
        .shift => "shift",
        .startup => "startup",
        .lost_prod_category => "lost_prod_category",
        .lost_prod_reason => "lost_prod_reason",
        .lost_production => "lost_production",
    };
}

pub fn getByIds(
    comptime T: type,
    alloc: Allocator,
    comptime table: Table,
    company_id: i32,
    site_id: ?i32,
    row_ids: ?[]i32,
    order_by_opts: ?OrderByOpts,
) ![]T {
    const info = @typeInfo(T);
    if (info != .@"struct") @compileError("T must be a struct");

    // Generate select for each field as column names.
    comptime var base_query: []const u8 = "SELECT ";
    inline for (info.@"struct".fields, 0..) |field, i| {
        base_query = base_query ++ field.name;
        if (i < info.@"struct".fields.len - 1) {
            base_query = base_query ++ ", ";
        }
    }
    base_query = base_query ++ " FROM " ++ comptime getTableName(table) ++ " s WHERE s.company_id = $1";

    var query = std.ArrayList(u8).init(alloc);
    defer query.deinit();
    try query.appendSlice(base_query);

    var paramIndex: i32 = 2;
    if (site_id != null) {
        try query.appendSlice(try std.fmt.allocPrint(alloc, " AND s.site = ${d}", .{paramIndex}));
        paramIndex += 1;
    }
    if (row_ids != null) {
        try query.appendSlice(try std.fmt.allocPrint(alloc, " AND s.id = any(${d})", .{paramIndex}));
        paramIndex += 1;
    }

    if (order_by_opts) |order_by| {
        const asc = if (order_by.ascending) "ASC" else "DESC";
        try query.appendSlice(
            try allocPrint(
                alloc,
                " ORDER BY s.{s} {s}",
                .{ order_by.columnName, asc },
            ),
        );
    }
    try query.appendSlice(";");

    const conn = try db.acquireConnection();
    defer conn.release();

    var stmt = try pg.Stmt.init(conn, .{ .allocator = alloc });
    errdefer stmt.deinit();

    stmt.prepare(query.items) catch |err| return db.logError(err, conn);

    try stmt.bind(company_id);
    if (site_id) |sid| {
        try stmt.bind(sid);
    }
    if (row_ids) |ids| {
        try stmt.bind(ids);
    }

    var result = stmt.execute() catch |err| return db.logError(err, conn);
    defer result.deinit();

    var arr = std.ArrayList(T).init(alloc);
    while (try result.next()) |row| {
        try arr.append(try row.to(T, .{ .allocator = alloc }));
    }
    return try arr.toOwnedSlice();
}

/// Deletes rows from the given table by id,
/// and responds with 204, 404, or 409 based on its success.
pub fn deleteRowByIds(comptime table: Table, company_id: i32, row_ids: []i32) !Response(void) {
    const sql = std.fmt.comptimePrint(
        \\ DELETE FROM {s} s
        \\ WHERE s.company_id = $1 AND s.id = any($2);
    ,
        .{comptime getTableName(table)},
    );

    const conn = try db.acquireConnection();
    defer conn.release();

    const rows_affected = conn.exec(sql, .{ company_id, row_ids }) catch |err| {
        if (db.isIntegrityConstraintViolation(err, conn)) {
            return .{ .err = "Resource is in use", .status = .conflict };
        }
        return db.logError(err, conn);
    } orelse 0;

    if (rows_affected == 0) {
        return .{ .status = .not_found };
    }

    return .{};
}

/// Deletes rows from the given table by id,
/// and responds with 204, 404, or 409 based on its success.
pub fn deleteSiteRowByIds(comptime table: Table, site_id: i32, row_ids: []i32) !Response(void) {
    const sql = std.fmt.comptimePrint(
        \\ DELETE FROM {s} s
        \\ WHERE s.site = $1 AND s.id = any($2);
    ,
        .{comptime getTableName(table)},
    );

    const conn = try db.acquireConnection();
    defer conn.release();

    const rows_affected = conn.exec(sql, .{ site_id, row_ids }) catch |err| {
        if (db.isIntegrityConstraintViolation(err, conn)) {
            return .{ .err = "Resource is in use", .status = .conflict };
        }
        return db.logError(err, conn);
    } orelse 0;

    if (rows_affected == 0) {
        return .{ .status = .not_found };
    }

    return .{};
}
