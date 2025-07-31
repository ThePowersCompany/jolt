const std = @import("std");
const Allocator = std.mem.Allocator;

/// Transforms snake_case to camelCase
/// Caller owns the returned memory, it must be freed.
pub fn snakeToCamelCase(alloc: Allocator, str: []const u8) Allocator.Error![]const u8 {
    var arr = std.ArrayList(u8).init(alloc);
    var capitalizeNextLetter = false;
    for (str) |char| {
        if (char == '_') {
            capitalizeNextLetter = true;
            continue;
        }

        if (capitalizeNextLetter) {
            try arr.append(std.ascii.toUpper(char));
            capitalizeNextLetter = false;
        } else {
            try arr.append(char);
        }
    }
    return try arr.toOwnedSlice();
}
