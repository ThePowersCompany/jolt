const std = @import("std");
const comptimePrint = std.fmt.comptimePrint;
const Allocator = std.mem.Allocator;

const zap = @import("../zap/zap.zig");
const MiddlewareContext = zap.Endpoint.MiddlewareContext;
const Request = zap.Request;
const HttpError = zap.HttpError;
const StatusCode = zap.StatusCode;
const allocPrint = std.fmt.allocPrint;
const parseInt = std.fmt.parseInt;
const parseFloat = std.fmt.parseFloat;
const Type = std.builtin.Type;
const hasFn = std.meta.hasFn;

const types = @import("../utils/types.zig");
const Unwrap = types.Unwrap;
const Optional = types.Optional;
const isOptional = types.isOptional;

const containsString = @import("../utils/array_utils.zig").containsString;

const validation = @import("./query_params/validation.zig");
const assertNoQueryKeyCollisions = validation.assertNoQueryKeyCollisions;
const hasParamParse = validation.hasParamParse;
const getRequiredKeyCount = validation.getRequiredKeyCount;
const containerKind = validation.containerKind;
const findFallbackVariant = validation.findFallbackVariant;
const requiredKeysPresent = validation.requiredKeysPresent;
const anyStructLeafKeyPresent = validation.anyStructLeafKeyPresent;

const query_params = "query_params";

/// The fields of union `T`, ordered most specific first:
/// a field requiring more keys comes before one requiring fewer,
/// so the all-optional fallback (zero required keys) is always tried last.
/// The order is fixed at compile time and independent of the request,
/// so callers just walk it and build the first variant whose keys are present.
///
/// The sort is stable, so two fields requiring the same number of keys keep their declaration order.
/// Such a pair can only both match when the request mixes keys from both,
/// which is rejected later as unexpected query params,
/// so their relative order does not change the outcome of a valid request.
fn orderedFields(comptime T: type) [@typeInfo(T).@"union".fields.len]Type.UnionField {
    comptime {
        const Entry = struct {
            field: *const Type.UnionField,
            required_keys: usize,

            fn moreSpecific(_: void, a: @This(), b: @This()) bool {
                return a.required_keys > b.required_keys;
            }
        };

        const fields = @typeInfo(T).@"union".fields;
        var entries: [fields.len]Entry = undefined;
        for (fields, 0..) |f, i| {
            entries[i] = .{
                .field = &f,
                .required_keys = getRequiredKeyCount(f.type),
            };
        }

        std.sort.insertion(Entry, &entries, {}, Entry.moreSpecific);

        var sorted: [fields.len]Type.UnionField = undefined;
        for (entries, 0..) |e, i| sorted[i] = e.field.*;
        return sorted;
    }
}

/// Specificity score of `variant` given the present `keys`,
/// or null if it does not match.
/// A higher score is a more specific match (see `chooseVariant`).
///
/// - A plain struct variant matches when all its required leaf keys are present,
///     and at least one of its keys is present,
///     scored by its required leaf key count.
///     An all-optional struct with no key present does not match here,
///     and is left for the fallback (see `findFallbackVariant`).
/// - A nested composite union variant is scored by its matched inner variant:
///     we recurse and take the most specific inner variant that matches,
///     so an inner `{first, last}` beats an inner `{first}` when both keys are present.
/// - Any other variant matches on its name key, scored as 1.
pub fn variantMatchScore(comptime variant: Type.UnionField, keys: []const []const u8) ?usize {
    const T = variant.type;

    if (comptime @typeInfo(T) == .@"struct" and containerKind(T) == .composite) {
        if (!requiredKeysPresent(T, keys)) return null;
        if (!anyStructLeafKeyPresent(T, keys)) return null;
        return comptime getRequiredKeyCount(T);
    }

    if (comptime @typeInfo(T) == .@"union" and containerKind(T) == .composite) {
        var highest_score: ?usize = null;
        inline for (@typeInfo(T).@"union".fields) |inner| {
            if (variantMatchScore(inner, keys)) |score| {
                if (highest_score == null or score > highest_score.?) {
                    highest_score = score;
                }
            }
        }
        return highest_score;
    }

    if (!containsString(keys, variant.name)) return null;

    return 1;
}

pub fn ParseQueryResult(comptime ReturnType: type) type {
    return union(enum) {
        success: ReturnType,
        fail: []const u8,

        pub fn assert(self: @This()) !ReturnType {
            if (self != .success) {
                std.log.err("{s}", .{self.fail});
                try std.testing.expect(false);
            }
            return self.success;
        }
    };
}

/// A parse result that owns all of its allocations via an internal arena.
pub fn ParsedQuery(comptime ReturnType: type) type {
    return struct {
        const Self = @This();

        arena: *std.heap.ArenaAllocator,
        result: ParseQueryResult(ReturnType),

        pub fn deinit(self: Self) void {
            const alloc = self.arena.child_allocator;
            self.arena.deinit();
            alloc.destroy(self.arena);
        }

        pub fn assert(self: Self) !ReturnType {
            return self.result.assert();
        }
    };
}

