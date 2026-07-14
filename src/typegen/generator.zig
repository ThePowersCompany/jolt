const std = @import("std");
const ArenaAllocator = std.heap.ArenaAllocator;
const Allocator = std.mem.Allocator;
const StringArrayHashMap = std.StringArrayHashMap;
const ArrayList = std.ArrayList;
const StringHashMap = std.StringHashMap;
const Type = std.builtin.Type;
const allocPrint = std.fmt.allocPrint;
const stringToEnum = std.meta.stringToEnum;

const EndpointDef = @import("../main.zig").EndpointDef;
const UnionRepr = types.UnionRepr;

const types = @import("../utils/types.zig");
const isOptional = types.isOptional;
const JsonArray = types.JsonArray;

const common = @import("./common.zig");
const strEqls = common.strEqls;
const startsWith = common.startsWith;
const Method = common.Method;
const EndpointData = common.EndpointData;
const ParseResult = common.ParseResult;
const AdjacentUnion = common.AdjacentUnion;
const FlatLeaf = common.FlatLeaf;

const qp_validation = @import("../middleware/query_params/validation.zig");
const hasParamParse = qp_validation.hasParamParse;
const isLiftableUnion = qp_validation.isLiftableUnion;
const getRequiredKeyCount = qp_validation.getRequiredKeyCount;

const expectEqual = std.testing.expectEqual;
const expectContent = @import("../utils/testing.zig").expectContent;

