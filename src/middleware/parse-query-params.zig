const std = @import("std");
const comptimePrint = std.fmt.comptimePrint;
const json = std.json;
const Allocator = std.mem.Allocator;

const zap = @import("../zap/zap.zig");
const MiddlewareContext = zap.Endpoint.MiddlewareContext;
const MiddlewareFn = zap.Endpoint.MiddlewareFn;
const Request = zap.Request;
const HttpError = zap.HttpError;
const StatusCode = zap.StatusCode;
const allocPrint = std.fmt.allocPrint;
const parseInt = std.fmt.parseInt;
const parseFloat = std.fmt.parseFloat;
const Type = std.builtin.Type;
const hasFn = std.meta.hasFn;

const types = @import("../utils/types.zig");
const isOptional = types.isOptional;
const Optional = types.Optional;

const query_params = "query_params";

const VariantChoice = union(enum) {
    /// Field index of the unique most specific matching variant
    selected: usize,
    /// Two or more variants match (a tie between variants)
    ambiguous,
    /// No specific variant matched, the caller may use the all optional fallback
    none,
};

/// - `all_req_keys_present[i]`: if variant i's required keys are all present
/// - `num_req_keys[i]`: variant i's number of required keys.
///
/// A variant requiring more keys is more specific than one requiring a subset of them.
/// If the highest score is shared by two or more matches, selection is `ambiguous`.
///
/// With no matches at all, the result is `none`.
fn chooseVariant(comptime n: usize, all_req_keys_present: [n]bool, num_req_keys: [n]usize) VariantChoice {
    var winner: ?usize = null;
    var highest_score: usize = 0;
    var tied = false;
    for (0..n) |i| {
        if (!all_req_keys_present[i]) continue;
        if (winner == null or num_req_keys[i] > highest_score) {
            winner = i;
            highest_score = num_req_keys[i];
            tied = false;
        } else if (num_req_keys[i] == highest_score) {
            tied = true;
        }
    }

    if (tied) {
        return .ambiguous;
    } else if (winner) |i| {
        return .{ .selected = i };
    }
    return .none;
}

/// Whether a struct/union type carries a custom `paramParse`
/// and therefore stays a single query key (its value parsed from a string),
/// rather than being flattened into leaf keys.
fn hasParamParse(comptime T: type) bool {
    return comptime hasFn(T, "paramParse");
}

/// Returns if a leaf/field is not required
/// (a custom `Optional`, a native `?T`, or a field with a default value).
fn isNotRequired(comptime field: Type.StructField) bool {
    return comptime isOptional(field.type) or
        @typeInfo(field.type) == .optional or
        field.defaultValue() != null;
}

/// Number of required leaf keys of the struct, recursing into nested plain
/// structs (which are flattened into their leaf keys).
fn getStructRequiredKeyCount(comptime S: Type.Struct) usize {
    comptime {
        var count: usize = 0;
        for (S.fields) |field| {
            if (isNotRequired(field)) continue;

            const info = @typeInfo(field.type);
            if (info == .@"struct" and !hasParamParse(field.type)) {
                count += getStructRequiredKeyCount(info.@"struct");
            } else {
                count += 1;
            }
        }
        return count;
    }
}

fn getVariantRequiredKeyCount(comptime variant: Type.UnionField) usize {
    const info = @typeInfo(variant.type);
    if (comptime info != .@"struct" or hasParamParse(variant.type)) {
        // Normal union variants are keyed by name, so we only have to return 1.
        return 1;
    }
    return getStructRequiredKeyCount(info.@"struct");
}

fn keysContain(keys: []const []const u8, name: []const u8) bool {
    for (keys) |k| {
        if (std.mem.eql(u8, k, name)) return true;
    }
    return false;
}

