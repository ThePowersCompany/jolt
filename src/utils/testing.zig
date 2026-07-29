const std = @import("std");
const Allocator = std.mem.Allocator;

/// Compares strings (strips whitespace for comparisons)
pub fn expectContent(expected: []const u8, actual: []const u8) !void {
    const alloc = std.testing.allocator;
    const s1 = try stripWhitespace(alloc, expected);
    defer alloc.free(s1);

    const s2 = try stripWhitespace(alloc, actual);
    defer alloc.free(s2);

    try std.testing.expectEqualStrings(s1, s2);
}

fn stripWhitespace(alloc: Allocator, str: []const u8) ![]const u8 {
    var chars: std.ArrayList(u8) = .empty;
    for (str) |c| if (!std.ascii.isWhitespace(c)) {
        try chars.append(alloc, c);
    };
    return try chars.toOwnedSlice(alloc);
}

fn noAlloc(_: *anyopaque, _: usize, _: std.mem.Alignment, _: usize) ?[*]u8 {
    return null;
}

/// An allocator that always returns error.OutOfMemory
pub const no_alloc: Allocator = .{
    .ptr = undefined,
    .vtable = &.{
        .alloc = noAlloc,
        .resize = Allocator.noResize,
        .remap = Allocator.noRemap,
        .free = Allocator.noFree,
    },
};