const ParseCtx = struct {
    pub const Self = @This();

    alloc: Allocator,
    query: []const u8,
    /// The recorded failure message.
    /// Lives on `alloc`, so it needs no separate cleanup.
    failure: ?[]const u8 = null,
    /// The query keys consumed while parsing.
    consumed: std.ArrayList([]const u8) = .empty,

    /// Records a failure message.
    /// The message must live at least as long as the parse result,
    /// so pass a string literal or a string allocated with `alloc`.
    /// The caller should return null after calling this function.
    fn fail(self: *Self, message: []const u8) void {
        if (self.failure == null) self.failure = message;
    }

    /// Records `name` as a consumed query key.
    fn markConsumed(self: *Self, name: []const u8) !void {
        try self.consumed.append(self.alloc, name);
    }

    const Snapshot = struct {
        failure: ?[]const u8,
        consumed_len: usize,
    };

    /// Captures the current mutable state so a parse attempt can be undone.
    fn snapshot(self: *const Self) Snapshot {
        return .{
            .failure = self.failure,
            .consumed_len = self.consumed.items.len,
        };
    }

    /// Restores the state captured by `snapshot`,
    /// discarding any failure recorded and any keys consumed since.
    fn restore(self: *Self, snap: Snapshot) void {
        self.failure = snap.failure;
        self.consumed.shrinkRetainingCapacity(snap.consumed_len);
    }

    fn getParamDecoded(self: *const Self, name: []const u8) !?std.ArrayList(u8) {
        return Request.getParamDecodedFromQuery(self.alloc, self.query, name);
    }

    fn paramSlices(self: *const Self) Request.ParamSliceIterator {
        return Request.ParamSliceIterator.init(self.query);
    }
};

/// Records a 400 for a query parameter whose value could not be parsed into its expected type.
fn recordInvalidParamType(ctx: *ParseCtx, field_name: []const u8) !void {
    ctx.fail(try allocPrint(ctx.alloc, "Incorrect query parameter for: {s}", .{field_name}));
}

/// Parses a single query value string into `FieldType`.
/// Returns `.not_provided` when the value does not parse as that type.
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
                // Strings arrive here. The value references the decoded buffer,
                // which lives on the parse arena, so we can hand it back directly.
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
            if (comptime hasParamParse(FieldType)) {
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
                } else {
                    // Switch on the tag rather than `.get()`:
                    // when `f.type` is itself optional (e.g. `?i32`),
                    // a successfully parsed `null` value would collapse through `.get()`
                    // and be indistinguishable from `.not_provided`, wrongly skipping this variant.
                    switch (_handleQueryParam(f.type, alloc, param)) {
                        .value => |v| return .{ .value = @unionInit(FieldType, f.name, v) },
                        .not_provided => {},
                    }
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
    if (str.len < 1) return error.InvalidArray;

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
    return try list.toOwnedSlice(alloc);
}

/// Returns whether any leaf key of struct `T` is present in the query.
/// This recurses into nested plain structs, which are flattened.
fn anyLeafPresent(comptime T: type, ctx: *ParseCtx) !bool {
    inline for (@typeInfo(T).@"struct".fields) |field| {
        const is_optional = comptime isOptional(field.type);
        const F = if (is_optional) field.type.childType() else field.type;
        if (comptime !is_optional and @typeInfo(F) == .@"union" and !hasParamParse(F)) {
            // A lifted union is present if any of its variant keys are.
            inline for (@typeInfo(F).@"union".fields) |variant| {
                if (try isVariantPresent(variant, ctx)) return true;
            }
        } else if (comptime @typeInfo(F) == .@"struct" and containerKind(F) == .composite) {
            if (try anyLeafPresent(F, ctx)) return true;
        } else if ((try ctx.getParamDecoded(field.name)) != null) {
            return true;
        }
    }
    return false;
}