/// Returns if every required leaf key of struct `T` is in `present_keys`.
fn requiredKeysPresent(comptime T: type, present_keys: []const []const u8) bool {
    inline for (@typeInfo(T).@"struct".fields) |field| {
        if (comptime isNotRequired(field)) continue;

        if (comptime @typeInfo(field.type) == .@"struct" and !hasParamParse(field.type)) {
            if (!requiredKeysPresent(field.type, present_keys)) return false;
        } else if (!keysContain(present_keys, field.name)) {
            return false;
        }
    }
    return true;
}

/// Returns if the present query `keys` satisfy `variant`.
/// A struct variant needs all its required keys present,
/// and any other variant needs its name key to be present.
fn variantMatchesKeys(comptime variant: Type.UnionField, keys: []const []const u8) bool {
    const T = variant.type;
    if (comptime @typeInfo(T) == .@"struct" and !hasParamParse(T)) {
        return requiredKeysPresent(T, keys);
    }
    return keysContain(keys, variant.name);
}

/// Field index of union `T`'s all-optional fallback variant (zero required keys),
/// or null if it has none.
/// At most one may exist (enforced here at compile time).
fn fallbackVariantIndex(comptime T: type) ?usize {
    comptime {
        var found: ?usize = null;
        for (@typeInfo(T).@"union".fields, 0..) |variant, i| {
            if (getVariantRequiredKeyCount(variant) == 0) {
                if (found != null) {
                    @compileError("Union " ++ @typeName(T) ++
                        " has more than one all-optional variant;" ++
                        " at most one may act as the fallback.");
                }
                found = i;
            }
        }
        return found;
    }
}

/// Variant selection for union `T` from the set of present query keys.
///
/// Each specific variant (>= 1 required key) is scored by its required key count,
/// and `chooseVariant` picks the most specific match.
///
/// When no specific variant matches, the all-optional fallback variant is selected if one exists.
/// Otherwise, the result is `.none`.
///
/// `.selected` always names a variant ready to build, and `.none` means there is nothing to build.
fn selectVariant(comptime T: type, present_keys: []const []const u8) VariantChoice {
    const fields = @typeInfo(T).@"union".fields;
    // Tracking data for each variant, by index
    var all_req_keys_present = [_]bool{false} ** fields.len;
    var num_req_keys = [_]usize{0} ** fields.len;

    inline for (fields, 0..) |variant, i| {
        const required = comptime getVariantRequiredKeyCount(variant);
        if (required == 0) continue;

        if (variantMatchesKeys(variant, present_keys)) {
            all_req_keys_present[i] = true;
            num_req_keys[i] = required;
        }
    }

    const choice = chooseVariant(fields.len, all_req_keys_present, num_req_keys);
    if (choice == .none) {
        if (comptime fallbackVariantIndex(T)) |i| return .{ .selected = i };
    }
    return choice;
}