pub const TypeGenerator = struct {
    const Self = @This();

    arena_alloc: Allocator,

    /// short type name => ParseResult
    top_level_types: StringHashMap(?ParseResult),

    get_endpoints: StringArrayHashMap(EndpointData),
    post_endpoints: StringArrayHashMap(EndpointData),
    put_endpoints: StringArrayHashMap(EndpointData),
    patch_endpoints: StringArrayHashMap(EndpointData),
    delete_endpoints: StringArrayHashMap(EndpointData),

    pub fn init(arena_alloc: Allocator) !Self {
        return .{
            .arena_alloc = arena_alloc,
            .top_level_types = StringHashMap(?ParseResult).init(arena_alloc),
            .get_endpoints = StringArrayHashMap(EndpointData).init(arena_alloc),
            .post_endpoints = StringArrayHashMap(EndpointData).init(arena_alloc),
            .put_endpoints = StringArrayHashMap(EndpointData).init(arena_alloc),
            .patch_endpoints = StringArrayHashMap(EndpointData).init(arena_alloc),
            .delete_endpoints = StringArrayHashMap(EndpointData).init(arena_alloc),
        };
    }

    pub fn deinit(self: *Self) void {
        self.top_level_types.deinit();
        self.get_endpoints.deinit();
        self.post_endpoints.deinit();
        self.put_endpoints.deinit();
        self.patch_endpoints.deinit();
        self.delete_endpoints.deinit();
    }

    fn endpointsData(self: *Self, method: Method) *StringArrayHashMap(EndpointData) {
        return switch (method) {
            .get => &self.get_endpoints,
            .post => &self.post_endpoints,
            .put => &self.put_endpoints,
            .patch => &self.patch_endpoints,
            .delete => &self.delete_endpoints,
        };
    }

    fn getTopLevelType(self: *Self, name: []const u8) ?ParseResult {
        const entry = self.top_level_types.get(name) orelse return null;
        return entry orelse ParseResult{ .parsed = name, .optional = false };
    }

    /// "Registers" a top level type name before the full type has been generated.
    fn declareTopLevelType(self: *Self, comptime name: []const u8) !void {
        if (self.top_level_types.contains(name)) {
            std.log.err("Tried to redeclare top level type: {s}", .{name});
            return error.DuplicateDeclaration;
        }
        try self.top_level_types.put(name, null);
    }

    fn setTopLevelType(self: *Self, name: []const u8, result: ParseResult) !void {
        if (self.top_level_types.get(name)) |entry| if (entry != null) {
            std.log.err("Duplicate type name {s}", .{name});
            return error.DuplicateTypeName;
        };
        try self.top_level_types.put(name, result);
    }

    fn shortTypeName(type_name: []const u8) []const u8 {
        var generic_iter = std.mem.splitScalar(u8, type_name, '(');
        const pre_generic = generic_iter.first();

        var iter = std.mem.splitBackwardsScalar(u8, pre_generic, '.');
        return iter.first();
    }

    pub fn generateTypes(self: *Self, comptime endpoints: []const EndpointDef) ![]const u8 {
        @setEvalBranchQuota(endpoints.len * 2000);

        // First pass: Find all pub top-level types across all endpoint files
        inline for (endpoints) |endpoint| {
            _, const EndpointType = endpoint;
            const type_info = @typeInfo(EndpointType);
            inline for (type_info.@"struct".decls) |decl| {
                const T = @TypeOf(@field(EndpointType, decl.name));
                switch (@typeInfo(T)) {
                    .type => {
                        switch (@typeInfo(@field(EndpointType, decl.name))) {
                            .@"struct" => {
                                try self.declareTopLevelType(decl.name);
                            },
                            .@"enum" => |e| {
                                try self.setTopLevelType(
                                    decl.name,
                                    .{ .parsed = try self.parseEnum(e), .optional = false },
                                );
                            },
                            .@"union" => |u| {
                                const parsed = if (isLiftableUnion(@field(EndpointType, decl.name)))
                                    try self.parseFlatUnion(u)
                                else
                                    try self._parseUnionAsStruct(u);
                                try self.setTopLevelType(decl.name, parsed);
                            },
                            else => {},
                        }
                    },
                    else => {},
                }
            }
        }

        // Second pass: Generate typings for all top-level types across all endpoint files
        inline for (endpoints) |endpoint| {
            const endpoint_path, const EndpointType = endpoint;
            const type_info = @typeInfo(EndpointType);
            inline for (type_info.@"struct".decls) |decl| {
                const decl_info = @typeInfo(@TypeOf(@field(EndpointType, decl.name)));
                switch (decl_info) {
                    .type => {
                        const t_info = @typeInfo(@field(EndpointType, decl.name));
                        switch (t_info) {
                            .@"struct" => |s| {
                                const res = self.parseStruct(decl.name, s) catch |err| {
                                    std.log.info(
                                        "Endpoint: {s} - Type: {s}",
                                        .{ endpoint_path, decl.name },
                                    );

                                    return err;
                                };
                                try self.setTopLevelType(decl.name, res);
                            },
                            else => {},
                        }
                    },
                    else => {},
                }
            }
        }

        // Third pass: Generate endpoint types
        inline for (endpoints) |endpoint| try self.genTypescript(endpoint);

        var res: ArrayList(u8) = .empty;

        {
            // Print top-level types
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

            var iter = self.top_level_types.iterator();
            while (iter.next()) |top| {
                const result = top.value_ptr.* orelse continue;
                try entries.append(self.arena_alloc, .{
                    .type_name = top.key_ptr.*,
                    .ts = try allocPrint(
                        self.arena_alloc,
                        "export type {s} =\n{s}\n\n",
                        .{ top.key_ptr.*, result.parsed },
                    ),
                });
            }

            // Sort alphabetically
            std.mem.sort(Entry, entries.items, {}, Entry.sort);
            for (entries.items) |entry| {
                try res.appendSlice(self.arena_alloc, entry.ts);
            }
        }

        // Print http method endpoint typings

        try res.appendSlice(self.arena_alloc, "export type Spec = {");

        inline for (@typeInfo(Method).@"enum".fields) |m| {
            const method: Method = @enumFromInt(m.value);
            var endpoints_data = self.endpointsData(method);

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

    fn genTypescript(self: *Self, endpoint: EndpointDef) !void {
        const endpoint_path, const EndpointType = endpoint;
        const type_info = @typeInfo(EndpointType);
        inline for (type_info.@"struct".decls) |decl| {
            const decl_info = @typeInfo(@TypeOf(@field(EndpointType, decl.name)));
            switch (decl_info) {
                // Find get/post/put/patch/delete functions
                .@"fn" => self.populateFnTypescript(decl, decl_info.@"fn", endpoint_path) catch |err| {
                    std.log.info(
                        "Endpoint: {s} - Type: {s}",
                        .{ endpoint_path, decl.name },
                    );
                    return err;
                },
                else => {},
            }
        }
    }

    fn populateStructTypescript(
        self: *Self,
        method: Method,
        S: Type.Struct,
        endpoint_path: []const u8,
    ) !void {
        var endpoints_data = self.endpointsData(method);

        var res = try endpoints_data.getOrPutValue(endpoint_path, .{});
        inline for (S.fields) |field| {
            if (comptime strEqls(field.name, "body")) {
                const body_info = @typeInfo(field.type);
                if (body_info == .pointer and body_info.pointer.child == u8) {
                    res.value_ptr.body = "body: BodyInit\n";
                } else {
                    const body_res = try self.extractIdentifier(field.type);
                    if (body_res.optional) {
                        res.value_ptr.body = try allocPrint(
                            self.arena_alloc,
                            "body?: {s}\n",
                            .{body_res.parsed},
                        );
                    } else {
                        res.value_ptr.body = try allocPrint(
                            self.arena_alloc,
                            "body: {s}\n",
                            .{body_res.parsed},
                        );
                    }
                }
            } else if (comptime strEqls(field.name, "query_params")) {
                const param_res = try self.extractQueryParams(field.type);
                if (param_res.optional) {
                    res.value_ptr.query_params = try allocPrint(
                        self.arena_alloc,
                        "queryParams?: {s}\n",
                        .{param_res.parsed},
                    );
                } else {
                    res.value_ptr.query_params = try allocPrint(
                        self.arena_alloc,
                        "queryParams: {s}\n",
                        .{param_res.parsed},
                    );
                }
            }
        }
    }

    fn parseStruct(self: *Self, struct_name: []const u8, S: Type.Struct) !ParseResult {
        // Find adjacent union ahead of time
        var adjacent_union: ?AdjacentUnion = null;
        {
            inline for (S.fields) |field| {
                const info = @typeInfo(field.type);
                if (info != .@"union") continue;

                var union_repr: ?UnionRepr = null;
                inline for (info.@"union".decls) |decl| {
                    if (comptime strEqls(decl.name, "_repr")) {
                        const repr = @field(field.type, decl.name);
                        if (repr == .adjacently) {
                            union_repr = repr;
                            break;
                        }
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
            const short_struct_name = shortTypeName(struct_name);
            try self.setTopLevelType(short_struct_name, try self.parseStructWithAdjacentUnion(S, au));

            return ParseResult{
                .optional = false,
                .parsed = short_struct_name,
            };
        }

        // Default struct parsing if there's no adjacent union present as a field
        return self._parseStruct(S);
    }

    fn _parseStruct(self: *Self, S: Type.Struct) !ParseResult {
        var all_optional = true;
        var res: ArrayList(u8) = .empty;
        try res.appendSlice(self.arena_alloc, "{\n");
        inline for (S.fields) |field| {
            try res.appendSlice(self.arena_alloc, field.name);

            comptime var T: type = field.type;
            if (comptime startsWith(@typeName(T), "utils.types.JsonArray(")) {
                T = @FieldType(@FieldType(T, "list"), "items");
            }

            // Ensure Optionals have default values
            if (comptime isOptional(field.type) and field.defaultValue() == null) {
                std.log.info(
                    "Optional type \"{s}\" must have a provided default value: {s}",
                    .{ field.name, @typeName(field.type) },
                );
                return error.OptionalMissingDefault;
            }

            const parse_result = try self.extractIdentifier(T);

            // TODO: This is a weird bug in defaultValue I had to work around...
            if (comptime strEqls(field.name, "_is_finished")) continue;

            const optional = if (field.defaultValue()) |_| true else parse_result.optional;

            all_optional = all_optional and optional;
            if (optional) {
                try res.appendSlice(self.arena_alloc, "?: ");
            } else {
                try res.appendSlice(self.arena_alloc, ": ");
            }

            try res.appendSlice(self.arena_alloc, parse_result.parsed);
            try res.appendSlice(self.arena_alloc, "\n");
        }

        try res.appendSlice(self.arena_alloc, "}");

        return .{
            .optional = all_optional,
            .parsed = try res.toOwnedSlice(self.arena_alloc),
        };
    }

    /// Helper function for adjacent unions
    fn _parseUnionAsStruct(self: *Self, U: Type.Union) !ParseResult {
        var all_optional = true;
        var res: ArrayList(u8) = .empty;
        try res.appendSlice(self.arena_alloc, "{\n");
        inline for (U.fields) |field| {
            try res.appendSlice(self.arena_alloc, field.name);

            const parse_result = try self.extractIdentifier(field.type);
            all_optional = all_optional and parse_result.optional;
            if (parse_result.optional) {
                try res.appendSlice(self.arena_alloc, "?: ");
            } else {
                try res.appendSlice(self.arena_alloc, ": ");
            }

            try res.appendSlice(self.arena_alloc, parse_result.parsed);
            try res.appendSlice(self.arena_alloc, "\n");
        }

        try res.appendSlice(self.arena_alloc, "}");

        return .{
            .optional = all_optional,
            .parsed = try res.toOwnedSlice(self.arena_alloc),
        };
    }

    fn parseStructWithAdjacentUnion(self: *Self, S: Type.Struct, adjacent_union: AdjacentUnion) !ParseResult {
        const union_short_name = shortTypeName(adjacent_union.name);

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

                if (self.getTopLevelType(union_short_name) == null) {
                    try self.setTopLevelType(
                        union_short_name,
                        try self._parseUnionAsStruct(field_info.@"union"),
                    );
                }
                try res.appendSlice(
                    self.arena_alloc,
                    try allocPrint(self.arena_alloc, "{s}: {s}[K]", .{ f.name, union_short_name }),
                );
            } else {
                try res.appendSlice(self.arena_alloc, f.name);
                const parse_result = try self.extractIdentifier(f.type);
                if (parse_result.optional) {
                    try res.appendSlice(self.arena_alloc, "?: ");
                } else {
                    try res.appendSlice(self.arena_alloc, ": ");
                }
                try res.appendSlice(self.arena_alloc, parse_result.parsed);
                try res.appendSlice(self.arena_alloc, "\n");
            }
        }

        try res.appendSlice(self.arena_alloc, try allocPrint(
            self.arena_alloc,
            "}};\n}}[keyof {s}];\n",
            .{union_short_name},
        ));

        return .{ .parsed = try res.toOwnedSlice(self.arena_alloc), .optional = false };
    }

    fn parseEnum(self: *Self, E: Type.Enum) ![]const u8 {
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

    fn parseUnion(self: *Self, U: Type.Union, T: type) ![]const u8 {
        var res: ArrayList(u8) = .empty;

        // Special case for Optional(T)
        if (comptime isOptional(T)) {
            const parsed_res = try self.extractIdentifier(@FieldType(T, "value"));
            try res.appendSlice(self.arena_alloc, parsed_res.parsed);
            return res.toOwnedSlice(self.arena_alloc);
        }

        var union_repr: ?UnionRepr = null;
        inline for (U.decls) |decl| {
            if (comptime strEqls(decl.name, "_repr")) {
                union_repr = @field(T, decl.name);
                break;
            }
        }

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
                            const parsed_res = try self.extractIdentifier(f.type);
                            try res.appendSlice(self.arena_alloc, try allocPrint(
                                self.arena_alloc,
                                "{s}{s}: {s}; ",
                                .{
                                    f.name,
                                    if (parsed_res.optional or f.defaultValue() != null) "?" else "",
                                    parsed_res.parsed,
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
                        .{shortTypeName(@typeName(T))},
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
                            const parsed_res = try self.extractIdentifier(f.type);
                            try res.appendSlice(self.arena_alloc, try allocPrint(
                                self.arena_alloc,
                                "{s}{s}: {s}; ",
                                .{
                                    f.name,
                                    if (parsed_res.optional or f.defaultValue() != null) "?" else "",
                                    parsed_res.parsed,
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
                            const ident = (try self.extractIdentifier(field.type)).parsed;
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

    fn populateFnTypescript(
        self: *Self,
        comptime decl: Type.Declaration,
        comptime F: Type.Fn,
        comptime endpoint_path: []const u8,
    ) !void {
        const method = (comptime stringToEnum(Method, decl.name)) orelse return;
        var endpoints_data = self.endpointsData(method);

        // Find context object (first parameter)
        const first_param = F.params[0].type orelse unreachable;
        const ctx_struct = @typeInfo(@typeInfo(first_param).pointer.child).@"struct";
        try self.populateStructTypescript(method, ctx_struct, endpoint_path);

        // Determine return type
        const return_type_info = @typeInfo(F.return_type.?);
        comptime var ResponseType: type = blk: {
            switch (return_type_info) {
                .error_union => {
                    const inner_info = @typeInfo(return_type_info.error_union.payload);
                    break :blk getResponseInnerType(inner_info.@"struct");
                },
                .@"struct" => break :blk getResponseInnerType(return_type_info.@"struct"),
                else => @compileError("Invalid fn return type"),
            }
        };

        var res = try endpoints_data.getOrPutValue(endpoint_path, .{});
        const ts = (try self.extractIdentifier(ResponseType)).parsed;
        res.value_ptr.response = ts;

        const type_info = @typeInfo(ResponseType);
        comptime var s: ?Type.Struct = null;
        if (type_info == .pointer) {
            const child_info = @typeInfo(type_info.pointer.child);
            if (child_info == .@"struct") {
                s = child_info.@"struct";
                ResponseType = type_info.pointer.child;
            }
        }

        if (s == null and type_info == .@"struct") s = type_info.@"struct";

        if (s) |_| {
            const type_name = shortTypeName(@typeName(ResponseType));
            if (!self.top_level_types.contains(type_name) and !isInlinedStruct(type_name)) {
                std.log.err("{s} must be pub", .{type_name});
                return error.ResponseTypeIsPrivate;
            }
        }
    }

    fn getResponseInnerType(s: Type.Struct) type {
        inline for (s.fields) |field| {
            if (comptime strEqls(field.name, "body")) {
                return @typeInfo(field.type).optional.child;
            }
        }
        @compileError("Return type of endpoint fns must be Response(T)");
    }

    /// Combines an already-emitted struct object (`res`)
    /// with the TS utility types implied by its `constraints` decl.
    /// The `any_of` rule additionally forces the field to be required.
    fn applyConstraints(self: *Self, comptime constraints: types.Constraints, res: ParseResult) !ParseResult {
        if (comptime constraints.any_of) {
            return .{
                .parsed = try allocPrint(self.arena_alloc, "AnyOf<{s}>", .{res.parsed}),
                .optional = false,
            };
        }
        return res;
    }

    /// Emits the TS type for a single query param leaf.
    /// A `paramParse` custom type always receives a string at runtime,
    /// so its typegen'd type should be `string`.
    /// `extractIdentifier` parses its struct fields, which is incorrect for query_params.
    /// Everything else delegates to `extractIdentifier`.
    fn queryLeafType(self: *Self, comptime T: type) !ParseResult {
        if (comptime hasParamParse(T)) return .{ .parsed = "string" };
        return self.extractIdentifier(T);
    }

    /// Query params are a flat key/value store, so a tagged union maps to a flattened TS union.
    /// The active variant is inferred at runtime from which keys are present.
    /// Anything else falls back to the normal struct handling.
    fn extractQueryParams(self: *Self, comptime T: type) !ParseResult {
        const type_name = comptime shortTypeName(@typeName(T));
        // The type was already registered at the top level
        if (self.getTopLevelType(type_name) != null) {
            return .{ .parsed = type_name, .optional = comptime queryParamsOptional(T) };
        }

        const info = @typeInfo(T);
        if (comptime info == .@"union" and !isOptional(T)) {
            const res = try self.parseFlatUnion(info.@"union");
            return .{ .parsed = res.parsed, .optional = comptime queryParamsOptional(T) };
        }
        if (comptime info == .@"struct") {
            return self.parseFlatQueryStruct(T, info.@"struct");
        }
        return self.extractIdentifier(T);
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
    fn parseFlatUnion(self: *Self, U: Type.Union) !ParseResult {
        var variants: ArrayList([]const u8) = .empty;
        inline for (U.fields) |field| {
            const info = @typeInfo(field.type);
            if (field.type == void) {
                try variants.append(self.arena_alloc, try allocPrint(
                    self.arena_alloc,
                    "{{ {s}: \"\" }}",
                    .{field.name},
                ));
            } else if (info == .@"struct" and !hasParamParse(field.type)) {
                const variant = (try self.buildFlatStruct(field.type, info.@"struct")) orelse "{}";
                try variants.append(self.arena_alloc, variant);
            } else {
                // Scalar / array / enum / paramParse struct: variant name is key.
                const ident = try self.queryLeafType(field.type);
                try variants.append(self.arena_alloc, try allocPrint(
                    self.arena_alloc,
                    "{{ {s}: {s} }}",
                    .{ field.name, ident.parsed },
                ));
            }
        }
        // A union always needs at least one matching key, so it is required.
        return .{ .parsed = try self.renderExclusiveUnion(variants.items), .optional = false };
    }

    /// Combines variants so exactly one may be present.
    fn renderExclusiveUnion(self: *Self, variants: []const []const u8) ![]const u8 {
        if (variants.len == 0) return "{}";

        // Emit the chain of XORs from left to right.
        // Each leading variant opens an `XOR<(variant), `,
        // the last variant sits in the middle,
        // then the openers are closed with a run of `>`.
        var res: ArrayList(u8) = .empty;
        for (variants, 0..) |variant, i| {
            const last = i == variants.len - 1;
            try res.appendSlice(self.arena_alloc, if (last)
                try allocPrint(self.arena_alloc, "({s})", .{variant})
            else
                try allocPrint(self.arena_alloc, "XOR<({s}), ", .{variant}));
        }
        try res.appendNTimes(self.arena_alloc, '>', variants.len - 1);
        return res.toOwnedSlice(self.arena_alloc);
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
    fn buildFlatStruct(self: *Self, comptime T: type, S: Type.Struct) !?[]const u8 {
        var flat_struct: FlatStruct = .{};
        try self.collectFlatLeaves(&flat_struct, S, false, null);

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
                return try allocPrint(self.arena_alloc, "AnyOf<{s}>", .{full});
            }
            return full;
        }

        // One AllOf per group, the independent keys as a plain object,
        // and AnyOf over everything if the constraint is set.
        var components: ArrayList([]const u8) = .empty;

        // Each group object contains only its own keys.
        for (flat_struct.groups.items) |group| {
            try components.append(self.arena_alloc, try allocPrint(
                self.arena_alloc,
                "AllOf<{s}>",
                .{try self.renderLeafObject(group.items)},
            ));
        }

        if (flat_struct.independent.items.len > 0) {
            try components.append(self.arena_alloc, try self.renderLeafObject(flat_struct.independent.items));
        }

        if (any_of) {
            try components.append(self.arena_alloc, try allocPrint(
                self.arena_alloc,
                "AnyOf<{s}>",
                .{try self.renderLeafObject(all.items)},
            ));
        }

        var res: ArrayList(u8) = .empty;
        for (components.items, 0..) |component, i| {
            if (i != 0) try res.appendSlice(self.arena_alloc, " & ");
            try res.appendSlice(self.arena_alloc, component);
        }
        return try res.toOwnedSlice(self.arena_alloc);
    }

    /// Emits the flat TS shape for a query param struct which contains lifted union fields.
    /// Each union field's variant alternatives live directly in the flat key space,
    /// so the result is the intersection of every union's flat shape,
    /// with the base object built from the remaining fields.
    fn parseFlatQueryStruct(self: *Self, comptime T: type, S: Type.Struct) !ParseResult {
        var components: ArrayList([]const u8) = .empty;

        inline for (S.fields) |field| {
            if (comptime isLiftableUnion(field.type)) {
                const shape = try self.parseFlatUnion(@typeInfo(field.type).@"union");
                try components.append(
                    self.arena_alloc,
                    try allocPrint(self.arena_alloc, "({s})", .{shape.parsed}),
                );
            }
        }

        // Base object from the non-union fields (collectFlatLeaves skips unions).
        if (try self.buildFlatStruct(T, S)) |base| {
            try components.append(self.arena_alloc, base);
        }

        var res: ArrayList(u8) = .empty;
        for (components.items, 0..) |component, i| {
            if (i != 0) try res.appendSlice(self.arena_alloc, " & ");
            try res.appendSlice(self.arena_alloc, component);
        }

        return .{
            .parsed = try res.toOwnedSlice(self.arena_alloc),
            .optional = comptime queryParamsOptional(T),
        };
    }

    /// Recursively flattens a struct into leaf keys, hoisting nested plain structs.
    /// `parent_optional` - Propagates optionality through Optional (or defaulted) wrappers.
    /// `current_group` - The open group that required leaves will join.
    ///   It is opened by an Optional nested struct.
    ///   Optional leaves never join a group, since they never have to exist with other leaves.
    fn collectFlatLeaves(
        self: *Self,
        flat_struct: *FlatStruct,
        S: Type.Struct,
        parent_optional: bool,
        current_group: ?*ArrayList(FlatLeaf),
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
                if (introduces_optional and current_group == null) {
                    // An optional nested struct opens a new group
                    var group: ArrayList(FlatLeaf) = .empty;
                    try self.collectFlatLeaves(flat_struct, info.@"struct", wrapper_optional, &group);
                    // AllOf needs >= 2 keys, a single key is just independent.
                    if (group.items.len >= 2) {
                        try flat_struct.groups.append(self.arena_alloc, group);
                    } else {
                        try flat_struct.independent.appendSlice(self.arena_alloc, group.items);
                    }
                } else {
                    // Required nested struct, or one already inside a group
                    try self.collectFlatLeaves(flat_struct, info.@"struct", wrapper_optional, current_group);
                }
            } else {
                const ident = try self.queryLeafType(T);
                const leaf: FlatLeaf = .{
                    .name = field.name,
                    .ts_type = ident.parsed,
                    .optional = wrapper_optional or ident.optional,
                };
                // A required leaf inside an open group is required together.
                // Everything else (optional, or no open group) is independent.
                const list = if (current_group != null and !introduces_optional)
                    current_group.?
                else
                    &flat_struct.independent;
                try list.append(self.arena_alloc, leaf);
            }
        }
    }

    /// Renders a TS object literal from the given leaves.
    fn renderLeafObject(self: *Self, leaves: []const FlatLeaf) ![]const u8 {
        var res: ArrayList(u8) = .empty;
        try res.appendSlice(self.arena_alloc, "{\n");
        for (leaves) |leaf| {
            try res.appendSlice(self.arena_alloc, leaf.name);
            try res.appendSlice(self.arena_alloc, if (leaf.optional) "?: " else ": ");
            try res.appendSlice(self.arena_alloc, leaf.ts_type);
            try res.appendSlice(self.arena_alloc, "\n");
        }
        try res.appendSlice(self.arena_alloc, "}");
        return res.toOwnedSlice(self.arena_alloc);
    }

    fn extractIdentifier(self: *Self, T: type) !ParseResult {
        const type_info = @typeInfo(T);
        switch (type_info) {
            .int, .float => return .{ .parsed = "number" },
            .bool => return .{ .parsed = "boolean" },
            .type, .void => return .{ .parsed = @typeName(T) },
            .pointer => {
                if (type_info.pointer.child == u8) {
                    return .{ .parsed = "string" };
                } else {
                    return .{ .parsed = try allocPrint(self.arena_alloc, "{s}[]", .{
                        (try self.extractIdentifier(type_info.pointer.child)).parsed,
                    }) };
                }
            },
            .@"struct" => {
                const type_name = comptime shortTypeName(@typeName(T));

                const res: ParseResult = if (self.getTopLevelType(type_name)) |gen|
                    .{ .parsed = type_name, .optional = gen.optional }
                else
                    try self.parseStruct(type_name, type_info.@"struct");

                // Wrap the emitted object in the matching TS utility type(s)
                // if any constraints should be applied.
                if (@hasDecl(T, "constraints")) {
                    return try self.applyConstraints(T.constraints, res);
                }
                return res;
            },
            .@"enum" => {
                const type_name = shortTypeName(@typeName(T));
                if (self.getTopLevelType(type_name)) |gen| {
                    return .{ .parsed = type_name, .optional = gen.optional };
                }
                return .{ .parsed = try self.parseEnum(type_info.@"enum") };
            },
            .@"union" => {
                return .{ .parsed = try self.parseUnion(type_info.@"union", T) };
            },
            .optional => {
                return .{
                    // "optional" in zig means nullable, not actually optional.
                    // This means the value could still be required, but could be set to null.
                    .optional = false,
                    .parsed = try allocPrint(self.arena_alloc, "{s}|null", .{
                        (try self.extractIdentifier(type_info.optional.child)).parsed,
                    }),
                };
            },
            .@"opaque" => {
                return .{ .parsed = @typeName(T) };
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

    const parse_result = try type_generator.extractIdentifier(Foo);
    try expectContent(
        \\ {
        \\   foo?: number|null
        \\   bar: number|null
        \\   baz?: number
        \\ }
    ,
        parse_result.parsed,
    );
}

test "Nested Optionals" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    const Optional = @import("../utils/types.zig").Optional;

    const Foo = struct {
        enabled: Optional(bool) = .not_provided,
        email: Optional(struct {
            enabled: Optional(bool) = .not_provided,
            threshold: Optional(f64) = .not_provided,
        }) = .not_provided,
    };

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.extractIdentifier(Foo);
    try expectContent(parse_result.parsed,
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

    const Optional = @import("../utils/types.zig").Optional;

    const Foo = struct {
        opt: Optional(bool),
    };

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    // This should throw an error
    _ = type_generator.extractIdentifier(Foo) catch |err| {
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

    const parse_result = try type_generator.extractIdentifier(Foo);
    try expectContent(parse_result.parsed,
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

test "extractQueryParams: union with subset variants" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.extractQueryParams(Filter);
    try expectContent(
        \\ XOR<({
        \\   start_date: string
        \\ }), ({
        \\   start_date: string
        \\   end_date: string
        \\ })>
    ,
        parse_result.parsed,
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

test "extractQueryParams: LostProductionFilter (scalar + struct variants + shared optionals)" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.extractQueryParams(LostProductionFilter);
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
        parse_result.parsed,
    );
}

const Query = struct {
    page: i32,
    filter: union(enum) {
        by_id: i32,
        by_date: struct { start_date: []const u8 },
    },
};

test "extractQueryParams: base keys + union variant keys" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.extractQueryParams(Query);
    try expectContent(
        \\ (XOR<({ by_id: number }), ({
        \\   start_date: string
        \\ })>) & {
        \\  page: number
        \\ }
    ,
        parse_result.parsed,
    );
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

test "extractQueryParams: plain struct coerces a paramParse leaf to string" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    const parse_result = try type_generator.extractQueryParams(PlainDateQuery);
    try expectContent(
        \\{
        \\  start_date: string
        \\  line: number
        \\}
    , parse_result.parsed);
}

const AllOptionalQuery = struct {
    line: ?i32 = null,
    shift: ?i32 = null,
};

test "extractQueryParams: optionality follows the minimum required key count" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    var type_generator = try TypeGenerator.init(arena.allocator());
    defer type_generator.deinit();

    // A struct with a required field is required
    try expectEqual(false, (try type_generator.extractQueryParams(PlainDateQuery)).optional);

    // A struct with only optional fields is optional
    try expectEqual(true, (try type_generator.extractQueryParams(AllOptionalQuery)).optional);

    // Every variant has required keys, so at least one key is always needed
    try expectEqual(false, (try type_generator.extractQueryParams(Filter)).optional);

    // The `all` variant needs zero keys (all-optional fallback), so it can be omitted
    try expectEqual(true, (try type_generator.extractQueryParams(LostProductionFilter)).optional);

    // Base key `page` is required
    try expectEqual(false, (try type_generator.extractQueryParams(Query)).optional);
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

    // Order matters: the referencing endpoint (/report) comes first, so `ReportRow` is still an
    // unrendered pass-1 placeholder when `Report`'s `rows` field is generated in pass 2. This is
    // the exact condition that previously caused the child to be inlined instead of referenced.
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
