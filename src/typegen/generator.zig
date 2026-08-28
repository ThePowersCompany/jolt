const std = @import("std");
const ArenaAllocator = std.heap.ArenaAllocator;
const Allocator = std.mem.Allocator;
const StringArrayHashMap = std.StringArrayHashMap;
const ArrayList = std.ArrayList;
const Type = std.builtin.Type;
const allocPrint = std.fmt.allocPrint;
const stringToEnum = std.meta.stringToEnum;

const EndpointDef = @import("../main.zig").EndpointDef;
const UnionRepr = @import("../utils/unions.zig").UnionRepr;

const types = @import("../utils/types.zig");
const Optional = types.Optional;
const isOptional = types.isOptional;
const JsonArray = types.JsonArray;

const common = @import("./common.zig");
const strEqls = common.strEqls;
const Method = common.Method;
const EndpointData = common.EndpointData;
const AdjacentUnion = common.AdjacentUnion;
const FlatLeaf = common.FlatLeaf;

const typescript = @import("./typescript.zig");
const TypeDescriptor = typescript.TypeDescriptor;
const TypeExpr = typescript.TypeExpr;

const declarations = @import("./declarations.zig");
const Registry = declarations.Registry;
const TypeUsage = declarations.TypeUsage;

const containers_mod = @import("../utils/containers.zig");
const hasParamParse = containers_mod.hasParamParse;
const getRequiredKeyCount = containers_mod.getRequiredKeyCount;

const Str = @import("../utils/str.zig").Str;
const Date = @import("../utils/datetime.zig").Date;
const DateTime = @import("../utils/datetime.zig").DateTime;

const unions_mod = @import("../utils/unions.zig");
const isLiftableUnion = unions_mod.isLiftableUnion;

const expectEqual = std.testing.expectEqual;
const expectContent = @import("../utils/testing.zig").expectContent;

const TypeGenerationContext = struct {
    usage: TypeUsage,
};

