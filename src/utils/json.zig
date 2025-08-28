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
pub fn stringifyBuf(buffer: []u8, value: anytype, options: std.json.Stringify.Options) !void {
    var writer = std.Io.Writer.fixed(buffer);
    try std.json.Stringify.value(value, options, &writer);
}
