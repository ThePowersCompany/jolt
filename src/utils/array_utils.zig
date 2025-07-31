const std = @import("std");

pub fn containsString(arr: []const []const u8, item: []const u8) bool {
    for (arr) |e| {
        if (std.mem.eql(u8, item, e)) {
            return true;
        }
    }
    return false;
}

pub fn sortByString(comptime T: type, items: []T, comptime field_name: []const u8) void {
    const sort = struct {
        fn sort(_: void, lhs: T, rhs: T) bool {
            return std.ascii.lessThanIgnoreCase(@field(lhs, field_name), @field(rhs, field_name));
        }
    }.sort;
    std.mem.sort(T, items, {}, sort);
}

pub fn sortByNumberAsc(comptime T: type, items: []T, comptime field_name: []const u8) void {
    const sort = struct {
        fn sort(_: void, lhs: T, rhs: T) bool {
            return @field(lhs, field_name) < @field(rhs, field_name);
        }
    }.sort;
    std.mem.sort(T, items, {}, sort);
}

pub fn sortByStringLengthDesc(comptime T: type, items: []T, comptime field_name: []const u8) void {
    const sort = struct {
        fn sort(_: void, lhs: T, rhs: T) bool {
            return @field(lhs, field_name).len > @field(rhs, field_name).len;
        }
    }.sort;
    std.mem.sort(T, items, {}, sort);
}