/// Creates a struct from the query's key/value pairs,
/// recording each consumed query key on `ctx`.
/// Returns null if a failure was recorded on `ctx`.
fn parseFlatStruct(comptime T: type, ctx: *ParseCtx) !?T {
    var result: T = undefined;
    inline for (@typeInfo(T).@"struct".fields) |field| {
        const is_optional = comptime isOptional(field.type);
        const InnerType = if (is_optional) field.type.childType() else field.type;
        const FieldType = Unwrap(InnerType);
        const field_info = @typeInfo(FieldType);

        if (comptime !is_optional and field_info == .@"union" and containerKind(FieldType) == .composite) {
            // Composite unions are key-selected from the variant keys present.
            // Scalar unions fall through to be value-selected against `field.name`,
            // exactly like an `Optional(union)` field.
            const u = (try parseCompositeUnion(FieldType, ctx)) orelse return null;
            @field(result, field.name) = u;
        } else if (comptime field_info == .@"struct" and containerKind(FieldType) == .composite) {
            if (try anyLeafPresent(FieldType, ctx)) {
                const s = (try parseFlatStruct(FieldType, ctx)) orelse return null;
                @field(result, field.name) = if (is_optional) .to(s) else s;
            } else if (is_optional) {
                @field(result, field.name) = .not_provided;
            } else if (field.defaultValue()) |v| {
                @field(result, field.name) = v;
            } else {
                // Required nested struct with no keys present.
                //
                // If it has a required leaf,
                // parseFlatStruct records the specific "Missing query parameter" failure and returns null.
                //
                // If it's entirely optional/has defaults,
                // it succeeds and we assign that all-optional value here.
                const s = (try parseFlatStruct(FieldType, ctx)) orelse return null;
                @field(result, field.name) = s;
            }
        } else {
            if (try ctx.getParamDecoded(field.name)) |param| {
                try ctx.markConsumed(field.name);
                // Parse against `InnerType` so a native `?T` still accepts "null".
                switch (_handleQueryParam(InnerType, ctx.alloc, param.items)) {
                    .value => |v| @field(result, field.name) = if (is_optional) .to(v) else v,
                    .not_provided => {
                        try recordInvalidParamType(ctx, field.name);
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
                ctx.fail(
                    try allocPrint(ctx.alloc, "Missing query parameter: {s}", .{field.name}),
                );
                return null;
            }
        }
    }
    return result;
}

/// Returns whether the keys identifying `variant` are present.
/// A plain struct variant matches on any of its flattened leaf keys.
/// Any other variant matches on its variant name used as a key.
fn isVariantPresent(comptime variant: Type.UnionField, ctx: *ParseCtx) !bool {
    const T = variant.type;
    if (comptime @typeInfo(T) == .@"struct" and containerKind(T) == .composite) {
        return try anyLeafPresent(T, ctx);
    }
    if (comptime @typeInfo(T) == .@"union" and containerKind(T) == .composite) {
        inline for (@typeInfo(T).@"union".fields) |inner| {
            if (try isVariantPresent(inner, ctx)) return true;
        }
        return false;
    }
    return (try ctx.getParamDecoded(variant.name)) != null;
}

/// Builds the `variant` of union `T` from the flat query keys,
/// recording each consumed query key on `ctx`.
/// Returns null if a failure was recorded on `ctx`.
fn buildVariant(comptime T: type, comptime field: Type.UnionField, ctx: *ParseCtx) !?T {
    const V = field.type;
    if (V == void) {
        try ctx.markConsumed(field.name);
        return @unionInit(T, field.name, {});
    } else if (comptime @typeInfo(V) == .@"struct" and containerKind(V) == .composite) {
        const s = try parseFlatStruct(V, ctx) orelse return null;
        if (types.validateConstraints(V, s)) |err_msg| {
            ctx.fail(err_msg);
            return null;
        }
        return @unionInit(T, field.name, s);
    } else if (comptime @typeInfo(V) == .@"union" and containerKind(V) == .composite) {
        const u = try parseCompositeUnion(V, ctx) orelse return null;
        return @unionInit(T, field.name, u);
    } else {
        const param = try ctx.getParamDecoded(field.name) orelse return error.Unreachable;
        switch (_handleQueryParam(V, ctx.alloc, param.items)) {
            .value => |v| {
                try ctx.markConsumed(field.name);
                return @unionInit(T, field.name, v);
            },
            .not_provided => {
                try recordInvalidParamType(ctx, field.name);
                return null;
            },
        }
    }
}

/// Resolves a composite tagged union `T` from the query keys.
///
/// A variant matches when all of its required keys are present,
/// and the most specific match wins.
/// Variants are attempted most specific first (see `orderedVariants`):
/// a variant requiring `{start_date, end_date}` is tried before one requiring only `{start_date}`,
/// so both keys select the former while `start_date` alone selects the latter.
/// This mirrors the generated `XOR` types.
///
/// A single all-optional variant (zero required keys) acts as the fallback and is ordered last,
/// so it is only reached once every specific variant has been tried.
/// Only one fallback may exist, which is enforced at compile time by `findFallbackVariant`.
///
/// If a request supplies keys from more than one variant at once,
/// whichever is attempted first consumes its own keys
/// and the leftover keys are reported as unexpected params by `rejectUnexpectedParams`,
/// so no explicit tie handling is needed.
///
/// Returns null if a failure was recorded on `ctx`.
fn parseCompositeUnion(comptime T: type, ctx: *ParseCtx) !?T {
    var present_keys: std.ArrayList([]const u8) = .empty;
    var it = ctx.paramSlices();
    while (it.next()) |pair| try present_keys.append(ctx.alloc, pair.name);

    const fallback = comptime findFallbackVariant(T);
    const snapshot = ctx.snapshot();

    // The first (most specific) parse failure seen,
    // reported if no variant ends up parsing cleanly.
    var first_failure: ?[]const u8 = null;

    // Attempt variants most specific first; the all-optional fallback is ordered last.
    // A specific variant is tried when its keys are present.
    inline for (comptime orderedFields(T)) |f| {
        const tag = @field(T, f.name);
        const matches = if (fallback == tag)
            first_failure == null
        else
            variantMatchScore(f, present_keys.items) != null;

        if (matches) {
            if (try buildVariant(T, f, ctx)) |v| return v;

            // Keys were present but the values did not parse:
            // keep the failure and roll back, then try the next variant.
            if (first_failure == null) first_failure = ctx.failure;
            ctx.restore(snapshot);
        }
    }

    // Report the most specific parse failure if we had one, otherwise no match.
    if (first_failure) |msg| {
        ctx.fail(msg);
        return null;
    }
    ctx.fail("No matching query parameters were provided");
    return null;
}

/// Records a 400 if there are any unexpected query params.
/// Returns true if a failure was recorded on `ctx`.
fn rejectUnexpectedParams(ctx: *ParseCtx) !bool {
    var unexpected: std.ArrayList([]const u8) = .empty;

    // Find any supplied query params that aren't in `ctx.consumed`.
    var it = std.mem.tokenizeScalar(u8, ctx.query, '&');
    while (it.next()) |token| {
        const name = if (std.mem.indexOfScalar(u8, token, '=')) |eq| token[0..eq] else token;
        if (!containsString(ctx.consumed.items, name)) {
            try unexpected.append(ctx.alloc, name);
        }
    }
    if (unexpected.items.len == 0) return false;

    var msg: std.ArrayList(u8) = .empty;
    try msg.appendSlice(ctx.alloc, "Unexpected query parameters were provided: ");

    for (unexpected.items, 0..) |name, i| {
        if (i != 0) try msg.appendSlice(ctx.alloc, ", ");
        try msg.appendSlice(ctx.alloc, name);
    }

    ctx.fail(msg.items);
    return true;
}

/// Whether every field of struct `QP` can be satisfied by an empty query:
/// (custom `Optional`, a default value, or a composite union field
/// whose union has an all-optional fallback variant).
/// If so, an empty query is acceptable.
fn allFieldsOptional(comptime QP: type) bool {
    inline for (@typeInfo(QP).@"struct".fields) |field| {
        if (isOptional(field.type) or field.defaultValue() != null) continue;
        // A required composite-union field is still satisfiable by an empty query
        // when the union has an all-optional fallback variant.
        const T = field.type;
        if (comptime @typeInfo(T) == .@"union" and containerKind(T) == .composite) {
            if (comptime findFallbackVariant(T) != null) continue;
        }
        return false;
    }
    return true;
}

/// Runs the full parse pipeline for `QP` against `ctx`.
/// Returns the parsed value, or null if a failure was recorded on `ctx`.
fn _parseQuery(comptime QP: type, ctx: *ParseCtx) !?QP {
    // `query_params` as a tagged union is parsed from a flat key/value set,
    // with the active variant inferred from which keys are present.
    // The consumed query keys are stored on `ctx`,
    // so `rejectUnexpectedParams` can see every key that was used.
    if (comptime @typeInfo(QP) == .@"union") {
        const value = (try parseCompositeUnion(QP, ctx)) orelse return null;
        if (try rejectUnexpectedParams(ctx)) return null;
        return value;
    }

    if (comptime !allFieldsOptional(QP)) {
        if (ctx.query.len == 0) {
            ctx.fail("No query params were provided");
            return null;
        }
    }

    const value = (try parseFlatStruct(QP, ctx)) orelse return null;
    if (types.validateConstraints(QP, value)) |err_msg| {
        ctx.fail(err_msg);
        return null;
    }

    if (try rejectUnexpectedParams(ctx)) return null;

    return value;
}

/// Entry point for query param parsing that owns its allocations.
/// Use `parseQueryLeaky` instead when you already have an arena
/// whose lifetime covers the result (e.g. a per-request arena in middleware).
///
/// Parses the raw `query` string into `QP`.
///
/// All parsing allocations go on an internal arena that is returned alongside the result:
/// call `deinit()` on the returned value to free everything at once.
/// The parsed value/failure message are valid until then.
///
/// The result is `.success` with the parsed value,
/// or `.fail` with a message describing why the query was rejected.
///
fn parseQuery(comptime QP: type, alloc: Allocator, query: []const u8) !ParsedQuery(QP) {
    const arena = try alloc.create(std.heap.ArenaAllocator);
    errdefer alloc.destroy(arena);
    arena.* = .init(alloc);
    errdefer arena.deinit();

    const result = try parseQueryLeaky(QP, arena.allocator(), query);
    return .{ .arena = arena, .result = result };
}

/// Same as `parseQuery`, but allocates into `alloc` and frees nothing: `alloc`
/// owns every allocation. `alloc` should be an arena (or similar) whose lifetime
/// covers the parsed value and any failure message. This is what the middleware
/// uses with the per-request arena.
fn parseQueryLeaky(comptime QP: type, alloc: Allocator, query: []const u8) !ParseQueryResult(QP) {
    var ctx = ParseCtx{ .alloc = alloc, .query = query };

    if (try _parseQuery(QP, &ctx)) |value| return .{ .success = value };

    if (ctx.failure) |message| return .{ .fail = message };

    // A null value should always carry a recorded failure; guard defensively.
    return .{ .fail = "Invalid query parameters" };
}

/// Parses the query params of the request and attaches it to the given Context.
/// Context must have a member named after each query param,
/// which resolves to the type meant to be parsed into an object.
///
/// This is a thin adapter over `parseQueryLeaky`:
/// it feeds the request's query string in,
/// then either assigns the parsed value onto the context
/// or translates a failure into `respondWithError`.
///
/// This uses the leaky variant because the per-request arena in `ctx.alloc`
/// already owns the parsed value for the whole request.
/// Parsing must not free the data when the middleware returns.
pub fn parseQueryParams(comptime Context: type, ctx: *const MiddlewareContext(Context)) !void {
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

    comptime assertNoQueryKeyCollisions(@FieldType(Context, query_params));

    const QueryType = @FieldType(Context, query_params);
    switch (try parseQueryLeaky(QueryType, ctx.alloc, ctx.req.query orelse "")) {
        .success => |value| @field(ctx.deps, query_params) = value,
        .fail => |message| try ctx.req.respondWithError(StatusCode.bad_request, message),
    }
}

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
    pub fn paramParse(_: Allocator, _: []const u8) !@This() {
        return null;
    }
};

test "orderedVariants: most specific first, all-optional fallback last" {
    // detailed (2 keys) > basic (1 key) > all (0 keys, the fallback).
    const fields = orderedFields(RangeUnion);
    try std.testing.expectEqualStrings("detailed", fields[0].name);
    try std.testing.expectEqualStrings("basic", fields[1].name);
    try std.testing.expectEqualStrings("all", fields[2].name);
}

test "orderedVariants: orders by required key count, not declaration order" {
    const U = union(enum) {
        // `other` is declared first but requires fewer keys, so it sorts after `nested`.
        other: struct { z: []const u8 },
        nested: struct {
            range: struct {
                start: []const u8,
                end: []const u8,
            },
        },
    };

    const fields = orderedFields(U);
    try std.testing.expectEqualStrings("nested", fields[0].name);
    try std.testing.expectEqualStrings("other", fields[1].name);
}

test "scalar union type" {
    // Scalar types (structs and unions) should not be flattened
    // because they can be constructed from a single query parameter value

    // If all the variant types of a union are scalars, then the union itself is scalar
    const IdOrAuto = union(enum) {
        id: i32,
        auto,
    };

    const Number = union(enum) {
        small: i32, // first, attempt to parse into small integer type
        big: i64, // then try to parse into larger integer type
    };

    const QP = struct {
        site: Optional(IdOrAuto) = .not_provided,
        company: Optional(IdOrAuto) = .not_provided,
        n: Optional(Number) = .not_provided,
    };

    {
        const parsed = try parseQuery(QP, std.testing.allocator, "");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.site == .not_provided);
        try std.testing.expect(result.company == .not_provided);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "site=auto");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.site.value == .auto);
        try std.testing.expect(result.company == .not_provided);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "site=123");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.site.value == .id);
        try std.testing.expectEqual(123, result.site.value.id);
        try std.testing.expect(result.company == .not_provided);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "company=auto");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.site == .not_provided);
        try std.testing.expect(result.company.value == .auto);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "company=123");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.site == .not_provided);
        try std.testing.expect(result.company.value == .id);
        try std.testing.expectEqual(123, result.company.value.id);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "site=123&company=auto");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.site.value == .id);
        try std.testing.expectEqual(123, result.site.value.id);
        try std.testing.expect(result.company.value == .auto);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "site=auto&company=auto");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.site.value == .auto);
        try std.testing.expect(result.company.value == .auto);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "id=123"); // unknown parameter named 'id'
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "auto");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "n=123");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.n.value == .small);
        try std.testing.expectEqual(123, result.n.value.small);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "n=12345678900");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.n.value == .big);
        try std.testing.expectEqual(12345678900, result.n.value.big);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "n=abc");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
}

