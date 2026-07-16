const std = @import("std");
const Allocator = std.mem.Allocator;

const jolt = @import("jolt");
const types = jolt.types;
const Response = jolt.Response;
const Optional = types.Optional;

const GetContext = struct {
    query_params: struct {
        room: i32,
        cursor: Optional(struct {
            cursor: i64,
            before: bool = false,
        }) = .not_provided,
        limit: u32 = 10,
    },
};

pub const GatedGroupQpDetails = struct {
    room: i32,
    limit: u32,
    cursor_present: bool,
    cursor: ?i64,
    before: ?bool,
};

pub fn get(ctx: *GetContext, _: Allocator) !Response(GatedGroupQpDetails) {
    const qp = ctx.query_params;
    if (qp.cursor.get()) |group| {
        return .{ .body = .{
            .room = qp.room,
            .limit = qp.limit,
            .cursor_present = true,
            .cursor = group.cursor,
            .before = group.before,
        } };
    }
    return .{ .body = .{
        .room = qp.room,
        .limit = qp.limit,
        .cursor_present = false,
        .cursor = null,
        .before = null,
    } };
}