/// Generates TypeScript API definitions from Jolt endpoint declarations.
///
/// It identifies named top-level types referenced by endpoint contexts and responses,
/// records whether each is used as a body or query parameter,
/// and renders the declarations required by the generated API.
/// It then builds `Spec`, which maps HTTP methods and paths to their request and response types.
///
/// Query-parameter types use different semantics from JSON bodies:
/// nested structs and unions may be flattened to reflect Jolt's query parsing behavior.
pub const TypeGenerator = struct {
    const Self = @This();

    arena_alloc: Allocator,
    registry: Registry,

    get_endpoints: StringArrayHashMap(EndpointData),
    post_endpoints: StringArrayHashMap(EndpointData),
    put_endpoints: StringArrayHashMap(EndpointData),
    patch_endpoints: StringArrayHashMap(EndpointData),
    delete_endpoints: StringArrayHashMap(EndpointData),

    pub fn init(arena_alloc: Allocator) !Self {
        return .{
            .arena_alloc = arena_alloc,
            .registry = Registry.init(arena_alloc),
            .get_endpoints = StringArrayHashMap(EndpointData).init(arena_alloc),
            .post_endpoints = StringArrayHashMap(EndpointData).init(arena_alloc),
            .put_endpoints = StringArrayHashMap(EndpointData).init(arena_alloc),
            .patch_endpoints = StringArrayHashMap(EndpointData).init(arena_alloc),
            .delete_endpoints = StringArrayHashMap(EndpointData).init(arena_alloc),
        };
    }

    pub fn deinit(self: *Self) void {
        self.registry.deinit();
        self.get_endpoints.deinit();
        self.post_endpoints.deinit();
        self.put_endpoints.deinit();
        self.patch_endpoints.deinit();
        self.delete_endpoints.deinit();
    }

    pub fn generateTypes(self: *Self, comptime endpoints: []const EndpointDef) ![]const u8 {
        @setEvalBranchQuota(endpoints.len * 2000);

        try self.collectTopLevelDeclarations(endpoints);
        try self.collectAllEndpointUses(endpoints);
        try self.renderTopLevelDeclarations(endpoints);
        try self.buildAllMethodEntries(endpoints);

        var res: ArrayList(u8) = .empty;

        {
            const Entry = struct {
                const E = @This();

                type_name: []const u8,
                ts: []const u8,

                fn sort(_: void, lhs: E, rhs: E) bool {
                    return std.ascii.lessThanIgnoreCase(lhs.type_name, rhs.type_name);
                }
            };
            var entries: ArrayList(Entry) = .empty;
            defer entries.deinit(self.arena_alloc);

            var iter = self.registry.declarations.iterator();
            while (iter.next()) |entry| {
                const result = entry.value_ptr.rendered orelse continue;
                try entries.append(self.arena_alloc, .{
                    .type_name = Registry.shortName(entry.key_ptr.*),
                    .ts = try allocPrint(
                        self.arena_alloc,
                        "export type {s} =\n{s}\n\n",
                        .{ Registry.shortName(entry.key_ptr.*), try self.render(result) },
                    ),
                });
            }

            std.mem.sort(Entry, entries.items, {}, Entry.sort);
            for (entries.items) |entry| {
                try res.appendSlice(self.arena_alloc, entry.ts);
            }
        }

        try res.appendSlice(self.arena_alloc, "export type Spec = {");

        inline for (@typeInfo(Method).@"enum".fields) |m| {
            const method: Method = @enumFromInt(m.value);
            var endpoints_data = self.endpointDataForMethod(method);

            try res.appendSlice(
                self.arena_alloc,
                try allocPrint(
                    self.arena_alloc,
                    "{s}: {{",
                    .{try std.ascii.allocUpperString(self.arena_alloc, m.name)},
                ),
            );

            var iter = endpoints_data.iterator();
            while (iter.next()) |endpoint| {
                try res.appendSlice(
                    self.arena_alloc,
                    try allocPrint(self.arena_alloc, "\"{s}\": {{\n", .{endpoint.key_ptr.*}),
                );

                if (endpoint.value_ptr.query_params) |query_params| {
                    try res.appendSlice(self.arena_alloc, query_params);
                }

                if (endpoint.value_ptr.body) |body| {
                    try res.appendSlice(self.arena_alloc, body);
                } else if (method != .get) {
                    try res.appendSlice(self.arena_alloc, "  body?: never,\n");
                }

                if (endpoint.value_ptr.response) |response| {
                    try res.appendSlice(
                        self.arena_alloc,
                        try allocPrint(self.arena_alloc, "response: {s},\n", .{response}),
                    );
                }
                try res.appendSlice(self.arena_alloc, "}\n");
            }
            try res.appendSlice(self.arena_alloc, "},\n");
        }

        try res.appendSlice(self.arena_alloc, "};");

        return res.toOwnedSlice(self.arena_alloc);
    }

    /// Registers named declarations before choosing their endpoint usage.
    fn collectTopLevelDeclarations(self: *Self, comptime endpoints: []const EndpointDef) !void {
        inline for (endpoints) |endpoint| {
            _, const EndpointType = endpoint;
            inline for (@typeInfo(EndpointType).@"struct".decls) |decl| {
                const field = @field(EndpointType, decl.name);
                if (comptime shouldDeclareTopLevel(field)) {
                    try self.registry.declare(field);
                }
            }
        }
    }

    fn collectAllEndpointUses(self: *Self, comptime endpoints: []const EndpointDef) !void {
        inline for (endpoints) |endpoint| {
            try self.collectEndpointUses(endpoint);
        }
    }

    /// Renders declarations once after endpoint collection establishes their usage.
    fn renderTopLevelDeclarations(self: *Self, comptime endpoints: []const EndpointDef) !void {
        inline for (endpoints) |endpoint| {
            const endpoint_path, const EndpointType = endpoint;
            inline for (@typeInfo(EndpointType).@"struct".decls) |decl| {
                const D = @field(EndpointType, decl.name);
                comptime if (!shouldDeclareTopLevel(D)) continue;

                const type_id = @typeName(D);
                if (self.registry.isReferenced(type_id)) {
                    const usage: TypeUsage = switch (@typeInfo(D)) {
                        // Enums render identically in both contexts.
                        .@"enum" => .body,
                        .@"struct", .@"union" => self.registry.usageFor(type_id) orelse return error.MissingTypeUsage,
                        else => unreachable,
                    };
                    const result = self.buildTopLevelType(D, usage) catch |err| {
                        std.log.info("Endpoint: {s} - Type: {s}", .{ endpoint_path, decl.name });
                        return err;
                    };
                    try self.registry.setRendered(type_id, result);
                }
            }
        }
    }

    fn buildAllMethodEntries(self: *Self, comptime endpoints: []const EndpointDef) !void {
        inline for (endpoints) |endpoint| {
            try self.buildMethodEntries(endpoint);
        }
    }

    fn endpointDataForMethod(self: *Self, method: Method) *StringArrayHashMap(EndpointData) {
        return switch (method) {
            .get => &self.get_endpoints,
            .post => &self.post_endpoints,
            .put => &self.put_endpoints,
            .patch => &self.patch_endpoints,
            .delete => &self.delete_endpoints,
        };
    }

    fn render(self: *Self, result: TypeDescriptor) ![]const u8 {
        return result.render(self.arena_alloc);
    }

    fn allocateExpr(self: *Self, expr: TypeExpr) !*const TypeExpr {
        return TypeExpr.allocate(self.arena_alloc, expr);
    }

    fn parenthesize(self: *Self, expr: *const TypeExpr) !*const TypeExpr {
        return self.allocateExpr(.{ .parens = expr });
    }

    fn exclusiveOr(self: *Self, left: *const TypeExpr, right: *const TypeExpr) !*const TypeExpr {
        return self.allocateExpr(try TypeExpr.makeGeneric(
            self.arena_alloc,
            "XOR",
            &.{ left, right },
        ));
    }

    fn anyOf(self: *Self, expr: TypeExpr) !TypeExpr {
        return TypeExpr.makeGeneric(
            self.arena_alloc,
            "AnyOf",
            &.{try self.allocateExpr(expr)},
        );
    }

    fn buildTopLevelType(self: *Self, comptime T: type, usage: TypeUsage) !TypeDescriptor {
        const context: TypeGenerationContext = .{ .usage = usage };
        return switch (@typeInfo(T)) {
            .@"struct" => |s| if (usage == .query_params)
                self.buildQueryStructType(T, s, context)
            else
                self.buildStructType(@typeName(T), s, context),
            .@"enum" => |e| .{ .expr = .{ .verbatim = try self.buildEnumType(e) } },
            .@"union" => |u| if (usage == .query_params or isLiftableUnion(T))
                self.buildFlatUnionType(u, context)
            else
                self.buildUnionObjectType(u, context),
            else => error.InvalidTopLevelType,
        };
    }

    fn buildMethodEntries(self: *Self, endpoint: EndpointDef) !void {
        const endpoint_path, const EndpointType = endpoint;
        inline for (@typeInfo(EndpointType).@"struct".decls) |decl| {
            const decl_info = @typeInfo(@TypeOf(@field(EndpointType, decl.name)));
            if (decl_info != .@"fn") continue;
            // Find get/post/put/patch/delete functions
            self.buildMethodEntry(decl, decl_info.@"fn", endpoint_path) catch |err| {
                std.log.info(
                    "Endpoint: {s} - Type: {s}",
                    .{ endpoint_path, decl.name },
                );
                return err;
            };
        }
    }

    fn collectEndpointUses(self: *Self, endpoint: EndpointDef) !void {
        const endpoint_path, const EndpointType = endpoint;
        inline for (@typeInfo(EndpointType).@"struct".decls) |decl| {
            const decl_info = @typeInfo(@TypeOf(@field(EndpointType, decl.name)));
            if (decl_info != .@"fn") continue;
            self.collectFunctionUses(decl, decl_info.@"fn") catch |err| {
                std.log.info(
                    "Endpoint: {s} - Type: {s}",
                    .{ endpoint_path, decl.name },
                );
                return err;
            };
        }
    }

    /// Builds `query_params` and `body`, if present.
    fn buildRequestContextFields(
        self: *Self,
        method: Method,
        comptime Context: type,
        endpoint_path: []const u8,
    ) !void {
        var endpoints_data = self.endpointDataForMethod(method);
        var res = try endpoints_data.getOrPutValue(endpoint_path, .{});

        if (comptime @hasField(Context, "body")) {
            const Body = @FieldType(Context, "body");
            const body_info = @typeInfo(Body);
            if (body_info == .pointer and body_info.pointer.child == u8) {
                res.value_ptr.body = "body: BodyInit\n";
            } else {
                res.value_ptr.body = try self.renderRequestContextField(
                    "body",
                    try self.buildTypeForUsage(Body, .body),
                );
            }
        }

        if (comptime @hasField(Context, "query_params")) {
            res.value_ptr.query_params = try self.renderRequestContextField(
                "queryParams",
                try self.buildQueryParamsType(@FieldType(Context, "query_params")),
            );
        }
    }

    fn renderRequestContextField(self: *Self, name: []const u8, built_type: TypeDescriptor) ![]const u8 {
        return allocPrint(
            self.arena_alloc,
            "{s}{s}: {s}\n",
            .{ name, if (built_type.optional) "?" else "", try self.render(built_type) },
        );
    }

    fn buildMethodEntry(
        self: *Self,
        comptime decl: Type.Declaration,
        comptime F: Type.Fn,
        comptime endpoint_path: []const u8,
    ) !void {
        const method = (comptime stringToEnum(Method, decl.name)) orelse return;
        var endpoints_data = self.endpointDataForMethod(method);

        const first_param = F.params[0].type orelse unreachable;
        const Context = @typeInfo(first_param).pointer.child;
        try self.buildRequestContextFields(method, Context, endpoint_path);

        const response = responseType(F);
        var entry = try endpoints_data.getOrPutValue(endpoint_path, .{});
        entry.value_ptr.response = try self.render(try self.buildTypeForUsage(response, .body));
    }

    fn collectFunctionUses(self: *Self, comptime decl: Type.Declaration, comptime F: Type.Fn) !void {
        _ = (comptime stringToEnum(Method, decl.name)) orelse return;

        const first_param = F.params[0].type orelse unreachable;
        const Context = @typeInfo(first_param).pointer.child;

        if (comptime @hasField(Context, "body")) {
            try self.recordTypeUses(@FieldType(Context, "body"), .body);
        }

        if (comptime @hasField(Context, "query_params")) {
            try self.recordTypeUses(@FieldType(Context, "query_params"), .query_params);
        }

        try self.recordTypeUses(responseType(F), .body);
    }

    fn recordTypeUses(self: *Self, comptime T: type, usage: TypeUsage) !void {
        if (comptime typescriptRepr(T)) |repr_type| {
            return self.recordTypeUses(repr_type, usage);
        }

        const type_id = @typeName(T);
        switch (@typeInfo(T)) {
            .@"struct", .@"union" => {
                if (self.registry.isDeclared(type_id)) {
                    const was_referenced = self.registry.isReferenced(type_id);
                    try self.registry.recordUsage(type_id, usage);

                    // A declaration may refer to other top-level declarations.
                    // Walk it the first time it is reached so those dependencies are emitted too.
                    // The guard also prevents infinite recursion.
                    if (was_referenced) return;
                }
            },
            .@"enum" => {
                self.registry.recordReference(type_id);
                return;
            },
            .pointer => |pointer| return self.recordTypeUses(pointer.child, usage),
            .optional => |optional| return self.recordTypeUses(optional.child, usage),
            else => return,
        }

        _ = try self.buildType(T, .{ .usage = usage });

        switch (@typeInfo(T)) {
            .@"struct" => |s| {
                inline for (s.fields) |field| {
                    const skips_query_recursion = comptime hasParamParse(field.type);
                    if (usage != .query_params or !skips_query_recursion) {
                        try self.recordTypeUses(field.type, usage);
                    }
                }
            },
            .@"union" => |u| {
                inline for (u.fields) |field| {
                    try self.recordTypeUses(field.type, usage);
                }
            },
            else => unreachable,
        }
    }

    fn responseBodyType(comptime Response: type) type {
        if (@hasField(Response, "body")) {
            return @typeInfo(@FieldType(Response, "body")).optional.child;
        }
        @compileError("Return type of endpoint fns must be Response(T)");
    }

    fn responseType(comptime F: Type.Fn) type {
        return switch (@typeInfo(F.return_type.?)) {
            .error_union => |error_union| responseBodyType(error_union.payload),
            .@"struct" => responseBodyType(F.return_type.?),
            else => @compileError("Invalid fn return type"),
        };
    }

    fn buildStructType(
        self: *Self,
        struct_name: []const u8,
        S: Type.Struct,
        context: TypeGenerationContext,
    ) !TypeDescriptor {
        // Find adjacent union ahead of time
        var adjacent_union: ?AdjacentUnion = null;
        {
            inline for (S.fields) |field| {
                const info = @typeInfo(field.type);
                if (info != .@"union") continue;

                var union_repr: ?UnionRepr = null;
                if (comptime @hasDecl(field.type, "_repr")) {
                    const repr = @field(field.type, "_repr");
                    if (@TypeOf(repr) == UnionRepr and repr == .adjacently) {
                        union_repr = repr;
                    }
                }

                if (union_repr) |repr| {
                    if (adjacent_union != null) {
                        std.log.err(
                            "Container cannot have more than one adjacent union discriminator fields.",
                            .{},
                        );
                        return error.MultipleAdjacentUnions;
                    }
                    adjacent_union = AdjacentUnion{
                        .discriminator = repr.adjacently.discriminator,
                        .name = @typeName(field.type),
                    };
                }
            }
        }

        // Check that discriminator is present as a field in the struct.
        if (adjacent_union) |au| {
            var found_required_field = false;
            inline for (S.fields) |f| {
                if (strEqls(f.name, au.discriminator)) {
                    found_required_field = true;
                    break;
                }
            }

            if (!found_required_field) {
                std.log.err(
                    "Struct {s} with adjacently tagged union requires field {s} to be present, but was missing.",
                    .{ au.name, au.discriminator },
                );
                return error.MissingRequiredField;
            }

            // Extract out to top level type
            const short_struct_name = Registry.shortName(struct_name);
            try self.registry.setRendered(struct_name, try self.buildAdjacentUnionStructType(S, au, context));

            return TypeDescriptor{
                .optional = false,
                .expr = .{ .named = short_struct_name },
            };
        }

        // Default struct parsing if there's no adjacent union present as a field
        return self.buildObjectType(S, context);
    }

    fn buildObjectType(self: *Self, S: Type.Struct, context: TypeGenerationContext) !TypeDescriptor {
        var all_optional = true;
        var fields: ArrayList(TypeExpr.Field) = .empty;
        inline for (S.fields) |field| {
            // Ensure Optionals have default values
            if (comptime isOptional(field.type) and field.defaultValue() == null) {
                std.log.info(
                    "Optional type \"{s}\" must have a provided default value: {s}",
                    .{ field.name, @typeName(field.type) },
                );
                return error.OptionalMissingDefault;
            }

            const built_type = try self.buildType(field.type, context);

            // TODO: This is a weird bug in defaultValue I had to work around...
            if (comptime strEqls(field.name, "_is_finished")) continue;

            const optional = if (field.defaultValue()) |_| true else built_type.optional;

            all_optional = all_optional and optional;
            try fields.append(self.arena_alloc, .{
                .name = field.name,
                .expr = try self.allocateExpr(built_type.expr),
                .optional = optional,
            });
        }

        return .{
            .optional = all_optional,
            .expr = .{ .object = try fields.toOwnedSlice(self.arena_alloc) },
        };
    }

    /// Helper function for adjacent unions
    fn buildUnionObjectType(self: *Self, U: Type.Union, context: TypeGenerationContext) !TypeDescriptor {
        var all_optional = true;
        var fields: ArrayList(TypeExpr.Field) = .empty;
        inline for (U.fields) |field| {
            const built_type = try self.buildType(field.type, context);
            all_optional = all_optional and built_type.optional;
            try fields.append(self.arena_alloc, .{
                .name = field.name,
                .expr = try self.allocateExpr(built_type.expr),
                .optional = built_type.optional,
            });
        }

        return .{
            .optional = all_optional,
            .expr = .{ .object = try fields.toOwnedSlice(self.arena_alloc) },
        };
    }

    fn buildAdjacentUnionStructType(
        self: *Self,
        S: Type.Struct,
        adjacent_union: AdjacentUnion,
        context: TypeGenerationContext,
    ) !TypeDescriptor {
        const union_short_name = Registry.shortName(adjacent_union.name);

        var res: ArrayList(u8) = .empty;
        try res.appendSlice(self.arena_alloc, "{\n");
        try res.appendSlice(self.arena_alloc, try allocPrint(
            self.arena_alloc,
            "[K in keyof {s}]: {{\n",
            .{union_short_name},
        ));

        inline for (S.fields) |f| {
            if (strEqls(f.name, adjacent_union.discriminator)) {
                try res.appendSlice(self.arena_alloc, f.name);
                try res.appendSlice(self.arena_alloc, ": K\n");
            } else if (strEqls(@typeName(f.type), adjacent_union.name)) {
                const field_info = @typeInfo(f.type);
                if (field_info != .@"union") {
                    return error.InvalidAdjacentUnionType;
                }

                if (self.registry.reference(adjacent_union.name) == null) {
                    try self.registry.setRendered(
                        adjacent_union.name,
                        try self.buildUnionObjectType(field_info.@"union", context),
                    );
                }
                try res.appendSlice(
                    self.arena_alloc,
                    try allocPrint(self.arena_alloc, "{s}: {s}[K]", .{ f.name, union_short_name }),
                );
            } else {
                try res.appendSlice(self.arena_alloc, f.name);
                const built_type = try self.buildType(f.type, context);
                if (built_type.optional) {
                    try res.appendSlice(self.arena_alloc, "?: ");
                } else {
                    try res.appendSlice(self.arena_alloc, ": ");
                }
                try res.appendSlice(self.arena_alloc, try self.render(built_type));
                try res.appendSlice(self.arena_alloc, "\n");
            }
        }

        try res.appendSlice(self.arena_alloc, try allocPrint(
            self.arena_alloc,
            "}};\n}}[keyof {s}];\n",
            .{union_short_name},
        ));

        return .{ .expr = .{ .verbatim = try res.toOwnedSlice(self.arena_alloc) } };
    }

    fn buildEnumType(self: *Self, E: Type.Enum) ![]const u8 {
        var res: ArrayList(u8) = .empty;
        try res.appendSlice(self.arena_alloc, " | (\n");
        inline for (E.fields) |field| {
            try res.appendSlice(self.arena_alloc, " | ");

            try res.appendSlice(self.arena_alloc, "\"");
            try res.appendSlice(self.arena_alloc, field.name);
            try res.appendSlice(self.arena_alloc, "\"");
        }
        try res.appendSlice(self.arena_alloc, "\n)");
        return res.toOwnedSlice(self.arena_alloc);
    }

    fn buildUnionType(self: *Self, U: Type.Union, T: type, context: TypeGenerationContext) ![]const u8 {
        var res: ArrayList(u8) = .empty;

        // Special case for Optional(T)
        if (comptime isOptional(T)) {
            const built_type = try self.buildType(@FieldType(T, "value"), context);
            try res.appendSlice(self.arena_alloc, try self.render(built_type));
            return res.toOwnedSlice(self.arena_alloc);
        }

        const union_repr: ?UnionRepr = blk: {
            if (context.usage == .query_params) break :blk .untagged;
            inline for (U.decls) |decl| {
                if (comptime strEqls(decl.name, "_repr")) {
                    break :blk @field(T, decl.name);
                }
            }
            break :blk null;
        };

        if (union_repr) |repr| {
            switch (repr) {
                .external => {
                    return error.WeWerentUsingThisWhenIWroteTheTypegenLol;
                },
                .internal => {
                    const disc: []const u8 = repr.internal.discriminator;
                    inline for (U.fields) |field| {
                        try res.appendSlice(self.arena_alloc, try allocPrint(
                            self.arena_alloc,
                            "\n | {{{s}: \"{s}\"; ",
                            .{ disc, field.name },
                        ));

                        const field_info: Type = @typeInfo(field.type);
                        if (field_info != .@"struct") return error.InvalidUnionRepr;

                        inline for (field_info.@"struct".fields) |f| {
                            const built_type = try self.buildType(f.type, context);
                            try res.appendSlice(self.arena_alloc, try allocPrint(
                                self.arena_alloc,
                                "{s}{s}: {s}; ",
                                .{
                                    f.name,
                                    if (built_type.optional or f.defaultValue() != null) "?" else "",
                                    try self.render(built_type),
                                },
                            ));
                        }
                        try res.appendSlice(self.arena_alloc, " }");
                    }
                },
                .adjacently => {
                    try res.appendSlice(self.arena_alloc, try allocPrint(
                        self.arena_alloc,
                        "{{\n [K in keyof {s}]: {{\n",
                        .{Registry.shortName(@typeName(T))},
                    ));
                    const disc: []const u8 = repr.adjacently.discriminator;
                    inline for (U.fields) |field| {
                        try res.appendSlice(self.arena_alloc, try allocPrint(
                            self.arena_alloc,
                            "\n | {{{s}: \"{s}\"; ",
                            .{ disc, field.name },
                        ));

                        const field_info: Type = @typeInfo(field.type);
                        if (field_info != .@"struct") return error.InvalidUnionRepr;

                        inline for (field_info.@"struct".fields) |f| {
                            const built_type = try self.buildType(f.type, context);
                            try res.appendSlice(self.arena_alloc, try allocPrint(
                                self.arena_alloc,
                                "{s}{s}: {s}; ",
                                .{
                                    f.name,
                                    if (built_type.optional or f.defaultValue() != null) "?" else "",
                                    try self.render(built_type),
                                },
                            ));
                        }
                        try res.appendSlice(self.arena_alloc, " }");
                    }
                },
                .untagged => {
                    // Get the type of each enum state, join them together
                    var first = true;
                    inline for (U.fields) |field| {
                        if (first) {
                            first = false;
                        } else {
                            try res.appendSlice(self.arena_alloc, " | ");
                        }

                        if (@typeInfo(field.type) == .void) {
                            try res.print(self.arena_alloc, "\"{s}\"", .{field.name});
                        } else {
                            const ident = try self.render(try self.buildType(field.type, context));
                            try res.appendSlice(self.arena_alloc, ident);
                        }
                    }
                },
            }
        } else {
            std.log.err("{s} is missing a _repr declaration (must be public)", .{@typeName(T)});
            return error.MissingTaggedUnionRepr;
        }

        return res.toOwnedSlice(self.arena_alloc);
    }

    /// Wraps a type in the utility required by its constraints.
    fn applyConstraints(self: *Self, comptime constraints: types.Constraints, res: TypeDescriptor) !TypeDescriptor {
        if (comptime constraints.any_of) {
            return .{
                .expr = try self.anyOf(res.expr),
                .optional = false,
            };
        }
        return res;
    }

    /// A `paramParse` query value is always a string on the wire.
    fn buildQueryLeafType(self: *Self, comptime T: type, context: TypeGenerationContext) !TypeDescriptor {
        if (comptime hasParamParse(T)) return .{ .expr = .{ .named = "string" } };
        return self.buildType(T, context);
    }

    /// Query params flatten structs and tagged unions into their wire keys.
    fn buildQueryParamsType(self: *Self, comptime T: type) !TypeDescriptor {
        return self.buildQueryType(T, .{ .usage = .query_params });
    }

    fn buildQueryType(self: *Self, comptime T: type, context: TypeGenerationContext) !TypeDescriptor {
        const type_id = @typeName(T);
        const ts_name = Registry.shortName(type_id);
        const is_top_level = self.registry.isDeclared(type_id);
        const info = @typeInfo(T);
        if (comptime info == .@"struct") {
            if (is_top_level) {
                return .{ .expr = .{ .named = ts_name }, .optional = comptime queryParamsOptional(T) };
            }
            return self.buildQueryStructType(T, info.@"struct", context);
        }

        // Enums have the same string-literal union representation in a body and
        // in query params, so they do not need a usage-specific rendering path.
        if (comptime info == .@"enum") {
            if (is_top_level) {
                return .{ .expr = .{ .named = ts_name }, .optional = comptime queryParamsOptional(T) };
            }
        }

        if (is_top_level) {
            return .{ .expr = .{ .named = ts_name }, .optional = comptime queryParamsOptional(T) };
        }

        if (comptime info == .@"union" and !isOptional(T)) {
            const res = try self.buildFlatUnionType(info.@"union", context);
            return .{ .expr = res.expr, .optional = comptime queryParamsOptional(T) };
        }
        return self.buildType(T, context);
    }

    /// Whether a `query_params` type may be omitted entirely.
    /// It is optional when it has zero required leaf keys (`getRequiredKeyCount`)
    /// and carries no `any_of` constraint (which forces at least one key to be present).
    /// A union counts as zero when it has an all-optional fallback variant,
    /// since that variant can be satisfied with no keys.
    fn queryParamsOptional(comptime T: type) bool {
        comptime {
            const has_any_of = switch (@typeInfo(T)) {
                .@"struct", .@"union", .@"enum" => @hasDecl(T, "constraints") and T.constraints.any_of,
                else => false,
            };
            return !has_any_of and getRequiredKeyCount(T) == 0;
        }
    }

    /// Parses a union into a flat TS union.
    /// A scalar (or `paramParse`/void) variant uses the variant name as its single key,
    /// and a plain struct variant is flattened into its leaf keys.
    fn buildFlatUnionType(self: *Self, U: Type.Union, context: TypeGenerationContext) !TypeDescriptor {
        var variants = TypeExpr.Components.init(self.arena_alloc);
        inline for (U.fields) |field| {
            const info = @typeInfo(field.type);
            if (field.type == void) {
                const variant = try TypeExpr.singleField(
                    self.arena_alloc,
                    field.name,
                    .{ .verbatim = "\"\"" },
                );
                try variants.append(variant);
            } else if (info == .@"struct" and !hasParamParse(field.type)) {
                const variant: TypeExpr =
                    (try self.buildQueryObjectType(field.type, info.@"struct", context)) orelse .{ .object = &.{} };

                try variants.append(inlineSingleFieldObject(variant));
            } else {
                // Scalar / array / enum / paramParse struct: variant name is key.
                const ident = try self.buildQueryLeafType(field.type, context);
                const variant = try TypeExpr.singleField(self.arena_alloc, field.name, ident.expr);
                try variants.append(variant);
            }
        }
        // A union always needs at least one matching key, so it is required.
        return .{ .expr = try self.renderExclusiveUnion(variants.items.items) };
    }

    /// Combines variants so exactly one may be present.
    fn renderExclusiveUnion(self: *Self, variants: []const *const TypeExpr) !TypeExpr {
        if (variants.len == 0) return .{ .object = &.{} };

        var result = try self.parenthesize(variants[variants.len - 1]);
        var index = variants.len - 1;
        while (index > 0) {
            index -= 1;
            const left = try self.parenthesize(variants[index]);
            result = try self.exclusiveOr(left, result);
        }
        return result.*;
    }

    fn inlineSingleFieldObject(expr: TypeExpr) TypeExpr {
        return switch (expr) {
            .object => |fields| if (fields.len == 1 and !fields[0].optional)
                .{ .inline_object = fields }
            else
                expr,
            else => expr,
        };
    }

    /// A `query_params` struct represented as a flat structure.
    /// `independent` - Leaves that are not in any kind of group.
    /// `groups` - Each set of leaves that must exist together in a group (for `AllOf`).
    const FlatStruct = struct {
        independent: ArrayList(FlatLeaf) = .empty,
        groups: ArrayList(ArrayList(FlatLeaf)) = .empty,
    };

    /// Builds the flat TS shape for a struct query parameter.
    ///
    /// Returns null when the struct contributes no keys (no leaves and no `any_of`),
    /// so it can be omitted from the generated types.
    ///
    /// Union fields are skipped here (see `collectFlatLeaves`) and handled by the caller.
    fn buildQueryObjectType(
        self: *Self,
        comptime T: type,
        S: Type.Struct,
        context: TypeGenerationContext,
    ) !?TypeExpr {
        var flat_struct: FlatStruct = .{};
        try self.collectFlatLeaves(&flat_struct, S, false, null, context);

        const any_of = comptime blk: {
            if (@hasDecl(T, "constraints")) break :blk T.constraints.any_of;
            break :blk false;
        };

        var all: ArrayList(FlatLeaf) = .empty;
        try all.appendSlice(self.arena_alloc, flat_struct.independent.items);
        for (flat_struct.groups.items) |group| {
            try all.appendSlice(self.arena_alloc, group.items);
        }

        if (all.items.len == 0 and !any_of) return null;

        // No groups means the full object is the base, optionally wrapped in AnyOf
        if (flat_struct.groups.items.len == 0) {
            const full = try self.renderLeafObject(all.items);
            if (any_of) {
                return try self.anyOf(full);
            }
            return full;
        }

        // One `XOR<present, {}>` per group, the independent keys as a plain object,
        // and AnyOf over everything if the constraint is set.
        var components = TypeExpr.Components.init(self.arena_alloc);

        // A group always has >= 1 required key (all-optional groups flatten inline)
        for (flat_struct.groups.items) |group| {
            const present = try self.allocateExpr(try self.renderLeafObject(group.items));
            const absent = try self.allocateExpr(.{ .object = &.{} });
            try components.appendRef(try self.exclusiveOr(present, absent));
        }

        if (flat_struct.independent.items.len > 0) {
            try components.append(try self.renderLeafObject(flat_struct.independent.items));
        }

        if (any_of) {
            const full = try self.allocateExpr(try self.renderLeafObject(all.items));
            try components.append(try self.anyOf(full.*));
        }

        return try components.intoIntersection();
    }

    /// Emits the flat TS shape for a query param struct which contains lifted union fields.
    /// Each union field's variant alternatives live directly in the flat key space,
    /// so the result is the intersection of every union's flat shape,
    /// with the base object built from the remaining fields.
    fn buildQueryStructType(
        self: *Self,
        comptime T: type,
        S: Type.Struct,
        context: TypeGenerationContext,
    ) !TypeDescriptor {
        var components = TypeExpr.Components.init(self.arena_alloc);

        inline for (S.fields) |field| {
            if (comptime isLiftableUnion(field.type)) {
                const shape = try self.buildFlatUnionType(@typeInfo(field.type).@"union", context);
                try components.appendRef(try self.parenthesize(
                    try self.allocateExpr(shape.expr),
                ));
            }
        }

        // Base object from the non-union fields (collectFlatLeaves skips unions).
        if (try self.buildQueryObjectType(T, S, context)) |base| {
            try components.append(base);
        }

        return .{
            .expr = try components.intoIntersection(),
            .optional = comptime queryParamsOptional(T),
        };
    }

    /// Recursively flattens a struct into leaf keys, hoisting nested plain structs.
    /// `parent_optional` - Propagates optionality through Optional (or defaulted) wrappers.
    /// `current_group` - The open group that leaves will join.
    ///   It is opened by an Optional nested struct that has at least one required key.
    ///   While a group is open, every key (required or optional) joins it so the whole struct is present-or-absent.
    fn collectFlatLeaves(
        self: *Self,
        flat_struct: *FlatStruct,
        S: Type.Struct,
        parent_optional: bool,
        current_group: ?*ArrayList(FlatLeaf),
        context: TypeGenerationContext,
    ) !void {
        inline for (S.fields) |field| {
            // Non-Optional tagged unions are lifted into the flat key space by the caller.
            if (comptime isLiftableUnion(field.type)) continue;

            const is_optional = comptime isOptional(field.type);
            const introduces_optional = is_optional or field.defaultValue() != null;
            const T = if (is_optional) field.type.childType() else field.type;
            const info = @typeInfo(T);
            const wrapper_optional = parent_optional or introduces_optional;

            if (info == .@"struct" and !hasParamParse(T)) {
                // An optional nested struct that has at least one required key is a "gated group":
                // at runtime, if any of its keys are present, the required ones are enforced.
                // Otherwise, the whole struct may be absent.
                // That is exactly `XOR<present, {}>`, so we collect all of its keys into a single group.
                //
                // We reset `parent_optional` to false inside the group
                // so required keys stay required in the "present" shape.
                // The group's own optionality is carried by the XOR-with-empty wrapper,
                // not by marking every key optional.
                if (introduces_optional and current_group == null and comptime getRequiredKeyCount(T) > 0) {
                    var group: ArrayList(FlatLeaf) = .empty;
                    try self.collectFlatLeaves(flat_struct, info.@"struct", false, &group, context);
                    try flat_struct.groups.append(self.arena_alloc, group);
                } else {
                    // Required nested struct, an all-optional nested struct,
                    // or one already inside a group: flatten in declaration order.
                    try self.collectFlatLeaves(flat_struct, info.@"struct", wrapper_optional, current_group, context);
                }
            } else {
                const ident = try self.buildQueryLeafType(T, context);
                const leaf: FlatLeaf = .{
                    .name = field.name,
                    .expr = try self.allocateExpr(ident.expr),
                    .optional = wrapper_optional or ident.optional,
                };

                const list = if (current_group) |g| g else &flat_struct.independent;
                try list.append(self.arena_alloc, leaf);
            }
        }
    }

    fn renderLeafObject(self: *Self, leaves: []const FlatLeaf) !TypeExpr {
        var fields: ArrayList(TypeExpr.Field) = .empty;
        for (leaves) |leaf| {
            try fields.append(self.arena_alloc, .{
                .name = leaf.name,
                .expr = leaf.expr,
                .optional = leaf.optional,
            });
        }
        return .{ .object = try fields.toOwnedSlice(self.arena_alloc) };
    }

    fn buildTypeForUsage(self: *Self, T: type, usage: TypeUsage) !TypeDescriptor {
        return self.buildType(T, .{ .usage = usage });
    }

    fn buildType(self: *Self, T: type, context: TypeGenerationContext) !TypeDescriptor {
        if (comptime typescriptRepr(T)) |repr_type| {
            return self.buildType(repr_type, context);
        }

        const type_info = @typeInfo(T);
        switch (type_info) {
            .int, .float => return .{ .expr = .{ .named = "number" } },
            .bool => return .{ .expr = .{ .named = "boolean" } },
            .type, .void => return .{ .expr = .{ .named = @typeName(T) } },
            .pointer => {
                if (type_info.pointer.child == u8) {
                    return .{ .expr = .{ .named = "string" } };
                } else {
                    const child = try self.buildType(type_info.pointer.child, context);
                    return .{ .expr = .{ .array = try self.allocateExpr(child.expr) } };
                }
            },
            .@"struct" => {
                const type_id = @typeName(T);

                const res: TypeDescriptor = if (self.registry.reference(type_id)) |gen| blk: {
                    break :blk .{ .expr = .{ .named = Registry.shortName(type_id) }, .optional = gen.optional };
                } else try self.buildStructType(type_id, type_info.@"struct", context);

                // Wrap the emitted object in the matching TS utility type(s)
                // if any constraints should be applied.
                if (@hasDecl(T, "constraints")) {
                    return try self.applyConstraints(T.constraints, res);
                }
                return res;
            },
            .@"enum" => {
                const type_id = @typeName(T);
                if (self.registry.reference(type_id)) |gen| {
                    return .{ .expr = .{ .named = Registry.shortName(type_id) }, .optional = gen.optional };
                }
                return .{ .expr = .{ .verbatim = try self.buildEnumType(type_info.@"enum") } };
            },
            .@"union" => {
                const type_id = @typeName(T);
                if (self.registry.reference(type_id)) |gen| {
                    return .{ .expr = .{ .named = Registry.shortName(type_id) }, .optional = gen.optional };
                }
                return .{ .expr = .{ .verbatim = try self.buildUnionType(type_info.@"union", T, context) } };
            },
            .optional => {
                return .{
                    // "optional" in zig means nullable, not actually optional.
                    // This means the value could still be required, but could be set to null.
                    .optional = false,
                    .expr = .{ .nullable = try self.allocateExpr(
                        (try self.buildType(type_info.optional.child, context)).expr,
                    ) },
                };
            },
            .@"opaque" => {
                return .{ .expr = .{ .named = @typeName(T) } };
            },
            else => {
                std.log.err("Unhandled identifier: {s}", .{@tagName(type_info)});
                return error.Unreachable;
            },
        }
    }

    fn isInlinedStruct(struct_name: []const u8) bool {
        // NOTE: There doesn't seem to be a better way of doing this, currently
        return std.mem.containsAtLeast(u8, struct_name, 1, "__struct_");
    }
};

