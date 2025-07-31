/// Returns a slice if it has a length > 0, else null.
pub fn extractSlice(comptime T: type, slice: ?[]const T) ?[]const T {
    if (slice) |s| {
        if (s.len != 0) {
            return s;
        }
    }
    return null;
}

/// Returns a string if it has a length > 0, else null.
pub fn extractString(str: ?[]const u8) ?[]const u8 {
    return extractSlice(u8, str);
}