test "weak scalar union" {
    // A weak scalar union can be interpreted differently based on how it is used.
    const Weak = union(enum) {
        id: i32,
        name: []const u8,
    };

    {
        // Used as scalar
        const QP = struct {
            v: Weak,
        };

        {
            const parsed = try parseQuery(QP, std.testing.allocator, "");
            defer parsed.deinit();
            try std.testing.expect(parsed.result == .fail);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "v=123");
            defer parsed.deinit();
            const result = try parsed.assert();
            try std.testing.expect(result.v == .id);
            try std.testing.expectEqual(123, result.v.id);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "v=abc");
            defer parsed.deinit();
            const result = try parsed.assert();
            try std.testing.expect(result.v == .name);
            try std.testing.expectEqualStrings("abc", result.v.name);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "id=123");
            defer parsed.deinit();
            try std.testing.expect(parsed.result == .fail);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "name=abc");
            defer parsed.deinit();
            try std.testing.expect(parsed.result == .fail);
        }
    }
    {
        // Used as composite
        const QP = Weak;

        {
            const parsed = try parseQuery(QP, std.testing.allocator, "");
            defer parsed.deinit();
            try std.testing.expect(parsed.result == .fail);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "id=123&name=abc");
            defer parsed.deinit();
            try std.testing.expect(parsed.result == .fail);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "id=123");
            defer parsed.deinit();
            const result = try parsed.assert();
            try std.testing.expect(result == .id);
            try std.testing.expectEqual(123, result.id);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "name=abc");
            defer parsed.deinit();
            const result = try parsed.assert();
            try std.testing.expect(result == .name);
            try std.testing.expectEqualStrings("abc", result.name);
        }
    }
    {
        // Weak union inside composite union
        const QP = struct {
            scalar: union(enum) {
                weak: Weak,
                // adding this `strong` variant causes the outer union to be composite
                strong: struct {
                    a: i32,
                    b: []const u8,
                },
            },
            c: ?i32 = null,
        };

        {
            const parsed = try parseQuery(QP, std.testing.allocator, "");
            defer parsed.deinit();
            try std.testing.expect(parsed.result == .fail);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "c=123");
            defer parsed.deinit();
            try std.testing.expect(parsed.result == .fail);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "weak=abc&strong=abc");
            defer parsed.deinit();
            try std.testing.expect(parsed.result == .fail);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "c=123");
            defer parsed.deinit();
            try std.testing.expect(parsed.result == .fail);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "id=123");
            defer parsed.deinit();
            try std.testing.expect(parsed.result == .fail);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "name=abc");
            defer parsed.deinit();
            try std.testing.expect(parsed.result == .fail);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "strong=123");
            defer parsed.deinit();
            try std.testing.expect(parsed.result == .fail);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "strong=abc");
            defer parsed.deinit();
            try std.testing.expect(parsed.result == .fail);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "a=123");
            defer parsed.deinit();
            try std.testing.expect(parsed.result == .fail);
        }
        {
            const parsed = try parseQuery(QP, std.testing.allocator, "a=123&b=abc");
            defer parsed.deinit();
            const result = try parsed.assert();
            try std.testing.expect(result.scalar == .strong);
            try std.testing.expectEqual(123, result.scalar.strong.a);
            try std.testing.expectEqualStrings("abc", result.scalar.strong.b);
        }
    }
}

