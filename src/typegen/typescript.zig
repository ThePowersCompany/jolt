const std = @import("std");
const expectContent = @import("../utils/testing.zig").expectContent;

const Allocator = std.mem.Allocator;
const ArrayList = std.ArrayList;

/// A TypeScript type before it is rendered as a string.
pub const TypeExpr = union(enum) {
    verbatim: []const u8,
    named: []const u8,
    array: *const TypeExpr,
    nullable: *const TypeExpr,
    object: []const Field,
    inline_object: []const Field,
    parens: *const TypeExpr,
    intersection: []const *const TypeExpr,
    generic: Generic,

    pub const Field = struct {
        name: []const u8,
        expr: *const TypeExpr,
        optional: bool = false,
    };

    pub const Generic = struct {
        name: []const u8,
        arguments: []const *const TypeExpr,
    };

    /// Builds a collection of expressions with arena-allocated nodes.
    pub const Components = struct {
        allocator: Allocator,
        items: ArrayList(*const TypeExpr) = .empty,

        pub fn init(allocator: Allocator) Components {
            return .{ .allocator = allocator };
        }

        /// Allocates `expr` before adding it to the collection.
        pub fn append(self: *Components, expr: TypeExpr) !void {
            try self.appendRef(try allocate(self.allocator, expr));
        }

        /// Adds a reference to an expression that has already been allocated.
        pub fn appendRef(self: *Components, expr: *const TypeExpr) !void {
            try self.items.append(self.allocator, expr);
        }

        pub fn intoIntersection(self: *Components) !TypeExpr {
            return .{ .intersection = try self.items.toOwnedSlice(self.allocator) };
        }
    };

    /// Allocates an expression so another expression can refer to it.
    pub fn allocate(allocator: Allocator, expr: TypeExpr) !*const TypeExpr {
        const result = try allocator.create(TypeExpr);
        result.* = expr;
        return result;
    }

    pub fn singleField(allocator: Allocator, name: []const u8, expr: TypeExpr) !TypeExpr {
        return .{ .inline_object = try allocator.dupe(Field, &.{
            .{ .name = name, .expr = try allocate(allocator, expr) },
        }) };
    }

    pub fn makeGeneric(
        allocator: Allocator,
        name: []const u8,
        arguments: []const *const TypeExpr,
    ) !TypeExpr {
        return .{ .generic = .{
            .name = name,
            .arguments = try allocator.dupe(*const TypeExpr, arguments),
        } };
    }

    pub fn render(self: TypeExpr, allocator: Allocator) ![]const u8 {
        return switch (self) {
            .verbatim, .named => |text| text,
            .array => |element| std.fmt.allocPrint(allocator, "{s}[]", .{try element.render(allocator)}),
            .nullable => |child| std.fmt.allocPrint(allocator, "{s}|null", .{try child.render(allocator)}),
            .object => |fields| {
                var result: ArrayList(u8) = .empty;
                try result.appendSlice(allocator, "{\n");
                for (fields) |field| {
                    try result.appendSlice(allocator, "  ");
                    try result.appendSlice(allocator, field.name);
                    try result.appendSlice(allocator, if (field.optional) "?: " else ": ");
                    try result.appendSlice(allocator, try field.expr.render(allocator));
                    try result.append(allocator, '\n');
                }
                try result.append(allocator, '}');
                return result.toOwnedSlice(allocator);
            },
            .inline_object => |fields| {
                var result: ArrayList(u8) = .empty;
                try result.appendSlice(allocator, "{ ");
                for (fields, 0..) |field, index| {
                    if (index != 0) try result.appendSlice(allocator, ", ");
                    try result.appendSlice(allocator, field.name);
                    try result.appendSlice(allocator, if (field.optional) "?: " else ": ");
                    try result.appendSlice(allocator, try field.expr.render(allocator));
                }
                try result.appendSlice(allocator, " }");
                return result.toOwnedSlice(allocator);
            },
            .parens => |child| std.fmt.allocPrint(allocator, "({s})", .{try child.render(allocator)}),
            .intersection => |components| {
                var result: ArrayList(u8) = .empty;
                for (components, 0..) |component, index| {
                    if (index != 0) try result.appendSlice(allocator, " & ");
                    try result.appendSlice(allocator, try component.render(allocator));
                }
                return result.toOwnedSlice(allocator);
            },
            .generic => |generic| {
                var result: ArrayList(u8) = .empty;
                try result.appendSlice(allocator, generic.name);
                try result.append(allocator, '<');
                for (generic.arguments, 0..) |argument, index| {
                    if (index != 0) try result.appendSlice(allocator, ", ");
                    try result.appendSlice(allocator, try argument.render(allocator));
                }
                try result.append(allocator, '>');
                return result.toOwnedSlice(allocator);
            },
        };
    }
};

/// A generated TypeScript type expression together with metadata about its use.
///
/// `expr` - Description of the TypeScript type itself.
/// `optional` - describes whether a property using that expression may be omitted (`property?: Type`)
/// and is kept separate because it is not part of the type expression:
/// a nullable type, for example, is still a required property unless it is explicitly omittable.
pub const TypeDescriptor = struct {
    expr: TypeExpr,
    optional: bool = false,

    pub fn render(self: TypeDescriptor, allocator: Allocator) ![]const u8 {
        return self.expr.render(allocator);
    }
};

test "TypeExpr renders nested types" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const allocator = arena.allocator();

    const number = try TypeExpr.allocate(allocator, .{ .named = "number" });
    const nullable_number = try TypeExpr.allocate(allocator, .{ .nullable = number });
    const fields = [_]TypeExpr.Field{.{
        .name = "limit",
        .expr = nullable_number,
        .optional = true,
    }};

    const object = TypeExpr{ .object = &fields };
    try expectContent(
        \\{
        \\  limit?: number|null
        \\}
    , try object.render(allocator));
}

test "TypeExpr.Components allocates values and preserves references" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const allocator = arena.allocator();

    var components = TypeExpr.Components.init(allocator);
    try components.append(.{ .named = "string" });
    try components.appendRef(try TypeExpr.allocate(allocator, .{ .named = "number" }));

    const expr = try components.intoIntersection();
    try expectContent("string & number", try expr.render(allocator));
}
