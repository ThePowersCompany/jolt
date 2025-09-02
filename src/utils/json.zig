const std = @import("std");
const Allocator = std.mem.Allocator;

/// Caller owns the returned memory.
pub fn stringify(alloc: Allocator, value: anytype, options: std.json.Stringify.Options) ![]const u8 {
    var writer = std.Io.Writer.Allocating.init(alloc);
    errdefer writer.deinit();
    try std.json.Stringify.value(value, options, &writer.writer);
    return writer.toOwnedSlice();
}

/// Caller owns the returned memory.
pub fn stringifyBuf(buffer: []u8, value: anytype, options: std.json.Stringify.Options) !usize {
    var writer = std.Io.Writer.fixed(buffer);
    try std.json.Stringify.value(value, options, &writer);
    return writer.buffered().len;
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
    const written_bytes = stringifyBuf(&buffer, .{ "pong", 123 }, .{}) catch |err| {
        std.log.err("{}", .{err});
        return try std.testing.expect(false);
    };

    const pong_json = buffer[0..written_bytes];
    try std.testing.expectEqualStrings("[\"pong\",123]", pong_json);
}
