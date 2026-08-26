const std = @import("std");

const Allocator = std.mem.Allocator;
const BuiltType = @import("typescript.zig").BuiltType;

pub const TypeUsage = enum { body, query_params };

const Declaration = struct {
    rendered: ?BuiltType = null,
    referenced: bool = false,
    usage: ?TypeUsage = null,
};

pub const Registry = struct {
    declarations: std.StringHashMap(Declaration),
    names: std.StringHashMap([]const u8),

    pub fn init(allocator: Allocator) Registry {
        return .{
            .declarations = std.StringHashMap(Declaration).init(allocator),
            .names = std.StringHashMap([]const u8).init(allocator),
        };
    }

    pub fn deinit(self: *Registry) void {
        self.declarations.deinit();
        self.names.deinit();
    }

    pub fn isDeclared(self: *const Registry, type_id: []const u8) bool {
        return self.declarations.contains(type_id);
    }

    pub fn reference(self: *const Registry, type_id: []const u8) ?BuiltType {
        const decl = self.declarations.get(type_id) orelse return null;
        return decl.rendered orelse .{ .expr = .{ .named = shortName(type_id) } };
    }

    pub fn declare(self: *Registry, comptime T: type) !void {
        const type_id = @typeName(T);
        if (self.declarations.contains(type_id)) {
            std.log.err("Tried to redeclare top-level type: {s}", .{type_id});
            return error.DuplicateDeclaration;
        }
        try self.declarations.put(type_id, .{});
    }

    pub fn setRendered(self: *Registry, type_id: []const u8, built_type: BuiltType) !void {
        const existing = self.declarations.get(type_id);
        if (existing) |decl| {
            if (decl.rendered != null) {
                std.log.err("Duplicate type name {s}", .{type_id});
                return error.DuplicateTypeName;
            }
        }
        try self.reserveName(type_id);
        const usage = if (existing) |decl| decl.usage else null;
        const referenced = if (existing) |decl| decl.referenced else false;
        try self.declarations.put(type_id, .{
            .rendered = built_type,
            .referenced = referenced,
            .usage = usage,
        });
    }

    pub fn recordUsage(self: *Registry, type_id: []const u8, usage: TypeUsage) !void {
        const decl = self.declarations.getPtr(type_id) orelse return;
        decl.referenced = true;
        if (decl.usage) |prev| {
            if (prev != usage) {
                std.log.err("Top-level type {s} is used as both a body and query params", .{type_id});
                return error.TopLevelTypeUsedInMultipleContexts;
            }
            return;
        }
        decl.usage = usage;
    }

    pub fn recordReference(self: *Registry, type_id: []const u8) void {
        const decl = self.declarations.getPtr(type_id) orelse return;
        decl.referenced = true;
    }

    pub fn isReferenced(self: *const Registry, type_id: []const u8) bool {
        const decl = self.declarations.get(type_id) orelse return false;
        return decl.referenced;
    }

    pub fn usageFor(self: *const Registry, type_id: []const u8) ?TypeUsage {
        const decl = self.declarations.get(type_id) orelse return null;
        return decl.usage;
    }

    pub fn shortName(type_name: []const u8) []const u8 {
        const last_dot = std.mem.lastIndexOfScalar(u8, type_name, '.') orelse return type_name;
        const last_segment = type_name[last_dot + 1 ..];
        var generic_iter = std.mem.splitScalar(u8, last_segment, '(');
        return generic_iter.first();
    }

    fn reserveName(self: *Registry, type_id: []const u8) !void {
        const ts_name = shortName(type_id);
        if (self.names.get(ts_name)) |other_type_id| {
            std.log.err(
                "Top-level types {s} and {s} both emit as {s}",
                .{ other_type_id, type_id, ts_name },
            );
            return error.DuplicateTypeName;
        }
        try self.names.put(ts_name, type_id);
    }
};