/// Returns the TypeScript wire type declared by `T._repr`, or `null` when `T` does not have a `_repr` decl.
///
/// A `_repr` must be either a `type` or a `UnionRepr` (unions).
fn typescriptRepr(comptime T: type) ?type {
    switch (@typeInfo(T)) {
        .@"struct", .@"union", .@"enum", .@"opaque" => {},
        else => return null,
    }
    if (!@hasDecl(T, "_repr")) return null;

    const Repr = @TypeOf(T._repr);
    if (Repr == UnionRepr) {
        if (@typeInfo(T) == .@"union") return null;
        @compileError("_repr on " ++ @typeName(T) ++ " must be a type, not UnionRepr");
    }
    if (Repr != type)
        @compileError("_repr on " ++ @typeName(T) ++ " must be a type or UnionRepr");

    return T._repr;
}

fn shouldDeclareTopLevel(comptime value: anytype) bool {
    if (@TypeOf(value) != type) return false;
    return switch (@typeInfo(value)) {
        .@"struct", .@"enum", .@"union" => typescriptRepr(value) == null,
        else => false,
    };
}

const TypeScriptOpaqueId = struct {
    bytes: [16]u8,

    pub const _repr: type = []const u8;
};

test "buildTypeForUsage: a type _repr overrides structural type generation" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const result = try type_generator.buildTypeForUsage(TypeScriptOpaqueId, .body);
    try expectContent("string", try result.render(arena.allocator()));
}

