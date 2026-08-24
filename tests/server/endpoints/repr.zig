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

pub const ReprPostBody = struct {
    epoch_millis: EpochMillis,
};

const PostContext = struct {
    body: ReprPostBody,
};

pub fn post(ctx: *PostContext, _: Allocator) !Response(ReprPostBody) {
    return .{ .body = ctx.body };
}

pub const ReprPatchBody = struct {
    foo: union(enum) {
        pub const _repr = f64;
        bar: i64,
        baz: f64,

        pub fn jsonParse(
            allocator: Allocator,
            source: anytype,
            options: std.json.ParseOptions,
        ) !@This() {
            const val = try std.json.innerParse(f64, allocator, source, options);
            if (@trunc(val) == val) {
                return .{ .bar = @intFromFloat(val) };
            }
            return .{ .baz = val };
        }
    },
};

const PatchContext = struct {
    body: ReprPatchBody,
};

pub fn patch(ctx: *PatchContext, _: Allocator) !Response(ReprPatchBody) {
    return .{ .body = ctx.body };
}
