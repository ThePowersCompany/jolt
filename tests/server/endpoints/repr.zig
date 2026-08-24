const std = @import("std");
const Allocator = std.mem.Allocator;

const jolt = @import("jolt");
const Response = jolt.Response;

const DateTime = @import("../utils/datetime.zig").DateTime;

const EpochMillis = struct {
    value: DateTime,

    pub const _repr: type = i64;

    pub fn jsonParse(
        allocator: Allocator,
        source: anytype,
        options: std.json.ParseOptions,
    ) !@This() {
        const timestamp = try std.json.innerParse(i64, allocator, source, options);
        return .{ .value = DateTime.fromUnix(timestamp, .milliseconds) };
    }

    pub fn jsonStringify(self: @This(), out: anytype) !void {
        try out.write(@divExact(self.value.micros, 1_000));
    }
};

pub const ReprBody = struct {
    epoch_millis: EpochMillis,
};

const PostContext = struct {
    body: ReprBody,
};

pub fn post(ctx: *PostContext, _: Allocator) !Response(ReprBody) {
    return .{ .body = ctx.body };
}