test "generateTypes: _repr types generate their declared TypeScript wire type" {
    const TypeScriptOpaqueIdEndpoint = struct {
        const Context = struct {
            body: struct {
                id: TypeScriptOpaqueId,
            },
        };
        const Response = struct { body: ?TypeScriptOpaqueId = null };

        pub fn post(_: *Context) Response {
            return .{};
        }
    };

    const alloc = std.testing.allocator;
    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const endpoints = [_]EndpointDef{.{ "/opaque-id", TypeScriptOpaqueIdEndpoint }};
    const output = try type_generator.generateTypes(&endpoints);

    try expectContent(
        \\export type Spec = {
        \\  GET: {},
        \\  POST: {
        \\    "/opaque-id": {
        \\      body: {
        \\        id: string
        \\      }
        \\      response: string,
        \\    }
        \\  },
        \\  PUT: {},
        \\  PATCH: {},
        \\  DELETE: {},
        \\};
    , output);
}

test "generateTypes: ignores top-level aliases to primitive types" {
    const AliasEndpoint = struct {
        pub const Timestamp = i64;
        const Ctx = struct {};
        const Res = struct { body: ?Timestamp = null };

        pub fn get(_: *Ctx) Res {
            return .{};
        }
    };

    var arena = ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const endpoints = [_]EndpointDef{.{ "/timestamp", AliasEndpoint }};
    const output = try type_generator.generateTypes(&endpoints);

    try std.testing.expect(std.mem.indexOf(u8, output, "export type Timestamp") == null);
    try std.testing.expect(std.mem.indexOf(u8, output, "response: number") != null);
}