/// Parses the query params of the request and attaches it to the given Context.
/// Context must have a member named after each query param,
/// which resolves to the type meant to be parsed into an object.
pub fn parseQueryParams(comptime Context: type) MiddlewareFn(Context) {
    if (!@hasField(Context, query_params)) {
        @compileError(
            comptimePrint(
                "{s} property was not provided for query params middleware.",
                .{query_params},
            ),
        );
    }

    if (comptime isOptional(@FieldType(Context, query_params))) {
        @compileError(
            comptimePrint(
                "{s} must not be optional.",
                .{query_params},
            ),
        );
    }

    return struct {
        fn sendInvalidParamTypeResponse(
            alloc: Allocator,
            req: Request,
            ExpectedType: type,
            field_name: []const u8,
        ) !void {
            return try req.respondWithError(
                StatusCode.bad_request,
                try allocPrint(
                    alloc,
                    "Incorrect query parameter type for {s} - Expected {any}",
                    .{ field_name, ExpectedType },
                ),
            );
        }

        /// Helper function for handleQueryParam.
        /// Returns true if the middleware should exit early.
        fn _handleQueryParam(comptime FieldType: type, alloc: Allocator, param: []const u8) Optional(FieldType) {
            const info = @typeInfo(FieldType);
            switch (info) {
                .bool => {
                    if (std.mem.eql(u8, param, "true")) {
                        return .{ .value = true };
                    } else if (std.mem.eql(u8, param, "false")) {
                        return .{ .value = false };
                    } else {
                        return .not_provided;
                    }
                },
                .int => {
                    const val = parseInt(FieldType, param, 10) catch {
                        return .not_provided;
                    };
                    return .{ .value = val };
                },
                .float => {
                    const val = parseFloat(FieldType, param) catch {
                        return .not_provided;
                    };
                    return .{ .value = val };
                },
                .pointer => {
                    const ChildT = info.pointer.child;
                    if (ChildT == u8) {
                        // Strings arrive here
                        return .{ .value = param };
                    } else {
                        const value = parseArrayFromString(alloc, ChildT, param) catch {
                            return .not_provided;
                        };
                        return .{ .value = value };
                    }
                },
                .@"enum" => {
                    if (std.meta.stringToEnum(FieldType, param)) |v| {
                        return .{ .value = v };
                    } else {
                        return .not_provided;
                    }
                },
                .@"struct", .@"union" => {
                    if (hasParamParse(FieldType)) {
                        const parsed: FieldType = FieldType.paramParse(alloc, param) catch {
                            std.log.err(
                                "query param failed to parse as custom: {s} - {s}",
                                .{ @typeName(FieldType), param },
                            );
                            return .not_provided;
                        };
                        return .{ .value = parsed };
                    }

                    if (info == .@"struct") {
                        @compileError("Must define paramParse for struct: " ++ @typeName(FieldType));
                    }

                    // Note: `untagged` union parsing
                    inline for (@typeInfo(FieldType).@"union".fields) |f| {
                        if (f.type == void) {
                            if (std.mem.eql(u8, f.name, param)) {
                                return .{ .value = @unionInit(FieldType, f.name, {}) };
                            }
                        } else if (_handleQueryParam(f.type, alloc, param).get()) |v| {
                            return .{ .value = @unionInit(FieldType, f.name, v) };
                        }
                    }
                    return .not_provided;
                },
                .optional => {
                    if (std.mem.eql(u8, param, "null")) {
                        return .{ .value = null };
                    } else {
                        // Optional(T) -> Optional(?T)
                        switch (_handleQueryParam(info.optional.child, alloc, param)) {
                            .value => |v| {
                                // Implicit conversion: ?T -> T
                                return .{ .value = v };
                            },
                            .not_provided => {
                                return .not_provided;
                            },
                        }
                    }
                },
                else => {
                    return .not_provided;
                },
            }
            @compileError("unreachable");
        }

        fn parseArrayFromString(alloc: Allocator, comptime T: type, str: []const u8) ![]T {
            if (str.len < 1) {
                return error.InvalidArray;
            }

            var list: std.ArrayList(T) = .empty;
            var it = std.mem.splitSequence(u8, str, ",");
            while (it.next()) |val_str| {
                var val: T = undefined;
                switch (@typeInfo(T)) {
                    .int => {
                        val = parseInt(T, std.mem.trim(u8, val_str, " "), 10) catch return error.InvalidArray;
                    },
                    .float => {
                        val = parseFloat(T, std.mem.trim(u8, val_str, " ")) catch return error.InvalidArray;
                    },
                    .pointer => {
                        // Array of strings
                        const ChildT = @typeInfo(T).pointer.child;
                        if (ChildT != u8) @compileError("Only array of strings is supported");
                        val = val_str;
                    },
                    .@"enum" => {
                        if (std.meta.stringToEnum(T, val_str)) |v| {
                            val = v;
                        } else {
                            return error.InvalidEnumVariant;
                        }
                    },
                    else => @compileError(
                        std.fmt.comptimePrint("Unsupported query param array child type: {s}", .{@typeName(T)}),
                    ),
                }
                try list.append(alloc, val);
            }
            return try list.toOwnedSlice(
                alloc,
            );
        }

        /// Returns whether any leaf key of struct `T` is present in the request.
        /// This recurses into nested plain structs, which are flattened.
        fn anyLeafPresent(comptime T: type, alloc: Allocator, req: Request) !bool {
            inline for (@typeInfo(T).@"struct".fields) |field| {
                const is_optional = comptime isOptional(field.type);
                const F = if (is_optional) field.type.childType() else field.type;
                if (comptime !is_optional and @typeInfo(F) == .@"union" and !hasParamParse(F)) {
                    // A lifted union is present if any of its variant keys are.
                    inline for (@typeInfo(F).@"union".fields) |variant| {
                        if (try isVariantPresent(variant, alloc, req)) return true;
                    }
                } else if (comptime @typeInfo(F) == .@"struct" and !hasParamParse(F)) {
                    if (try anyLeafPresent(F, alloc, req)) return true;
                } else if ((try req.getParamDecoded(alloc, field.name)) != null) {
                    return true;
                }
            }
            return false;
        }

        /// A successfully parsed value together with the query keys it consumed.
        fn Parsed(comptime T: type) type {
            return struct { value: T, consumed: std.ArrayList([]const u8) };
        }

        /// Creates a struct from the query's key/value pairs.
        /// Returns null if an error response was sent.
        fn parseFlatStruct(comptime T: type, alloc: Allocator, req: Request) !?Parsed(T) {
            var result: T = undefined;
            var consumed: std.ArrayList([]const u8) = .empty;
            inline for (@typeInfo(T).@"struct".fields) |field| {
                const is_optional = comptime isOptional(field.type);
                const FieldType = if (is_optional) field.type.childType() else field.type;
                const field_info = @typeInfo(FieldType);

                if (comptime !is_optional and field_info == .@"union" and !hasParamParse(FieldType)) {
                    const u = (try parseFlatUnionValue(FieldType, alloc, req)) orelse return null;
                    @field(result, field.name) = u.value;
                    try consumed.appendSlice(alloc, u.consumed.items);
                } else if (comptime field_info == .@"struct" and !hasParamParse(FieldType)) {
                    if (try anyLeafPresent(FieldType, alloc, req)) {
                        const s = (try parseFlatStruct(FieldType, alloc, req)) orelse return null;
                        @field(result, field.name) = if (is_optional) .to(s.value) else s.value;
                        try consumed.appendSlice(alloc, s.consumed.items);
                    } else if (is_optional) {
                        @field(result, field.name) = .not_provided;
                    } else if (field.defaultValue()) |v| {
                        @field(result, field.name) = v;
                    } else {
                        // Required nested struct with no keys present.
                        //
                        // If it has a required leaf,
                        // parseFlatStruct sends the specific "Missing query parameter" error and returns null.
                        //
                        // If it's entirely optional/has defaults,
                        // it succeeds and we assign that all-optional value here.
                        const s = (try parseFlatStruct(FieldType, alloc, req)) orelse return null;
                        @field(result, field.name) = s.value;
                        try consumed.appendSlice(alloc, s.consumed.items);
                    }
                } else {
                    if (try req.getParamDecoded(alloc, field.name)) |param| {
                        try consumed.append(alloc, field.name);
                        switch (_handleQueryParam(FieldType, alloc, param.items)) {
                            .value => |v| @field(result, field.name) = if (is_optional) .to(v) else v,
                            .not_provided => {
                                try sendInvalidParamTypeResponse(alloc, req, FieldType, field.name);
                                return null;
                            },
                        }
                    } else if (field.defaultValue()) |v| {
                        @field(result, field.name) = v;
                    } else if (is_optional) {
                        @field(result, field.name) = .not_provided;
                    } else if (field_info == .optional) {
                        @field(result, field.name) = null;
                    } else {
                        try req.respondWithError(
                            StatusCode.bad_request,
                            try allocPrint(alloc, "Missing query parameter: {s}", .{field.name}),
                        );
                        return null;
                    }
                }
            }
            return .{ .value = result, .consumed = consumed };
        }

        /// Returns whether the keys identifying `variant` are present.
        /// A plain struct variant matches on any of its flattened leaf keys.
        /// Any other variant matches on its variant name used as a key.
        fn isVariantPresent(comptime variant: Type.UnionField, alloc: Allocator, req: Request) !bool {
            const T = variant.type;
            if (comptime @typeInfo(T) == .@"struct" and !hasParamParse(T)) {
                return try anyLeafPresent(T, alloc, req);
            }
            return (try req.getParamDecoded(alloc, variant.name)) != null;
        }

        /// Builds the `variant` of union `T` from the request's flat query keys.
        /// Returns null if an error response was sent.
        fn buildVariant(
            comptime T: type,
            comptime variant: Type.UnionField,
            alloc: Allocator,
            req: Request,
        ) !?Parsed(T) {
            const V = variant.type;
            var consumed: std.ArrayList([]const u8) = .empty;

            if (V == void) {
                try consumed.append(alloc, variant.name);
                return .{
                    .value = @unionInit(T, variant.name, {}),
                    .consumed = consumed,
                };
            } else if (comptime @typeInfo(V) == .@"struct" and !hasParamParse(V)) {
                const s = try parseFlatStruct(V, alloc, req) orelse return null;
                if (types.validateConstraints(V, s.value)) |err_msg| {
                    try req.respondWithError(StatusCode.bad_request, err_msg);
                    return null;
                }
                return .{
                    .value = @unionInit(T, variant.name, s.value),
                    .consumed = s.consumed,
                };
            } else {
                const param = try req.getParamDecoded(alloc, variant.name) orelse return error.Unreachable;
                switch (_handleQueryParam(V, alloc, param.items)) {
                    .value => |v| {
                        try consumed.append(alloc, variant.name);
                        return .{
                            .value = @unionInit(T, variant.name, v),
                            .consumed = consumed,
                        };
                    },
                    .not_provided => {
                        try sendInvalidParamTypeResponse(alloc, req, V, variant.name);
                        return null;
                    },
                }
            }
        }

        /// Resolves a tagged union `T` from flat query keys.
        ///
        /// A variant matches when all of its required keys are present,
        /// and the most specific match wins.
        /// Among matching variants, we pick the one requiring the most keys.
        /// Optional keys never affect selection.
        ///
        /// This mirrors the generated `XOR` types:
        /// a variant requiring `{start_date, end_date}` is chosen over one requiring only `{start_date}`
        /// when both keys are present,
        /// while `start_date` alone selects the latter.
        ///
        /// Variants with equal required key sets are rejected at compile time,
        /// so they can always be told apart by their required keys.
        ///
        /// A runtime tie is still possible
        /// when a request supplies the keys of two different same-count variants at once
        /// (e.g. `{x}` and `{y}` both present),
        /// which results in a 400 error.
        ///
        /// A single all-optional variant (zero required keys) acts as the fallback,
        /// used when no other variant matches, including an empty query.
        /// At most one fallback may exist (enforced at compile time).
        ///
        /// Returns null if an error response was sent.
        fn parseFlatUnionValue(comptime T: type, alloc: Allocator, req: Request) !?Parsed(T) {
            const fields = @typeInfo(T).@"union".fields;

            var present_keys: std.ArrayList([]const u8) = .empty;
            var it = req.getParamSlices();
            while (it.next()) |pair| try present_keys.append(alloc, pair.name);

            switch (selectVariant(T, present_keys.items)) {
                .selected => |selected_index| {
                    inline for (fields, 0..) |variant, i| {
                        if (i == selected_index) {
                            return try buildVariant(T, variant, alloc, req);
                        }
                    }
                    // Shouldn't be possible, would be a logic error
                    return error.Unreachable;
                },
                .ambiguous => {
                    try req.respondWithError(
                        StatusCode.bad_request,
                        "Query parameters match more than one variant",
                    );
                    return null;
                },
                .none => {
                    try req.respondWithError(
                        StatusCode.bad_request,
                        "No matching query parameters were provided",
                    );
                    return null;
                },
            }
        }

        /// Sends a 400 error if there are any unexpected query params.
        /// Returns true if an error response was sent.
        fn rejectUnexpectedParams(alloc: Allocator, req: Request, consumed: []const []const u8) !bool {
            var unexpected: std.ArrayList([]const u8) = .empty;

            // Find any supplied query_params that aren't in `consumed`.
            var it = req.getParamSlices();
            while (it.next()) |param| {
                if (!keysContain(consumed, param.name)) {
                    try unexpected.append(alloc, param.name);
                }
            }
            if (unexpected.items.len == 0) return false;

            var msg: std.ArrayList(u8) = .empty;
            try msg.appendSlice(alloc, "Unexpected query parameters were provided: ");

            for (unexpected.items, 0..) |name, i| {
                if (i != 0) try msg.appendSlice(alloc, ", ");
                try msg.appendSlice(alloc, name);
            }

            try req.respondWithError(StatusCode.bad_request, msg.items);
            return true;
        }

        fn parseQueryParams(ctx: *MiddlewareContext(Context)) anyerror!void {
            const QueryType = @FieldType(Context, query_params);

            // `query_params` as a tagged union is parsed from a flat key/value set,
            // with the active variant inferred from which keys are present.
            if (comptime @typeInfo(QueryType) == .@"union") {
                const parsed = (try parseFlatUnionValue(QueryType, ctx.alloc, ctx.req)) orelse return;
                if (try rejectUnexpectedParams(ctx.alloc, ctx.req, parsed.consumed.items)) return;
                @field(ctx.ctx, query_params) = parsed.value;
                return;
            }

            var all_fields_are_optional = true;
            outer: inline for (@typeInfo(Context).@"struct".fields) |ctx_field| {
                if (comptime std.mem.eql(u8, ctx_field.name, query_params)) {
                    inline for (@typeInfo(ctx_field.type).@"struct".fields) |field| {
                        if (!isOptional(field.type) and field.defaultValue() == null) {
                            all_fields_are_optional = false;
                            break :outer;
                        }
                    }
                }
            }

            if (!all_fields_are_optional and ctx.req.isQueryEmpty()) {
                return try ctx.req.respondWithError(
                    StatusCode.bad_request,
                    "No query params were provided",
                );
            }

            const parsed = (try parseFlatStruct(QueryType, ctx.alloc, ctx.req)) orelse return;

            if (types.validateConstraints(QueryType, parsed.value)) |err_msg| {
                return try ctx.req.respondWithError(StatusCode.bad_request, err_msg);
            }

            if (try rejectUnexpectedParams(ctx.alloc, ctx.req, parsed.consumed.items)) return;

            @field(ctx.ctx, query_params) = parsed.value;
        }
    }.parseQueryParams;
}

