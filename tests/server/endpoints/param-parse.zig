const std = @import("std");
const Allocator = std.mem.Allocator;

const jolt = @import("jolt");
const types = jolt.types;
const Response = jolt.Response;
const Optional = types.Optional;

const Timestamp = struct {
    millis: i64,

    pub fn paramParse(_: Allocator, param: []const u8) !Timestamp {
        return .{ .millis = try std.fmt.parseInt(i64, param, 10) };
    }
};

const GetContext = struct {
    query_params: struct {
        when: Optional(Timestamp) = .not_provided,
    },
};

pub const ParamParseQpDetails = struct {
    when_present: bool,
    millis: ?i64,
};

pub fn get(ctx: *GetContext, _: Allocator) !Response(ParamParseQpDetails) {
    if (ctx.query_params.when.get()) |ts| {
        return .{ .body = .{
            .when_present = true,
            .millis = ts.millis,
        } };
    }
    return .{ .body = .{
        .when_present = false,
        .millis = null,
    } };
}