test "generateTypes: omits unused public declarations" {
    const Endpoint = struct {
        pub const UnusedStruct = struct { id: i32 };
        pub const UnusedEnum = enum { enabled, disabled };
        pub const UnusedUnion = union(enum) { by_id: i32 };

        const Ctx = struct {};
        const Res = struct { body: ?bool = null };

        pub fn get(_: *Ctx) Res {
            return .{};
        }
    };

    var arena = ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const endpoints = [_]EndpointDef{.{ "/unused-declarations", Endpoint }};
    const output = try type_generator.generateTypes(&endpoints);

    inline for (.{ "UnusedStruct", "UnusedEnum", "UnusedUnion" }) |name| {
        try std.testing.expect(std.mem.indexOf(u8, output, "export type " ++ name) == null);
    }
    try std.testing.expect(std.mem.indexOf(u8, output, "response: boolean") != null);
}

test "generateTypes: unused declarations do not reserve a TypeScript name" {
    const UnusedEndpoint = struct {
        pub const Shared = struct { ignored: bool };
    };
    const UsedEndpoint = struct {
        pub const Shared = struct { id: i32 };
        const Ctx = struct {};
        const Res = struct { body: ?Shared = null };

        pub fn get(_: *Ctx) Res {
            return .{};
        }
    };

    var arena = ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const endpoints = [_]EndpointDef{
        .{ "/unused-shared", UnusedEndpoint },
        .{ "/used-shared", UsedEndpoint },
    };
    const output = try type_generator.generateTypes(&endpoints);

    try std.testing.expect(std.mem.indexOf(u8, output, "export type Shared =") != null);
    try std.testing.expect(std.mem.indexOf(u8, output, "ignored:") == null);
    try std.testing.expect(std.mem.indexOf(u8, output, "response: Shared") != null);
}

