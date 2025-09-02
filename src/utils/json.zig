const std = @import("std");
const Allocator = std.mem.Allocator;

/// Caller owns the returned memory.
pub fn stringify(alloc: Allocator, value: anytype, options: std.json.Stringify.Options) ![]const u8 {
    var writer = std.Io.Writer.Allocating.init(alloc);
    errdefer writer.deinit();
    try std.json.Stringify.value(value, options, &writer.writer);
    return writer.toOwnedSlice();
}

/// Returns a sub-slice of `buffer`.
pub fn stringifyBuf(buffer: []u8, value: anytype, options: std.json.Stringify.Options) ![]const u8 {
    var writer = std.Io.Writer.fixed(buffer);
    try std.json.Stringify.value(value, options, &writer);
    return writer.buffered();
}

test "stringify" {
    const alloc = std.testing.allocator;
    const pong_json = stringify(alloc, .{ "pong", 123 }, .{}) catch |err| {
        std.log.err("{}", .{err});
        return try std.testing.expect(false);
    };
    defer alloc.free(pong_json);

    try std.testing.expectEqualStrings("[\"pong\",123]", pong_json);
}

test "stringifyBuf" {
    var buffer: [256]u8 = undefined;
    const pong_json = stringifyBuf(&buffer, .{ "pong", 123 }, .{}) catch |err| {
        std.log.err("{}", .{err});
        return try std.testing.expect(false);
    };

    try std.testing.expectEqualStrings("[\"pong\",123]", pong_json);
}