test "scalar struct" {
    const DateStr = struct {
        year: i32,
        month: i32,
        day: i32,

        pub fn paramParse(_: Allocator, _: []const u8) !@This() {
            return .{ .year = 2026, .month = 7, .day = 6 };
        }
    };

    const QP = struct {
        date: DateStr,
    };

    {
        const parsed = try parseQuery(QP, std.testing.allocator, "date=2026-07-06");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expectEqual(2026, result.date.year);
        try std.testing.expectEqual(7, result.date.month);
        try std.testing.expectEqual(6, result.date.day);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "year=2026&month=7&day=6");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "day=2026");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
}

test "heavy nested scalar struct" {
    const QP = struct {
        one: struct {
            two: struct {
                three: struct {
                    four: struct {
                        a: i32,
                        b: i32,

                        pub fn paramParse(_: Allocator, _: []const u8) !@This() {
                            return .{ .a = 1, .b = 2 };
                        }
                    },
                },
            },
        },
    };

    {
        const parsed = try parseQuery(QP, std.testing.allocator, "");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "one=abc");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "two=abc");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "three=abc");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "four=abc");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expectEqual(result.one.two.three.four.a, 1);
        try std.testing.expectEqual(result.one.two.three.four.b, 2);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "a=1&b=2");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
}