// selectVariant tests

const Basic = struct {
    start_date: []const u8,
};

const Detailed = struct {
    start_date: []const u8,
    end_date: []const u8,
};

const RangeUnion = union(enum) {
    basic: Basic,
    detailed: Detailed,
    all: struct {
        page: ?u32 = null,
    },
};

const CustomParam = struct {
    n: u32,
    pub fn paramParse(_: []const u8) ?@This() {
        return null;
    }
};

test "getVariantRequiredKeyCount: scores by required key count" {
    const fields = @typeInfo(RangeUnion).@"union".fields;
    // basic
    try std.testing.expectEqual(1, getVariantRequiredKeyCount(fields[0]));
    // detailed
    try std.testing.expectEqual(2, getVariantRequiredKeyCount(fields[1]));
    // fallback (all optional leaves = score of 0)
    try std.testing.expectEqual(0, getVariantRequiredKeyCount(fields[2]));
}

test "getVariantRequiredKeyCount: flattened nested keys, single leaf variants score 1" {
    const U = union(enum) {
        nested: struct {
            range: struct {
                start: []const u8,
                end: []const u8,
            },
            opt: ?u32 = null,
        },
        custom: CustomParam,
        flag,
    };

    const fields = @typeInfo(U).@"union".fields;
    // Nested flattens to { start, end }, and `opt` is optional and excluded
    try std.testing.expectEqual(2, getVariantRequiredKeyCount(fields[0]));
    // Structs using `paramParse` resolve to a single key
    try std.testing.expectEqual(1, getVariantRequiredKeyCount(fields[1]));
    // void variant is keyed by its name
    try std.testing.expectEqual(1, getVariantRequiredKeyCount(fields[2]));
}

