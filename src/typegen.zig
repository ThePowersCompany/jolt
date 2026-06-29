const std = @import("std");
const ArenaAllocator = std.heap.ArenaAllocator;
const Allocator = std.mem.Allocator;
const StringArrayHashMap = std.StringArrayHashMap;
const ArrayList = std.ArrayList;
const StringHashMap = std.StringHashMap;
const assert = std.debug.assert;
const allocPrint = std.fmt.allocPrint;
const Type = std.builtin.Type;
const EndpointDef = @import("main.zig").EndpointDef;
const stringToEnum = std.meta.stringToEnum;
const UnionRepr = @import("middleware/parse-body.zig").UnionRepr;
const types = @import("utils/types.zig");
const isOptional = types.isOptional;
const JsonArray = types.JsonArray;

const endpoint_fn_names = [_][]const u8{ "get", "post", "put", "patch", "delete" };

fn strEqls(s1: []const u8, s2: []const u8) bool {
    return std.mem.eql(u8, s1, s2);
}

fn startsWith(haystack: []const u8, needle: []const u8) bool {
    return std.mem.startsWith(u8, haystack, needle);
}

const Method = enum {
    get,
    post,
    put,
    patch,
    delete,
};

const EndpointData = struct {
    query_params: ?[]const u8 = null,
    body: ?[]const u8 = null,
    response: ?[]const u8 = null,
};

const PrivateUtilityTypes = [_][]const u8{
    \\ type SetRequired<T, K extends keyof T> = Omit<T, K> & Required<Pick<T, K>>;
    ,
    \\ type RequireAtLeastOne<T, K extends keyof T = keyof T> = {
    \\   [P in K]: SetRequired<T, P>;
    \\ }[K];
    ,
    \\ type RequireTogether<T, K extends keyof T = keyof T> =
    \\   | (T & Required<Pick<T, K>>)
    \\   | (Omit<T, K> & { [P in K]?: never });
    ,
    \\ type MutuallyExclusive<T, K extends keyof T> =
    \\   | (Omit<T, K> & { [P in K]?: never })
    \\   | {
    \\     [E in K]: Omit<T, K> & { [P in E]?: T[P] } & {
    \\       [P in Exclude<K, E>]?: never;
    \\     };
    \\   }[K];
};

pub fn generateTypesFile(
    alloc: Allocator,
    ts_file_name: []const u8,
    endpoints: []const EndpointDef,
) !void {
    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();
    const arena_alloc = arena.allocator();

    var ts: ArrayList(u8) = .empty;
    defer ts.deinit(alloc);

    try ts.appendSlice(alloc,
        \\ // === DO NOT MODIFY ===
        \\ //
        \\ // Auto-generated type definitions
        \\ //
        \\ // === DO NOT MODIFY ===
        \\
        \\
    );

    // Emit the private TS utility types used by the generated definitions below.
    for (PrivateUtilityTypes) |utility_type| {
        try ts.appendSlice(alloc, utility_type);
        try ts.appendSlice(alloc, "\n\n");
    }

    var type_generator = try TypeGenerator.init(arena_alloc);
    defer type_generator.deinit();

    try ts.appendSlice(alloc, try type_generator.generateTypes(endpoints));

    const file = try std.fs.cwd().createFile(ts_file_name, .{ .read = true });
    defer file.close();
    try file.writeAll(ts.items);

    try formatWithPrettier(arena_alloc, ts_file_name);
}

/// Uses prettier to format the given TS file.
fn formatWithPrettier(alloc: Allocator, file_name: []const u8) !void {
    const result = try std.process.Child.run(.{
        .allocator = alloc,
        .argv = &[_][]const u8{
            "npx",
            "prettier",
            "--write",
            file_name,
        },
    });
    const status_code: u32 = switch (result.term) {
        .Exited => |e| e,
        .Stopped => |s| s,
        .Unknown => |u| u,
        else => 0,
    };
    if (status_code != 0) {
        std.log.err("{s}", .{result.stderr});
        return error.PrettierError;
    }
}

const StructInfo = struct {
    name: []const u8,
    S: Type.Struct,
};