test "matched specific variant parse failure is preferred over fallback" {
    // A union whose specific variant (`date_range`) shares no required key
    // with the all-optional fallback (`all`).
    // When `start_date` is present but its value does not parse,
    // the failure must be the parse error,
    // not an "unexpected params" message from silently falling back to `all` (which ignores `start_date`).
    const QP = union(enum) {
        id: i32,
        dsc_row_id: i32,
        date_range: struct {
            start_date: i32,
            end_date: Optional(i32) = .not_provided,
            line: Optional(i32) = .not_provided,
            shift: Optional(i32) = .not_provided,
        },
        all: struct {
            line: Optional(i32) = .not_provided,
            shift: Optional(i32) = .not_provided,
        },
    };

    {
        // `start_date` is present but fails to parse as a string.
        const parsed = try parseQuery(QP, std.testing.allocator, "start_date=invalid");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
        // The reported failure is the parse error for start_date, not "unexpected params".
        try std.testing.expectEqualStrings(
            parsed.result.fail,
            "Incorrect query parameter for: start_date",
        );
    }
    {
        // A valid date_range still parses.
        const parsed = try parseQuery(QP, std.testing.allocator, "start_date=123");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expectEqual(123, result.date_range.start_date);
    }
    {
        // Fallback-only keys still resolve to `all`.
        const parsed = try parseQuery(QP, std.testing.allocator, "line=1&shift=2");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expectEqual(1, result.all.line.get());
        try std.testing.expectEqual(2, result.all.shift.get());
    }
    {
        // Empty query resolves to the all-optional fallback.
        const parsed = try parseQuery(QP, std.testing.allocator, "");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result == .all);
    }
}

test "foo bar baz" {
    const QP = union(enum) {
        foo: union(enum) {
            bar: struct {
                baz: ?[]const u8 = null,
            },
        },
    };

    {
        const parsed = try parseQuery(QP, std.testing.allocator, "");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expectEqual(null, result.foo.bar.baz);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "baz=abc");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expectEqualStrings("abc", result.foo.bar.baz.?);
    }
}

test "nullable field is required" {
    // only Optional and field with defaults values are truly optional
    const QP = struct {
        required: ?i32,
    };

    {
        const parsed = try parseQuery(QP, std.testing.allocator, "");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "required=123");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expectEqual(123, result.required);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "required=null");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expectEqual(null, result.required);
    }
}

test "nullable require together" {
    const QP = struct {
        n: ?struct {
            a: i32,
            b: i32,
        } = null,
    };

    {
        const parsed = try parseQuery(QP, std.testing.allocator, "");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.n == null);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "a=123");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "b=123");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "a=123&b=456");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.n != null);
        try std.testing.expectEqual(123, result.n.?.a);
        try std.testing.expectEqual(456, result.n.?.b);
    }
}

