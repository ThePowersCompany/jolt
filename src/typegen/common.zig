const std = @import("std");
const TypeExpr = @import("typescript.zig").TypeExpr;

pub fn strEqls(s1: []const u8, s2: []const u8) bool {
    return std.mem.eql(u8, s1, s2);
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
    expr: *const TypeExpr,
    optional: bool,
};