test "generateTypes: exports declarations referenced by an exported endpoint type" {
    const Endpoint = struct {
        pub const Child = struct { id: i32 };
        pub const Parent = struct { child: Child };

        const Ctx = struct {};
        const Res = struct { body: ?Parent = null };

        pub fn get(_: *Ctx) Res {
            return .{};
        }
    };

    var arena = ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const endpoints = [_]EndpointDef{.{ "/nested-declaration", Endpoint }};
    const output = try type_generator.generateTypes(&endpoints);

    try expectContent(
        \\export type Child = {
        \\  id: number
        \\}
        \\export type Parent = {
        \\  child: Child
        \\}
        \\export type Spec = {
        \\  GET: {
        \\    "/nested-declaration": {
        \\      response: Parent,
        \\    }
        \\  },
        \\  POST: {},
        \\  PUT: {},
        \\  PATCH: {},
        \\  DELETE: {},
        \\};
    , output);
}

const DateStrBodyEndpoint = struct {
    const DateStr = Str(Date);

    const Context = struct {
        body: struct {
            date: DateStr,
        },
    };
    const Response = struct { body: ?bool = null };

    pub fn post(_: *Context) Response {
        return .{};
    }
};

const ReprEpochMillis = struct {
    value: DateTime,

    pub const _repr: type = i64;

    pub fn jsonParse(
        allocator: Allocator,
        source: anytype,
        options: std.json.ParseOptions,
    ) !@This() {
        const timestamp = try std.json.innerParse(i64, allocator, source, options);
        return .{ .value = DateTime.fromUnix(timestamp, .milliseconds) };
    }
};

const ReprEpochMillisEndpoint = struct {
    pub const Body = struct {
        timestamp: ReprEpochMillis,
    };

    const Context = struct { body: Body };
    const Response = struct { body: ?ReprEpochMillis = null };

    pub fn post(_: *Context) Response {
        return .{};
    }
};

test "custom _repr structs generate their numeric wire type" {
    const alloc = std.testing.allocator;
    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const endpoints = [_]EndpointDef{.{ "/custom-json-type", ReprEpochMillisEndpoint }};
    const output = try type_generator.generateTypes(&endpoints);

    try expectContent(
        \\export type Body = {
        \\  timestamp: number
        \\}
        \\export type Spec = {
        \\  GET: {},
        \\  POST: {
        \\    "/custom-json-type": {
        \\      body: Body
        \\      response: number,
        \\    }
        \\  },
        \\  PUT: {},
        \\  PATCH: {},
        \\  DELETE: {},
        \\};
    , output);
}

test "Str types generate as strings in request bodies" {
    const alloc = std.testing.allocator;
    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const endpoints = [_]EndpointDef{.{ "/date", DateStrBodyEndpoint }};
    const output = try type_generator.generateTypes(&endpoints);

    try expectContent(
        \\export type Spec = {
        \\  GET: {},
        \\  POST: {
        \\    "/date": {
        \\      body: {
        \\        date: string
        \\      }
        \\      response: boolean,
        \\    }
        \\  },
        \\  PUT: {},
        \\  PATCH: {},
        \\  DELETE: {},
        \\};
    , output);
}

test "required nullable fields" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    const Foo = struct {
        // Optional and nullable
        foo: ?i32 = null,
        // Nullable (not optional)
        bar: ?i32,
        baz: i32 = 0,
    };

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.buildTypeForUsage(Foo, .body);
    try expectContent(
        \\ {
        \\   foo?: number|null
        \\   bar: number|null
        \\   baz?: number
        \\ }
    ,
        try parse_result.render(arena.allocator()),
    );
}

test "Nested Optionals" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    const Foo = struct {
        enabled: Optional(bool) = .not_provided,
        email: Optional(struct {
            enabled: Optional(bool) = .not_provided,
            threshold: Optional(f64) = .not_provided,
        }) = .not_provided,
    };

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.buildTypeForUsage(Foo, .body);
    try expectContent(try parse_result.render(arena.allocator()),
        \\ {
        \\   enabled?: boolean
        \\   email?: {
        \\     enabled?: boolean
        \\     threshold?: number
        \\   }
        \\ }
    );
}

test "Optionals require default values" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    const Foo = struct {
        opt: Optional(bool),
    };

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    // This should throw an error
    _ = type_generator.buildTypeForUsage(Foo, .body) catch |err| {
        try std.testing.expectEqual(err, error.OptionalMissingDefault);
        return;
    };

    // If this is reached, we did not throw an error when expected.
    try std.testing.expect(false);
}

test "JsonArray(T)" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    const Foo = struct {
        list: JsonArray(struct { abc: i32 }),
    };

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.buildTypeForUsage(Foo, .body);
    try expectContent(try parse_result.render(arena.allocator()),
        \\ {
        \\   list: {
        \\     abc: number
        \\   }[]
        \\ }
    );
}

// Query parameter edge case tests

// Test containers are declared at container scope so their `@typeName` stays clean (e.g. `generator.Filter`).
// A type declared inside a test body embeds the full test description in its name,
// which breaks `shortTypeName` when the description contains `(`.
const Filter = union(enum) {
    basic: struct { start_date: []const u8 },
    detailed: struct { start_date: []const u8, end_date: []const u8 },
};

test "buildQueryParamsType: union with subset variants" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const built_type = try type_generator.buildQueryParamsType(Filter);
    try expectContent(
        \\ XOR<({
        \\   start_date: string
        \\ }), ({
        \\   start_date: string
        \\   end_date: string
        \\ })>
    ,
        try built_type.render(arena.allocator()),
    );
}

const LostProductionFilter = union(enum) {
    id: i32,
    dsc_row: i32,
    date_range: struct {
        start_date: []const u8,
        end_date: ?[]const u8,
        line: ?i32 = null,
        shift: ?i32 = null,
    },
    all: struct {
        line: ?i32 = null,
        shift: ?i32 = null,
    },
};

test "buildQueryParamsType: LostProductionFilter (scalar + struct variants + shared optionals)" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.buildQueryParamsType(LostProductionFilter);
    try expectContent(
        \\ XOR<({ id: number }), XOR<({ dsc_row: number }), XOR<({
        \\   start_date: string
        \\   end_date: string|null
        \\   line?: number|null
        \\   shift?: number|null
        \\ }), ({
        \\   line?: number|null
        \\   shift?: number|null
        \\ })>>>
    ,
        try parse_result.render(arena.allocator()),
    );
}

const Query = struct {
    page: i32,
    filter: union(enum) {
        by_id: i32,
        by_date: struct { start_date: []const u8 },
    },
};

test "buildQueryParamsType: base keys + union variant keys" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.buildQueryParamsType(Query);
    try expectContent(
        \\ (XOR<({ by_id: number }), ({
        \\   start_date: string
        \\ })>) & {
        \\  page: number
        \\ }
    ,
        try parse_result.render(arena.allocator()),
    );
}