test "optional require together" {
    const QP = struct {
        n: Optional(struct {
            a: i32,
            b: i32,
        }) = .not_provided,
    };

    {
        const parsed = try parseQuery(QP, std.testing.allocator, "");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.n == .not_provided);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "a=123");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "b=123");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "a=123&b=456");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.n == .value);
        try std.testing.expectEqual(123, result.n.value.a);
        try std.testing.expectEqual(456, result.n.value.b);
    }
}

test "require at least one" {
    const QP = struct {
        a: Optional(i32) = .not_provided,
        b: Optional(i32) = .not_provided,
        c: Optional(i32) = .not_provided,
        d: Optional(i32) = .not_provided,

        pub const constraints: types.Constraints = .{ .any_of = true };
    };

    {
        const parsed = try parseQuery(QP, std.testing.allocator, "");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "a=123");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expectEqual(123, result.a.value);
        try std.testing.expect(result.b == .not_provided);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "a=123&b=456");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expectEqual(123, result.a.value);
        try std.testing.expectEqual(456, result.b.value);
    }
}

test "scalar union with null" {
    const U = union(enum) {
        a: []const u8,
        b: ?i32,
    };

    const W = union(enum) {
        b: ?i32,
        a: []const u8,
    };

    const QP = struct {
        u: U,
        w: W,
    };

    {
        const parsed = try parseQuery(QP, std.testing.allocator, "u=null&w=null");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.u == .a);
        try std.testing.expectEqualStrings("null", result.u.a);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "a=null");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "u=null&w=null");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.w == .b);
        try std.testing.expectEqual(null, result.w.b);
    }
}

test "nested scalar union" {
    // A scalar union whose first variant is itself a scalar union.
    // This is the shape the `.get()` -> tag-switch fix protects:
    // a successfully parsed `null` from the inner `b: ?i32` must survive being bubbled up
    // through the outer union instead of collapsing and being mistaken for "no match".
    const QP = struct {
        outer: union(enum) {
            inner: union(enum) {
                b: ?i32,
                a: []const u8,
            },
            c: bool,
        },
    };

    {
        // "null" -> outer.inner -> inner.b (?i32) parses to a real `null`.
        const parsed = try parseQuery(QP, std.testing.allocator, "outer=null");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.outer == .inner);
        try std.testing.expect(result.outer.inner == .b);
        try std.testing.expectEqual(null, result.outer.inner.b);
    }
    {
        // "123" -> outer.inner -> inner.b (?i32) parses to 123.
        const parsed = try parseQuery(QP, std.testing.allocator, "outer=123");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.outer == .inner);
        try std.testing.expect(result.outer.inner == .b);
        try std.testing.expectEqual(123, result.outer.inner.b);
    }
    {
        // "abc" is not an i32, so inner.b is skipped and inner.a ([]const u8) wins.
        const parsed = try parseQuery(QP, std.testing.allocator, "outer=abc");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.outer == .inner);
        try std.testing.expect(result.outer.inner == .a);
        try std.testing.expectEqualStrings("abc", result.outer.inner.a);
    }
}

test "nested composite union" {
    const QP = union(enum) {
        a: union(enum) {
            c: i32,
            d: []const u8,
        },
        b: union(enum) {
            foo: []const u8,
            bar: struct {
                bar: ?[]const u8 = null,
                baz: ?[]const u8 = null,
            },
        },
    };

    {
        const parsed = try parseQuery(QP, std.testing.allocator, "");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result == .b);
        try std.testing.expect(result.b == .bar);
        try std.testing.expectEqual(null, result.b.bar.bar);
        try std.testing.expectEqual(null, result.b.bar.baz);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "c=123");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "d=abc");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "a=123");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result == .a);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "a=abc");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result == .a);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "b=abc");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "foo=abc");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result == .b);
        try std.testing.expect(result.b == .foo);
        try std.testing.expectEqualStrings("abc", result.b.foo);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "bar=abc");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result == .b);
        try std.testing.expect(result.b == .bar);
        try std.testing.expectEqualStrings("abc", result.b.bar.bar.?);
        try std.testing.expectEqual(null, result.b.bar.baz);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "baz=abc");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result == .b);
        try std.testing.expect(result.b == .bar);
        try std.testing.expectEqual(null, result.b.bar.bar);
        try std.testing.expectEqualStrings("abc", result.b.bar.baz.?);
    }
}

test "scalar union: greedy string variant shadows later variants by order" {
    // A scalar union has no keys to disambiguate variants, only the value string.
    // A `[]const u8` variant always parses, so when it is declared first it shadows every later variant.
    const QP = struct {
        m: union(enum) {
            a: []const u8, // greedy: matches any value
            b: ?i32,
        },
    };

    {
        // Even a valid integer is captured by `a` because it is tried first;
        // `b: ?i32` is never reached.
        const parsed = try parseQuery(QP, std.testing.allocator, "m=123");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.m == .a);
        try std.testing.expectEqualStrings("123", result.m.a);
    }
    {
        // `null` lands on `a` rather than `b: ?i32`.
        const parsed = try parseQuery(QP, std.testing.allocator, "m=null");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.m == .a);
        try std.testing.expectEqualStrings("null", result.m.a);
    }
}