const TypeGenerator = struct {
    const Self = @This();

    arena_alloc: Allocator,

    /// short type name => ParseResult
    top_level_types: StringHashMap(ParseResult),

    get_endpoints: StringArrayHashMap(EndpointData),
    post_endpoints: StringArrayHashMap(EndpointData),
    put_endpoints: StringArrayHashMap(EndpointData),
    patch_endpoints: StringArrayHashMap(EndpointData),
    delete_endpoints: StringArrayHashMap(EndpointData),

    pub fn init(arena_alloc: Allocator) !Self {
        return .{
            .arena_alloc = arena_alloc,
            .top_level_types = StringHashMap(ParseResult).init(arena_alloc),
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
        return self.top_level_types.get(name);
    }

    fn setTopLevelType(self: *Self, name: []const u8, result: ParseResult) !void {
        if (self.top_level_types.get(name)) |r| if (r.parsed.len > 0) {
            std.log.err("Duplicate struct name {s}", .{name});
            return error.Foo;
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
                        const t_info = @typeInfo(@field(EndpointType, decl.name));
                        if (t_info == .@"struct") {
                            try self.setTopLevelType(decl.name, .{ .parsed = "", .optional = false });
                        } else if (t_info == .@"enum") {
                            if (!self.top_level_types.contains(decl.name)) {
                                const parsed = try self.parseEnum(t_info.@"enum");
                                try self.setTopLevelType(decl.name, .{ .parsed = parsed, .optional = false });
                            }
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
                try entries.append(self.arena_alloc, .{
                    .type_name = top.key_ptr.*,
                    .ts = try allocPrint(
                        self.arena_alloc,
                        "export type {s} = \n{s}\n\n",
                        .{ top.key_ptr.*, top.value_ptr.*.parsed },
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
            if (strEqls(field.name, "body")) {
                const body_info = @typeInfo(field.type);
                if (body_info == .pointer and body_info.pointer.child == u8) {
                    res.value_ptr.body = "body: BodyInit\n";
                } else {
                    const body_res = try self.extractIdentifier(field.type);
                    if (body_res.optional) {
                        res.value_ptr.body = try allocPrint(self.arena_alloc, "body?: {s}\n", .{body_res.parsed});
                    } else {
                        res.value_ptr.body = try allocPrint(self.arena_alloc, "body: {s}\n", .{body_res.parsed});
                    }
                }
            } else if (strEqls(field.name, "query_params")) {
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
                        std.log.err("Container cannot have more than one adjacent union discriminator fields.", .{});
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

                try self.setTopLevelType(union_short_name, try self._parseUnionAsStruct(field_info.@"union"));
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

    const ParseResult = struct {
        // If empty, parsing hasn't completed yet.
        parsed: []const u8,
        // Whether all the fields of the parsed type are optional.
        optional: bool = false,
    };

    const AdjacentUnion = struct {
        /// The discriminator of an adjacently tagged union.
        /// Only one field in a struct may be this type of union.
        discriminator: []const u8,
        /// The full type name of the Union.
        name: []const u8,
    };

    /// Combines an already-emitted struct object (`res`)
    /// with the TS utility types implied by its `constraints` decl.
    /// The `require_at_least_one` rule additionally forces the field to be required.
    fn applyConstraints(self: *Self, comptime constraints: types.Constraints, res: ParseResult) !ParseResult {
        if (comptime constraints.require_at_least_one) {
            return .{
                .parsed = try allocPrint(self.arena_alloc, "RequireAtLeastOne<{s}>", .{res.parsed}),
                .optional = false,
            };
        }
        return res;
    }

    /// Whether a struct type carries a custom `paramParse`
    /// and therefore stays a single query key (its value parsed from a string),
    /// rather than being flattened into leaf keys.
    fn hasParamParse(comptime T: type) bool {
        return comptime std.meta.hasFn(T, "paramParse");
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
    fn extractQueryParams(self: *Self, T: type) !ParseResult {
        const info = @typeInfo(T);
        if (comptime info == .@"union" and !isOptional(T)) {
            return try self.parseFlatUnion(info.@"union");
        }

        if (comptime isStructContainingUnionField(T)) {
            return try self.parseFlatQueryStruct(T, info.@"struct");
        }
        return try self.extractIdentifier(T);
    }

    /// Parses a union into a flat TS union.
    /// A scalar (or `paramParse`/void) variant uses the variant name as its single key,
    /// and a plain struct variant is flattened into its leaf keys.
    fn parseFlatUnion(self: *Self, U: Type.Union) !ParseResult {
        var res: ArrayList(u8) = .empty;
        inline for (U.fields, 0..) |field, i| {
            if (i != 0) try res.appendSlice(self.arena_alloc, "\n | ");

            const info = @typeInfo(field.type);
            if (field.type == void) {
                try res.appendSlice(self.arena_alloc, try allocPrint(
                    self.arena_alloc,
                    "{{ {s}: \"\" }}",
                    .{field.name},
                ));
            } else if (info == .@"struct" and !hasParamParse(field.type)) {
                const shape = (try self.buildFlatStruct(field.type, info.@"struct")) orelse "{}";
                try res.appendSlice(self.arena_alloc, shape);
            } else {
                // Scalar / array / enum / paramParse struct: variant name is key.
                const ident = try self.queryLeafType(field.type);
                try res.appendSlice(self.arena_alloc, try allocPrint(
                    self.arena_alloc,
                    "{{ {s}: {s} }}",
                    .{ field.name, ident.parsed },
                ));
            }
        }
        // A union always needs at least one matching key, so it is required.
        return .{ .parsed = try res.toOwnedSlice(self.arena_alloc), .optional = false };
    }

    /// A single flattened query-param leaf key.
    const FlatLeaf = struct {
        name: []const u8,
        ts_type: []const u8,
        optional: bool,
    };

    /// A `query_params` struct represented as a flat structure.
    /// `independent` - Leaves that are not in any kind of group.
    /// `groups` - Each set of leaves that must exist together in a group (for `RequireTogether`).
    const FlatStruct = struct {
        independent: ArrayList(FlatLeaf) = .empty,
        groups: ArrayList(ArrayList(FlatLeaf)) = .empty,
    };

    /// Builds the flat TS shape for a struct query parameter.
    ///
    /// Returns null when the struct contributes no keys (no leaves and no `require_at_least_one`),
    /// so it can be omitted from the generated types.
    ///
    /// Union fields are skipped here (see `collectFlatLeaves`) and handled by the caller.
    fn buildFlatStruct(self: *Self, comptime T: type, S: Type.Struct) !?[]const u8 {
        var flat_struct: FlatStruct = .{};
        try self.collectFlatLeaves(&flat_struct, S, false, null);

        const require_at_least_one = comptime blk: {
            if (@hasDecl(T, "constraints")) break :blk T.constraints.require_at_least_one;
            break :blk false;
        };

        var all: ArrayList(FlatLeaf) = .empty;
        try all.appendSlice(self.arena_alloc, flat_struct.independent.items);
        for (flat_struct.groups.items) |group| {
            try all.appendSlice(self.arena_alloc, group.items);
        }

        if (all.items.len == 0 and !require_at_least_one) return null;

        // No groups means the full object is the base, optionally wrapped in RequireAtLeastOne
        if (flat_struct.groups.items.len == 0) {
            const full = try self.renderLeafObject(all.items);
            if (require_at_least_one) {
                return try allocPrint(self.arena_alloc, "RequireAtLeastOne<{s}>", .{full});
            }
            return full;
        }

        // One RequireTogether per group, the independent keys as a plain object,
        // and RequireAtLeastOne over everything if the constraint is set.
        var components: ArrayList([]const u8) = .empty;

        // Each group object contains only its own keys.
        for (flat_struct.groups.items) |group| {
            try components.append(self.arena_alloc, try allocPrint(
                self.arena_alloc,
                "RequireTogether<{s}>",
                .{try self.renderLeafObject(group.items)},
            ));
        }

        if (flat_struct.independent.items.len > 0) {
            try components.append(self.arena_alloc, try self.renderLeafObject(flat_struct.independent.items));
        }

        if (require_at_least_one) {
            try components.append(self.arena_alloc, try allocPrint(
                self.arena_alloc,
                "RequireAtLeastOne<{s}>",
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

    /// Returns if the field is a non-Optional tagged union (without `paramParse`).
    fn isUnionField(field: Type.StructField) bool {
        return (comptime @typeInfo(field.type) == .@"union" //
        and !isOptional(field.type) //
        and !hasParamParse(field.type));
    }

    /// Whether `T` is a struct with at least one field
    /// that is a non-Optional tagged union (without `paramParse`),
    /// i.e. a union to be lifted into the flat query key space.
    fn isStructContainingUnionField(comptime T: type) bool {
        if (@typeInfo(T) != .@"struct") return false;
        inline for (@typeInfo(T).@"struct".fields) |field| {
            if (comptime isUnionField(field)) return true;
        }
        return false;
    }

    /// Emits the flat TS shape for a query param struct which contains lifted union fields.
    /// Each union field's variant alternatives live directly in the flat key space,
    /// so the result is the intersection of every union's flat shape,
    /// with the base object built from the remaining fields.
    fn parseFlatQueryStruct(self: *Self, comptime T: type, S: Type.Struct) !ParseResult {
        var components: ArrayList([]const u8) = .empty;

        inline for (S.fields) |field| {
            if (comptime isUnionField(field)) {
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
        // A union always needs at least one matching key, so it is required.
        return .{ .parsed = try res.toOwnedSlice(self.arena_alloc), .optional = false };
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
            if (comptime isUnionField(field)) continue;

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
                    // RequireTogether needs >= 2 keys, a single key is just independent.
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

                const res: ParseResult = if (self.top_level_types.get(type_name)) |gen|
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
                if (self.top_level_types.get(type_name)) |gen| {
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
    try std.testing.expectEqualStrings(
        \\{
        \\foo?: number|null
        \\bar: number|null
        \\baz?: number
        \\}
    ,
        parse_result.parsed,
    );
}

test "Nested Optionals" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    const Optional = @import("utils/types.zig").Optional;

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
    try std.testing.expectEqualStrings(parse_result.parsed,
        \\{
        \\enabled?: boolean
        \\email?: {
        \\enabled?: boolean
        \\threshold?: number
        \\}
        \\}
    );
}

test "Optionals require default values" {
    const alloc = std.testing.allocator;

    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();

    const Optional = @import("utils/types.zig").Optional;

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
    try std.testing.expectEqualStrings(parse_result.parsed,
        \\{
        \\list: {
        \\abc: number
        \\}[]
        \\}
    );
}