const WorkerFilter = union(enum) {
    id: i32,
    unassigned,

    // Query param parsing does not need this, but confirm it is ignored when present.
    pub const _repr: UnionRepr = .untagged;
};

const WorkerQuery = struct {
    worker: Optional(WorkerFilter) = .not_provided,
};

test "buildQueryParamsType: Optional union leaf renders as an untagged TS union" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.buildQueryParamsType(WorkerQuery);
    try expectContent(
        \\ {
        \\   worker?: number | "unassigned"
        \\ }
    ,
        try parse_result.render(arena.allocator()),
    );
}

const WorkerFilterNoRepr = union(enum) {
    id: i32,
    unassigned,
};

test "buildQueryParamsType: query param union needs no _repr declaration" {
    const WorkerQueryNoRepr = struct {
        worker: Optional(WorkerFilterNoRepr) = .not_provided,
    };

    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.buildQueryParamsType(WorkerQueryNoRepr);
    try expectContent(
        \\ {
        \\   worker?: number | "unassigned"
        \\ }
    ,
        try parse_result.render(arena.allocator()),
    );
}

test "buildQueryParamsType: Optional(?Union) leaf is optional and nullable" {
    const WorkerQueryNestedOptional = struct {
        worker: Optional(?WorkerFilterNoRepr) = .not_provided,
    };

    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.buildQueryParamsType(WorkerQueryNestedOptional);
    try expectContent(
        \\ {
        \\   worker?: number | "unassigned" | null
        \\ }
    ,
        try parse_result.render(arena.allocator()),
    );
}

test "buildQueryParamsType: ?Union leaf is optional and nullable" {
    const WorkerQueryNativeOptional = struct {
        worker: ?WorkerFilterNoRepr = null,
    };

    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.buildQueryParamsType(WorkerQueryNativeOptional);
    try expectContent(
        \\ {
        \\   worker?: number | "unassigned" | null
        \\ }
    ,
        try parse_result.render(arena.allocator()),
    );
}

const WorkerQueryRequiredNative = struct {
    worker: ?WorkerFilterNoRepr,
};

test "buildQueryParamsType: required native ?Union leaf is required and nullable" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.buildQueryParamsType(WorkerQueryRequiredNative);
    try expectContent(
        \\ {
        \\   worker: number | "unassigned" | null
        \\ }
    ,
        try parse_result.render(arena.allocator()),
    );
    try expectEqual(false, parse_result.optional);
}

// Mimics `Str(Date)`: a query-param wrapper whose runtime value is always a string,
// so typegen must coerce it to `string` instead of expanding its internal `{ str, data }` representation.
const StrDate = struct {
    str: []const u8,
    data: struct { year: i32, month: i32, day: i32 },
    pub fn paramParse() void {}
};

// A plain query-param struct with no union field, containing a paramParse leaf.
const PlainDateQuery = struct {
    start_date: StrDate,
    line: i32,
};

test "buildQueryParamsType: plain struct coerces a paramParse leaf to string" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.buildQueryParamsType(PlainDateQuery);
    try expectContent(
        \\{
        \\  start_date: string
        \\  line: number
        \\}
    , try parse_result.render(arena.allocator()));
}

const AllOptionalQuery = struct {
    line: ?i32 = null,
    shift: ?i32 = null,
};

test "buildQueryParamsType: optionality follows the minimum required key count" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    // A struct with a required field is required
    try expectEqual(false, (try type_generator.buildQueryParamsType(PlainDateQuery)).optional);

    // A struct with only optional fields is optional
    try expectEqual(true, (try type_generator.buildQueryParamsType(AllOptionalQuery)).optional);

    // Every variant has required keys, so at least one key is always needed
    try expectEqual(false, (try type_generator.buildQueryParamsType(Filter)).optional);

    // The `all` variant needs zero keys (all-optional fallback), so it can be omitted
    try expectEqual(true, (try type_generator.buildQueryParamsType(LostProductionFilter)).optional);

    // Base key `page` is required
    try expectEqual(false, (try type_generator.buildQueryParamsType(Query)).optional);
}

// An optional nested struct with a single required leaf and an optional sibling.
// Because a key is present at runtime only when `cursor` is supplied,
// it renders as a gated group `XOR<{ cursor; before? }, {}>` rather than flattening every key optional.
const CursorQuery = struct {
    room: i32,
    cursor: types.Optional(struct {
        cursor: i64,
        before: bool = false,
    }) = .not_provided,
    limit: u32 = 10,
};

test "buildQueryParamsType: optional nested struct with a required key renders as a gated XOR group" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.buildQueryParamsType(CursorQuery);
    try expectContent(
        \\XOR<{
        \\  cursor: number
        \\  before?: boolean
        \\}, {}> & {
        \\  room: number
        \\  limit?: number
        \\}
    , try parse_result.render(arena.allocator()));
}

const RangeQuery = struct {
    page: u32 = 1,
    range: Optional(struct {
        start: []const u8,
        end: []const u8,
        note: bool = false,
    }) = .not_provided,
};

test "buildQueryParamsType: handles gated group optionality" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.buildQueryParamsType(RangeQuery);
    try expectContent(
        \\XOR<{
        \\  start: string
        \\  end: string
        \\  note?: boolean
        \\}, {}> & {
        \\  page?: number
        \\}
    , try parse_result.render(arena.allocator()));
}

const RangeQueryOneRequired = struct {
    page: u32 = 1,
    range: Optional(struct {
        start: []const u8,
        end: []const u8 = "",
        note: bool = false,
    }) = .not_provided,
};

test "buildQueryParamsType: gated group with a single required key" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.buildQueryParamsType(RangeQueryOneRequired);
    try expectContent(
        \\XOR<{
        \\  start: string
        \\  end?: string
        \\  note?: boolean
        \\}, {}> & {
        \\  page?: number
        \\}
    , try parse_result.render(arena.allocator()));
}

const AnyOfQuery = struct {
    a: Optional(i32) = .not_provided,
    b: Optional([]const u8) = .not_provided,

    pub const constraints: types.Constraints = .{ .any_of = true };
};

test "buildQueryParamsType: any_of constraint wraps the object in AnyOf" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.buildQueryParamsType(AnyOfQuery);
    try expectContent(
        \\AnyOf<{
        \\  a?: number
        \\  b?: string
        \\}>
    , try parse_result.render(arena.allocator()));
    try expectEqual(false, parse_result.optional);
}

const LostProdEndpoint = struct {
    pub const LostProductionQueryParams = union(enum) {
        id: i32,
        dsc_row_id: i32,
        date_range: struct {
            start_date: []const u8,
            end_date: types.Optional([]const u8) = .not_provided,
        },
    };

    const Ctx = struct { query_params: LostProductionQueryParams };
    pub const Body = struct { ok: bool };
    const Res = struct { body: ?Body = null };

    pub fn get(_: *Ctx) Res {
        return .{};
    }
};

test "generateTypes: public union query params are exported and referenced by name" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const endpoints = [_]EndpointDef{.{ "/company/lost-production", LostProdEndpoint }};
    const output = try type_generator.generateTypes(&endpoints);

    try std.testing.expect(
        std.mem.indexOf(u8, output, "export type LostProductionQueryParams =") != null,
    );

    // Referenced by name at the endpoint rather than inlined as the raw type
    try std.testing.expect(
        std.mem.indexOf(u8, output, "queryParams: LostProductionQueryParams") != null,
    );
}

// A public union whose non-scalar variant holds a `paramParse` date leaf (like `Str(PlainDate)`),
const DscQueryEndpoint = struct {
    pub const DscQuery = union(enum) {
        id: i32,
        filter: struct {
            date: StrDate,
            shift: i32,
            line: i32,
        },
    };
    const Ctx = struct { query_params: DscQuery };
    const Res = struct { body: ?bool = null };
    pub fn get(_: *Ctx) Res {
        return .{};
    }
};

const DscQueryReportEndpoint = struct {
    const Ctx = struct { query_params: DscQueryEndpoint.DscQuery };
    const Res = struct { body: ?bool = null };
    pub fn get(_: *Ctx) Res {
        return .{};
    }
};

test "generateTypes: union query param with a paramParse leaf, shared by two endpoints" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const endpoints = [_]EndpointDef{
        .{ "/company/dsc", DscQueryEndpoint },
        .{ "/company/dsc/report", DscQueryReportEndpoint },
    };
    const output = try type_generator.generateTypes(&endpoints);

    try expectContent(
        \\ export type DscQuery =
        \\   XOR<({ id: number }), ({
        \\     date: string
        \\     shift: number
        \\     line: number
        \\   })>
        \\
        \\ export type Spec = {
        \\   GET: {
        \\     "/company/dsc": {
        \\       queryParams: DscQuery
        \\       response: boolean,
        \\     }
        \\     "/company/dsc/report": {
        \\       queryParams: DscQuery
        \\       response: boolean,
        \\     }
        \\   },
        \\   POST: {},
        \\   PUT: {},
        \\   PATCH: {},
        \\   DELETE: {},
        \\ };
    , output);
}

