const std = @import("std");

pub fn strEqls(s1: []const u8, s2: []const u8) bool {
    return std.mem.eql(u8, s1, s2);
}

pub fn startsWith(haystack: []const u8, needle: []const u8) bool {
    return std.mem.startsWith(u8, haystack, needle);
}

pub const Method = enum {
    get,
    post,
    put,
    patch,
    delete,
};

pub const EndpointData = struct {
    query_params: ?[]const u8 = null,
    body: ?[]const u8 = null,
    response: ?[]const u8 = null,
};

pub const ParseResult = struct {
    // If empty, parsing hasn't completed yet.
    parsed: []const u8,
    // Whether all the fields of the parsed type are optional.
    optional: bool = false,
};

pub const AdjacentUnion = struct {
    /// The discriminator of an adjacently tagged union.
    /// Only one field in a struct may be this type of union.
    discriminator: []const u8,
    /// The full type name of the Union.
    name: []const u8,
};

/// A single flattened query-param leaf key.
pub const FlatLeaf = struct {
    name: []const u8,
    ts_type: []const u8,
    optional: bool,
};