test "selectVariant: single required key selects Basic variant" {
    const choice = selectVariant(RangeUnion, &.{"start_date"});
    try std.testing.expectEqual(.selected, choice);
    try std.testing.expectEqual(0, choice.selected);
}

test "selectVariant: most specific match wins (Detailed over Basic)" {
    const choice = selectVariant(RangeUnion, &.{ "start_date", "end_date" });
    try std.testing.expectEqual(.selected, choice);
    try std.testing.expectEqual(1, choice.selected);
}

test "selectVariant: empty query selects the all optional fallback" {
    const choice = selectVariant(RangeUnion, &.{});
    try std.testing.expectEqual(.selected, choice);
    try std.testing.expectEqual(2, choice.selected);
}

test "selectVariant: fallback only keys select the all optional fallback" {
    const choice = selectVariant(RangeUnion, &.{"page"});
    try std.testing.expectEqual(.selected, choice);
    try std.testing.expectEqual(2, choice.selected);
}

test "selectVariant: no specific match and no fallback -> none" {
    // Neither variant is all-optional, so an unmatched query selects `.none`
    const U = union(enum) {
        a: struct { x: []const u8 },
        b: struct { y: []const u8 },
    };
    try std.testing.expect(selectVariant(U, &.{"z"}) == .none);
}