test "generateTypes: public struct query param coerces paramParse leaves into string" {
    const DateFilterEndpoint = struct {
        pub const DateFilter = struct {
            start_date: StrDate,
            end_date: StrDate,
            worker: ?i32 = null,
            hour: ?i32 = null,
        };
        const Ctx = struct { query_params: DateFilter };
        const Res = struct { body: ?bool = null };
        pub fn get(_: *Ctx) Res {
            return .{};
        }
    };

    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const endpoints = [_]EndpointDef{.{ "/date-filter", DateFilterEndpoint }};
    const output = try type_generator.generateTypes(&endpoints);

    try expectContent(
        \\ export type DateFilter =
        \\   {
        \\     start_date: string
        \\     end_date: string
        \\     worker?: number|null
        \\     hour?: number|null
        \\   }
        \\
        \\ export type Spec = {
        \\   GET: {
        \\     "/date-filter": {
        \\       queryParams: DateFilter
        \\       response: boolean,
        \\     }
        \\   },
        \\   POST: {},
        \\   PUT: {},
        \\   PATCH: {},
        \\   DELETE: {},
        \\ };
    , output);
}

test "generateTypes: public enum may be shared by query params and a response" {
    const Endpoint = struct {
        pub const Metric = enum { capacity, units };
        const Ctx = struct { query_params: Metric };
        const Res = struct { body: ?Metric = null };

        pub fn get(_: *Ctx) Res {
            return .{};
        }
    };

    var arena = ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const endpoints = [_]EndpointDef{.{ "/metric", Endpoint }};
    const output = try type_generator.generateTypes(&endpoints);

    try std.testing.expect(std.mem.indexOf(u8, output, "export type Metric =") != null);
    try std.testing.expect(std.mem.indexOf(u8, output, "queryParams: Metric") != null);
    try std.testing.expect(std.mem.indexOf(u8, output, "response: Metric") != null);
}

const AlertTopic = enum { downtime, lost_production };

// A public adjacently-tagged union used in an endpoint response.
const AlertPayload = union(AlertTopic) {
    downtime: struct { line: []const u8, minutes: f32 },
    lost_production: struct { line: []const u8, units: f64 },

    pub const _repr: UnionRepr = .{ .adjacently = .{ .discriminator = "topic" } };
};

const Alert = struct {
    id: i64,
    topic: AlertTopic,
    payload: AlertPayload,
};

const AlertsEndpoint = struct {
    const Ctx = struct {};
    const Res = struct { body: ?[]Alert = null };
    pub fn get(_: *Ctx) Res {
        return .{};
    }
};

test "generateTypes: tagged union used in a response is exported by name" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const endpoints = [_]EndpointDef{.{ "/alerts", AlertsEndpoint }};

    // The tagged union is exported as a named top-level type
    const output = try type_generator.generateTypes(&endpoints);
    try expectContent(
        \\ export type Alert =
        \\   {
        \\     [K in keyof AlertPayload]: {
        \\       id: number
        \\       topic: K
        \\       payload: AlertPayload[K]
        \\     };
        \\   }[keyof AlertPayload];
        \\
        \\
        \\ export type AlertPayload =
        \\   {
        \\     downtime: {
        \\       line: string
        \\       minutes: number
        \\     }
        \\     lost_production: {
        \\       line: string
        \\       units: number
        \\     }
        \\   }
        \\
        \\ export type Spec = {
        \\   GET: {
        \\     "/alerts": {
        \\       response: Alert[],
        \\     }
        \\   },
        \\   POST: {},
        \\   PUT: {},
        \\   PATCH: {},
        \\   DELETE: {},
        \\ };
    , output);
}

// A second endpoint whose response uses the same `Alert` type.
const AlertsImportEndpoint = struct {
    const Ctx = struct {};
    const Res = struct { body: ?[]Alert = null };
    pub fn get(_: *Ctx) Res {
        return .{};
    }
};

test "generateTypes: same tagged union reached from two endpoints is exported once" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const endpoints = [_]EndpointDef{
        .{ "/alerts", AlertsEndpoint },
        .{ "/company/alerts", AlertsImportEndpoint },
    };
    const output = try type_generator.generateTypes(&endpoints);

    // Exported exactly once despite being referenced from both endpoints
    try std.testing.expectEqual(
        1,
        std.mem.count(u8, output, "export type AlertPayload ="),
    );
}

const NotifTopic = enum { post_created, comment_created };

// An adjacently-tagged union that is a public decl of its endpoint
// must export as an object of variants form instead of the flat XOR form.
const NotifEndpoint = struct {
    pub const NotifPayload = union(NotifTopic) {
        post_created: struct { id: i32, title: []const u8 },
        comment_created: struct { id: i32, body: []const u8 },

        pub const _repr: UnionRepr = .{ .adjacently = .{ .discriminator = "topic" } };
    };
    const Notif = struct {
        id: i32,
        topic: NotifTopic,
        payload: NotifPayload,
    };
    const Ctx = struct {};
    const Res = struct { body: ?[]Notif = null };
    pub fn get(_: *Ctx) Res {
        return .{};
    }
};

test "generateTypes: a public tagged-union decl exports as object-of-variants, not XOR" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const endpoints = [_]EndpointDef{.{ "/notif", NotifEndpoint }};
    const output = try type_generator.generateTypes(&endpoints);
    try expectContent(
        \\ export type Notif =
        \\   {
        \\     [K in keyof NotifPayload]: {
        \\       id: number
        \\       topic: K
        \\       payload: NotifPayload[K]
        \\     };
        \\   }[keyof NotifPayload];
        \\
        \\
        \\ export type NotifPayload =
        \\   {
        \\     post_created: {
        \\       id: number
        \\       title: string
        \\     }
        \\     comment_created: {
        \\       id: number
        \\       body: string
        \\     }
        \\   }
        \\
        \\ export type Spec = {
        \\   GET: {
        \\     "/notif": {
        \\       response: Notif[],
        \\     }
        \\   },
        \\   POST: {},
        \\   PUT: {},
        \\   PATCH: {},
        \\   DELETE: {},
        \\ };
    , output);
}

// A child struct declared as a top-level type in its own endpoint file
const ReportRowEndpoint = struct {
    pub const ReportRow = struct {
        id: i32,
        label: []const u8,
    };
    const Ctx = struct {};
    const Res = struct { body: ?ReportRow = null };
    pub fn get(_: *Ctx) Res {
        return .{};
    }
};

// A parent in a different endpoint that references the same type above
const ReportEndpoint = struct {
    pub const Report = struct {
        id: i32,
        rows: []ReportRowEndpoint.ReportRow,
    };
    const Ctx = struct {};
    const Res = struct { body: ?Report = null };
    pub fn get(_: *Ctx) Res {
        return .{};
    }
};

test "generateTypes: a type declared in another endpoint is referenced by name, not inlined" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    // Order matters:
    // the referencing endpoint (/report) comes first,
    // so `ReportRow` is still an unrendered pass-1 placeholder
    // when `Report`'s `rows` field is generated in pass 2.
    // This is the exact condition that previously caused the child to be inlined instead of referenced.
    const endpoints = [_]EndpointDef{
        .{ "/report", ReportEndpoint },
        .{ "/report/row", ReportRowEndpoint },
    };
    const output = try type_generator.generateTypes(&endpoints);

    // The parent references the child by name...
    try std.testing.expect(
        std.mem.indexOf(u8, output, "rows: ReportRow[]") != null,
    );
    // ...and the child is still exported as its own top-level type.
    try std.testing.expect(
        std.mem.indexOf(u8, output, "export type ReportRow =") != null,
    );
}

// Tests I'd like to cover if we can figure out how to test compilation errors:
//
//   - Identical required keys:
//       union(enum) {
//         a: struct{ x:[]const u8 },
//         b: struct{ x:[]const u8 },
//       }
//
//   - Identical required keys differing only by an optional key
//     (optional keys never participate in selection, so both variants only require `start_date`):
//       union(enum) {
//         basic: struct{ start_date: []const u8 },
//         detailed: struct{ start_date: []const u8, end_date: ?[]const u8 = null },
//       }
//
//   - Colliding base keys with variant keys:
//       struct {
//         start_date: []const u8,
//         f: union(enum) {
//           a: struct {
//             start_date:[]const u8,
//           },
//         }
//       }
//
//   - More than one variant with all optional keys:
//       union(enum) {
//         a: struct {
//           x: ?i32 = null,
//         },
//         b: struct {
//           y: ?i32 = null,
//         },
//       }
