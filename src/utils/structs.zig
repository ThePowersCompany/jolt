const std = @import("std");

const containers_module = @import("./containers.zig");
const containerKind = containers_module.containerKind;
const isNotRequired = containers_module.isNotRequired;

const containsString = @import("./array_utils.zig").containsString;

/// Returns if every required leaf key of struct `T` is in `present_keys`.
pub fn requiredKeysPresent(comptime T: type, present_keys: []const []const u8) bool {
    inline for (@typeInfo(T).@"struct".fields) |field| {
        if (comptime isNotRequired(field)) continue;

        if (comptime @typeInfo(field.type) == .@"struct" and containerKind(field.type) == .composite) {
            if (!requiredKeysPresent(field.type, present_keys)) return false;
        } else if (!containsString(present_keys, field.name)) {
            return false;
        }
    }
    return true;
}

/// Returns if any leaf key of struct `T` is in `present_keys`.
pub fn anyStructLeafKeyPresent(comptime T: type, present_keys: []const []const u8) bool {
    inline for (@typeInfo(T).@"struct".fields) |field| {
        if (comptime @typeInfo(field.type) == .@"struct" and containerKind(field.type) == .composite) {
            if (anyStructLeafKeyPresent(field.type, present_keys)) return true;
        } else if (containsString(present_keys, field.name)) {
            return true;
        }
    }
    return false;
}