test "non-scalar union with null" {
    const QP = union(enum) {
        a: []const u8,
        b: ?i32,
    };

    {
        const parsed = try parseQuery(QP, std.testing.allocator, "");
        defer parsed.deinit();
        try std.testing.expect(parsed.result == .fail);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "a=null");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result == .a);
        try std.testing.expectEqualStrings("null", result.a);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "a=abc");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result == .a);
        try std.testing.expectEqualStrings("abc", result.a);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "b=null");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result == .b);
        try std.testing.expectEqual(null, result.b);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "b=123");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result == .b);
        try std.testing.expectEqual(123, result.b);
    }
}

test "complex" {
    const DateStr = struct {
        year: i32,
        month: i32,
        day: i32,

        pub fn paramParse(_: Allocator, _: []const u8) !@This() {
            return .{ .year = 2026, .month = 7, .day = 6 };
        }
    };

    const QP = struct {
        filter: union(enum) {
            id: i32,
            name: union(enum) {
                first: []const u8,
                last: []const u8,
                full: struct {
                    first: []const u8,
                    last: []const u8,
                },
            },
            date_range: struct {
                start_date: DateStr,
                end_date: DateStr,
            },
            pagination: struct {
                cursor: ?u64 = null,
                limit: ?u32 = null,
            },
        },
        order_by: ?enum {
            asc,
            desc,
        } = null,
        inactive: Optional(bool) = .not_provided,
    };

    {
        const parsed = try parseQuery(QP, std.testing.allocator, "");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.filter == .pagination);
        try std.testing.expectEqual(null, result.filter.pagination.cursor);
        try std.testing.expectEqual(null, result.filter.pagination.limit);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "id=123");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.filter == .id);
        try std.testing.expectEqual(123, result.filter.id);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "first=abc");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.filter == .name);
        try std.testing.expect(result.filter.name == .first);
        try std.testing.expectEqualStrings("abc", result.filter.name.first);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "last=abc");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.filter == .name);
        try std.testing.expect(result.filter.name == .last);
        try std.testing.expectEqualStrings("abc", result.filter.name.last);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "first=abc&last=def");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.filter == .name);
        try std.testing.expect(result.filter.name == .full);
        try std.testing.expectEqualStrings("abc", result.filter.name.full.first);
        try std.testing.expectEqualStrings("def", result.filter.name.full.last);
    }
    {
        const parsed = try parseQuery(
            QP,
            std.testing.allocator,
            "start_date=2026-07-06&end_date=2026-07-08&order_by=asc",
        );
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.filter == .date_range);
        try std.testing.expectEqual(2026, result.filter.date_range.start_date.year);
        try std.testing.expectEqual(7, result.filter.date_range.end_date.month);
        try std.testing.expect(result.order_by == .asc);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "cursor=999");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.filter == .pagination);
        try std.testing.expectEqual(999, result.filter.pagination.cursor);
        try std.testing.expectEqual(null, result.filter.pagination.limit);
    }
}

test "complex union variant matching" {
    const QP = struct {
        one: union(enum) {
            // foo will fail to parse `end_date=null`, so it should fall "up" to the `bar` variant
            bar: struct {
                start_date: i32, // required
                end_date: ?i32 = null, // optional
            },
            // should attempt to parse into `foo` first because it has more required properties
            foo: struct {
                start_date: i32, // required
                end_date: i32, // required
            },
        },
        two: union(enum) {
            foo: struct {
                b: ?i32, // required
            },
            bar: struct {
                b: []const u8 = "abc", // optional
            },
        },
    };

    {
        const parsed = try parseQuery(QP, std.testing.allocator, "start_date=123");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.one == .bar);
        try std.testing.expectEqual(123, result.one.bar.start_date);
        try std.testing.expectEqual(null, result.one.bar.end_date);
        try std.testing.expect(result.two == .bar);
        try std.testing.expectEqualStrings("abc", result.two.bar.b);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "start_date=123&end_date=123");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.one == .foo);
        try std.testing.expectEqual(123, result.one.foo.start_date);
        try std.testing.expectEqual(123, result.one.foo.end_date);
        try std.testing.expect(result.two == .bar);
        try std.testing.expectEqualStrings("abc", result.two.bar.b);
    }
    {
        const parsed = try parseQuery(QP, std.testing.allocator, "start_date=123&end_date=null");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expect(result.one == .bar);
        try std.testing.expectEqual(123, result.one.bar.start_date);
        try std.testing.expectEqual(null, result.one.bar.end_date);
        try std.testing.expect(result.two == .bar);
        try std.testing.expectEqualStrings("abc", result.two.bar.b);
    }
}

test "optional nullable" {
    const QP = struct {
        shift: Optional(?i32) = .not_provided,
        area: Optional(?i32) = .not_provided,
        line: Optional(?i32) = .not_provided,
    };

    {
        const parsed = try parseQuery(QP, std.testing.allocator, "shift=63&area=61&line=null");
        defer parsed.deinit();
        const result = try parsed.assert();
        try std.testing.expectEqual(63, result.shift.get());
        try std.testing.expectEqual(61, result.area.get());
        try std.testing.expectEqual(null, result.line.get());
    }
}