test "selectVariant: different same-count variants are ambiguous" {
    const Xor = union(enum) {
        a: struct { x: []const u8 },
        b: struct { y: []const u8 },
    };
    try std.testing.expect(selectVariant(Xor, &.{ "x", "y" }) == .ambiguous);

    const a = selectVariant(Xor, &.{"x"});
    try std.testing.expect(a == .selected and a.selected == 0);

    const b = selectVariant(Xor, &.{"y"});
    try std.testing.expect(b == .selected and b.selected == 1);
}

test "selectVariant: nested struct needs all its required leaf keys" {
    const U = union(enum) {
        nested: struct {
            range: struct {
                start: []const u8,
                end: []const u8,
            },
        },
        other: struct {
            z: []const u8,
        },
    };

    // Only one nested leaf present -> nested does not match
    try std.testing.expect(selectVariant(U, &.{"start"}) == .none);

    // Both nested leaves present -> nested selected
    const both = selectVariant(U, &.{ "start", "end" });
    try std.testing.expect(both == .selected and both.selected == 0);
}

test "selectVariant: name keyed variants match on their variant name" {
    const U = union(enum) {
        custom: CustomParam,
        flag,
        other: struct {
            z: []const u8,
        },
    };

    {
        const choice = selectVariant(U, &.{"custom"});
        try std.testing.expect(choice == .selected and choice.selected == 0);
    }

    {
        const choice = selectVariant(U, &.{"flag"});
        try std.testing.expect(choice == .selected and choice.selected == 1);
    }

    {
        const choice = selectVariant(U, &.{"z"});
        try std.testing.expect(choice == .selected and choice.selected == 2);
    }
}
