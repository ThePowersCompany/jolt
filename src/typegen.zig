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

const ts_file_name = "types.d.ts";
const endpoint_fn_names = [_][]const u8{ "get", "post", "put", "patch", "delete" };

fn strEqls(s1: []const u8, s2: []const u8) bool {
    return std.mem.eql(u8, s1, s2);
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

pub fn generateTypesFile(alloc: Allocator, endpoints: []const EndpointDef) !void {
    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();
    const arena_alloc = arena.allocator();

    var ts = ArrayList(u8).init(arena_alloc);
    defer ts.deinit();

    try ts.appendSlice(
        \\ // === DO NOT MODIFY ===
        \\ //
        \\ // Auto-generated type definitions
        \\ // Regenerate the types with `zig build types`
        \\ //
        \\ // === DO NOT MODIFY ===
        \\
        \\
    );

    var type_generator = try TypeGenerator.init(arena_alloc);
    defer type_generator.deinit();

    try ts.appendSlice(try type_generator.generateTypes(endpoints));

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
        var iter = std.mem.splitBackwardsScalar(u8, type_name, '.');
        return iter.first();
    }

    pub fn generateTypes(self: *Self, comptime endpoints: []const EndpointDef) ![]const u8 {
        // First pass: Find all pub top-level types across all endpoint files
        inline for (endpoints) |endpoint| {
            _, const EndpointType = endpoint;
            const type_info = @typeInfo(EndpointType);
            inline for (type_info.@"struct".decls) |decl| {
                const decl_info = @typeInfo(@TypeOf(@field(EndpointType, decl.name)));
                switch (decl_info) {
                    .type => {
                        const t_info = @typeInfo(@field(EndpointType, decl.name));
                        if (t_info == .@"struct") {
                            try self.setTopLevelType(decl.name, .{ .parsed = "", .optional = false });
                        }
                    },
                    else => {},
                }
            }
        }

        // Second pass: Generate typings for all top-level types across all endpoint files
        inline for (endpoints) |endpoint| {
            _, const EndpointType = endpoint;
            const type_info = @typeInfo(EndpointType);
            inline for (type_info.@"struct".decls) |decl| {
                const decl_info = @typeInfo(@TypeOf(@field(EndpointType, decl.name)));
                switch (decl_info) {
                    .type => {
                        const t_info = @typeInfo(@field(EndpointType, decl.name));
                        switch (t_info) {
                            .@"struct" => |s| {
                                const res = try self.parseStruct(decl.name, s);
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

        var res = ArrayList(u8).init(self.arena_alloc);

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
            var entries = ArrayList(Entry).init(self.arena_alloc);
            defer entries.deinit();

            var iter = self.top_level_types.iterator();
            while (iter.next()) |top| {
                try entries.append(.{
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
                try res.appendSlice(entry.ts);
            }
        }

        // Print http method endpoint interface typings
        inline for (@typeInfo(Method).@"enum".fields) |m| {
            const method: Method = @enumFromInt(m.value);
            var endpoints_data = self.endpointsData(method);

            try res.appendSlice(
                try allocPrint(
                    self.arena_alloc,
                    "export interface {c}{s}Endpoints {{",
                    .{ std.ascii.toUpper(m.name[0]), m.name[1..] },
                ),
            );

            var iter = endpoints_data.iterator();
            while (iter.next()) |endpoint| {
                try res.appendSlice(try allocPrint(self.arena_alloc, "\"{s}\": {{\n", .{endpoint.key_ptr.*}));

                if (endpoint.value_ptr.query_params) |query_params| {
                    try res.appendSlice(query_params);
                }

                if (endpoint.value_ptr.body) |body| {
                    try res.appendSlice(body);
                } else if (method != .get) {
                    try res.appendSlice("  body?: never,\n");
                }

                if (endpoint.value_ptr.response) |response| {
                    try res.appendSlice(
                        try allocPrint(self.arena_alloc, "response: {s},\n", .{response}),
                    );
                }

                try res.appendSlice("}\n");
            }
            try res.appendSlice("}\n\n");
        }

        return res.toOwnedSlice();
    }

    fn genTypescript(self: *Self, endpoint: EndpointDef) !void {
        const endpoint_path, const EndpointType = endpoint;
        const type_info = @typeInfo(EndpointType);
        inline for (type_info.@"struct".decls) |decl| {
            const decl_info = @typeInfo(@TypeOf(@field(EndpointType, decl.name)));
            switch (decl_info) {
                // Find get/post/put/patch/delete functions
                .@"fn" => try self.populateFnTypescript(decl, decl_info.@"fn", endpoint_path),
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
                const param_res = try self.extractIdentifier(field.type);
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
        var union_ts: []const u8 = undefined;
        {
            inline for (S.fields) |field| {
                const info = @typeInfo(field.type);
                if (info != .@"union") continue;

                union_ts = try self.parseUnion(info.@"union", field.type);

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
        var res = ArrayList(u8).init(self.arena_alloc);
        try res.appendSlice("{\n");
        inline for (S.fields) |field| {
            try res.appendSlice(field.name);

            const parse_result = try self.extractIdentifier(field.type);

            // TODO: This is a weird bug in defaultValue I had to work around...
            if (comptime strEqls(field.name, "_is_finished")) continue;

            const optional = if (field.defaultValue()) |_| true else parse_result.optional;

            all_optional = all_optional and optional;
            if (optional) {
                try res.appendSlice("?: ");
            } else {
                try res.appendSlice(": ");
            }

            try res.appendSlice(parse_result.parsed);
            try res.appendSlice("\n");
        }

        try res.appendSlice("}");

        return .{
            .optional = all_optional,
            .parsed = try res.toOwnedSlice(),
        };
    }

    /// Helper function for adjacent unions
    fn _parseUnionAsStruct(self: *Self, U: Type.Union) !ParseResult {
        var all_optional = true;
        var res = ArrayList(u8).init(self.arena_alloc);
        try res.appendSlice("{\n");
        inline for (U.fields) |field| {
            try res.appendSlice(field.name);

            const parse_result = try self.extractIdentifier(field.type);
            all_optional = all_optional and parse_result.optional;
            if (parse_result.optional) {
                try res.appendSlice("?: ");
            } else {
                try res.appendSlice(": ");
            }

            try res.appendSlice(parse_result.parsed);
            try res.appendSlice("\n");
        }

        try res.appendSlice("}");

        return .{
            .optional = all_optional,
            .parsed = try res.toOwnedSlice(),
        };
    }

    fn parseStructWithAdjacentUnion(self: *Self, S: Type.Struct, adjacent_union: AdjacentUnion) !ParseResult {
        const union_short_name = shortTypeName(adjacent_union.name);

        var res = ArrayList(u8).init(self.arena_alloc);
        try res.appendSlice("{\n");
        try res.appendSlice(try allocPrint(
            self.arena_alloc,
            "[K in keyof {s}]: {{\n",
            .{union_short_name},
        ));

        inline for (S.fields) |f| {
            if (strEqls(f.name, adjacent_union.discriminator)) {
                try res.appendSlice(f.name);
                try res.appendSlice(": K\n");
            } else if (strEqls(@typeName(f.type), adjacent_union.name)) {
                const field_info = @typeInfo(f.type);
                if (field_info != .@"union") {
                    return error.InvalidAdjacentUnionType;
                }

                try self.setTopLevelType(union_short_name, try self._parseUnionAsStruct(field_info.@"union"));
                try res.appendSlice(
                    try allocPrint(self.arena_alloc, "{s}: {s}[K]", .{ f.name, union_short_name }),
                );
            } else {
                try res.appendSlice(f.name);
                const parse_result = try self.extractIdentifier(f.type);
                if (parse_result.optional) {
                    try res.appendSlice("?: ");
                } else {
                    try res.appendSlice(": ");
                }
                try res.appendSlice(parse_result.parsed);
                try res.appendSlice("\n");
            }
        }

        try res.appendSlice(try allocPrint(
            self.arena_alloc,
            "}};\n}}[keyof {s}];\n",
            .{union_short_name},
        ));

        return .{ .parsed = try res.toOwnedSlice(), .optional = false };
    }

    fn parseEnum(self: *Self, E: Type.Enum) ![]const u8 {
        var res = ArrayList(u8).init(self.arena_alloc);
        try res.appendSlice(" | (\n");
        inline for (E.fields) |field| {
            try res.appendSlice(" | ");

            try res.appendSlice("\"");
            try res.appendSlice(field.name);
            try res.appendSlice("\"");
        }
        try res.appendSlice("\n)");
        return res.toOwnedSlice();
    }

    fn parseUnion(self: *Self, U: Type.Union, T: type) ![]const u8 {
        var union_repr: ?UnionRepr = null;
        inline for (U.decls) |decl| {
            if (comptime strEqls(decl.name, "_repr")) {
                union_repr = @field(T, decl.name);
                break;
            }
        }

        var res = ArrayList(u8).init(self.arena_alloc);

        if (union_repr) |repr| {
            switch (repr) {
                .external => {
                    return error.WeWerentUsingThisWhenIWroteTheTypegenLol;
                },
                .internal => {
                    const disc: []const u8 = repr.internal.discriminator;
                    inline for (U.fields) |field| {
                        try res.appendSlice(try allocPrint(
                            self.arena_alloc,
                            "\n | {{{s}: \"{s}\"; ",
                            .{ disc, field.name },
                        ));

                        const field_info: Type = @typeInfo(field.type);
                        if (field_info != .@"struct") return error.InvalidUnionRepr;

                        inline for (field_info.@"struct".fields) |f| {
                            const parsed_res = try self.extractIdentifier(f.type);
                            try res.appendSlice(try allocPrint(
                                self.arena_alloc,
                                "{s}{s}: {s}; ",
                                .{ f.name, if (parsed_res.optional or f.defaultValue() != null) "?" else "", parsed_res.parsed },
                            ));
                        }
                        try res.appendSlice(" }");
                    }
                },
                .adjacently => {
                    try res.appendSlice(try allocPrint(
                        self.arena_alloc,
                        "{{\n [K in keyof {s}]: {{\n",
                        .{shortTypeName(@typeName(T))},
                    ));
                    const disc: []const u8 = repr.adjacently.discriminator;
                    inline for (U.fields) |field| {
                        try res.appendSlice(try allocPrint(
                            self.arena_alloc,
                            "\n | {{{s}: \"{s}\"; ",
                            .{ disc, field.name },
                        ));

                        const field_info: Type = @typeInfo(field.type);
                        if (field_info != .@"struct") return error.InvalidUnionRepr;

                        inline for (field_info.@"struct".fields) |f| {
                            const parsed_res = try self.extractIdentifier(f.type);
                            try res.appendSlice(try allocPrint(
                                self.arena_alloc,
                                "{s}{s}: {s}; ",
                                .{ f.name, if (parsed_res.optional or f.defaultValue() != null) "?" else "", parsed_res.parsed },
                            ));
                        }
                        try res.appendSlice(" }");
                    }
                },
                .untagged => {
                    // Get the type of each enum state, join them together
                    var first = true;
                    inline for (U.fields) |field| {
                        if (first) {
                            first = false;
                        } else {
                            try res.appendSlice(" | ");
                        }
                        const ident = (try self.extractIdentifier(field.type)).parsed;
                        try res.appendSlice(ident);
                    }
                },
            }
        } else {
            std.log.err("{s} is missing a _repr declaration (must be public)", .{@typeName(T)});
            return error.MissingTaggedUnionRepr;
        }

        return res.toOwnedSlice();
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
                const type_name = shortTypeName(@typeName(T));
                if (self.top_level_types.get(type_name)) |gen| {
                    return .{ .parsed = type_name, .optional = gen.optional };
                }
                return try self.parseStruct(type_name, type_info.@"struct");
            },
            .@"enum" => {
                return .{ .parsed = try self.parseEnum(type_info.@"enum") };
            },
            .@"union" => {
                return .{ .parsed = try self.parseUnion(type_info.@"union", T) };
            },
            .optional => {
                return .{
                    .optional = true,
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

pub fn main() !void {
    var gpa = std.heap.GeneralPurposeAllocator(.{
        .thread_safe = true,
        .stack_trace_frames = 100,
    }){};

    {
        var arena = ArenaAllocator.init(gpa.allocator());
        defer arena.deinit();

        try generateTypesFile(arena.allocator(), &(@import("main.zig").endpoints));
    }

    const memory_leak = gpa.detectLeaks();
    if (memory_leak) {
        std.log.err("Memory leak!\n", .{});
    }
}
