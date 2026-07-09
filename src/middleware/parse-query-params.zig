const std = @import("std");
const comptimePrint = std.fmt.comptimePrint;
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
const Unwrap = types.Unwrap;
const Optional = types.Optional;
const isOptional = types.isOptional;

const assertNoQueryKeyCollisions = @import("./query_params/validation.zig").assertNoQueryKeyCollisions;

const query_params = "query_params";

pub fn VariantChoice(comptime T: type) type {
    return union(enum) {
        /// Field index of the unique most specific matching variant
        single: std.meta.Tag(T),
        /// Two or more variants match (a tie between variants)
        multiple,
        /// No specific variant matched, the caller may use the all optional fallback
        none,
    };
}

pub fn Variant(comptime T: type) type {
    return struct {
        tag: std.meta.Tag(T),
        /// Whether all of this variant's required keys are present in the query.
        matched: bool = false,
        /// The variant's number of required keys, used as its specificity score.
        /// A variant requiring more keys is more specific than one requiring a subset.
        required_key_count: usize = 0,
    };
}

/// Chooses the most specific matching variant from `variants` (indexed by union field index).
///
/// Each matched variant is scored by its `required_key_count`.
/// The highest score wins, a tie for the highest score is `.multiple`, and no matches is `.none`.
fn chooseVariant(comptime T: type, variants: []const Variant(T)) VariantChoice(T) {
    var winner: ?std.meta.Tag(T) = null;
    var highest_score: usize = 0;
    var tied = false;
    for (variants) |v| {
        if (!v.matched) continue;
        if (winner == null or v.required_key_count > highest_score) {
            winner = v.tag;
            highest_score = v.required_key_count;
            tied = false;
        } else if (v.required_key_count == highest_score) {
            tied = true;
        }
    }

    if (tied) return .multiple;
    if (winner) |i| return .{ .single = i };
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
    return comptime blk: {
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
        break :blk count;
    };
}

fn getVariantRequiredKeyCount(comptime variant: Type.UnionField) usize {
    const info = @typeInfo(variant.type);
    if (comptime info != .@"struct" or hasParamParse(variant.type)) {
        // Normal union variants are keyed by name, so we only have to return 1.
        return 1;
    }
    return getStructRequiredKeyCount(info.@"struct");
}

/// How a union is resolved from query params.
///
/// - composite: at least one variant carries a nested plain struct with its own leaf keys.
///     Composite unions are key-selected from which variant keys are present (`parseCompositeUnion`),
///     so variant declaration order is irrelevant.
/// - strong_scalar: a scalar union that has a `void` variant.
/// - weak_scalar: a scalar union with no `void` variant.
const UnionKind = enum {
    composite,
    strong_scalar,
    weak_scalar,
};

fn unionKind(comptime U: type) UnionKind {
    const fields = @typeInfo(U).@"union".fields;

    inline for (fields) |variant| {
        if (comptime @typeInfo(variant.type) == .@"struct" and !hasParamParse(variant.type)) {
            return .composite;
        }
    }

    inline for (fields) |variant| {
        if (comptime variant.type == void) return .strong_scalar;
    }
    return .weak_scalar;
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

/// Specificity score of `variant` given the present `keys`,
/// or null if it does not match.
/// A higher score is a more specific match (see `chooseVariant`).
///
/// - A plain struct variant matches when all its required leaf keys are present,
///     scored by its required leaf key count.
/// - A nested composite union variant is scored by its matched inner variant:
///     we recurse and take the most specific inner variant that matches,
///     so an inner `{first, last}` beats an inner `{first}` when both keys are present.
/// - Any other variant matches on its name key, scored as 1.
fn variantMatchScore(comptime variant: Type.UnionField, keys: []const []const u8) ?usize {
    const T = variant.type;

    if (comptime @typeInfo(T) == .@"struct" and !hasParamParse(T)) {
        if (!requiredKeysPresent(T, keys)) return null;
        return comptime getStructRequiredKeyCount(@typeInfo(T).@"struct");
    }

    if (comptime @typeInfo(T) == .@"union" and !hasParamParse(T) and unionKind(T) == .composite) {
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

    if (!keysContain(keys, variant.name)) return null;

    return 1;
}

/// Field index of union `T`'s all-optional fallback variant (zero required keys),
/// or null if it has none.
/// At most one may exist (enforced here at compile time).
fn fallbackVariantIndex(comptime T: type) ?std.meta.Tag(T) {
    comptime {
        var found: ?std.meta.Tag(T) = null;
        for (@typeInfo(T).@"union".fields) |f| {
            if (getVariantRequiredKeyCount(f) == 0) {
                if (found != null) {
                    @compileError("Union " ++ @typeName(T) ++
                        " has more than one all-optional variant;" ++
                        " at most one may act as the fallback.");
                }
                found = @field(T, f.name);
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
/// `.single` always names a variant ready to build, and `.none` means there is nothing to build.
fn selectVariant(comptime T: type, present_keys: []const []const u8) VariantChoice(T) {
    const variants = variants: {
        const fields = @typeInfo(T).@"union".fields;
        var variants = [_]Variant(T){.{ .tag = undefined }} ** fields.len;
        inline for (fields, 0..) |f, i| {
            const score = variantMatchScore(f, present_keys);
            variants[i] = .{
                .tag = @field(T, f.name),
                .matched = if (score) |s| s > 0 else false,
                .required_key_count = score orelse 0,
            };
        }
        break :variants variants;
    };

    const choice = chooseVariant(T, &variants);
    if (choice == .none) {
        if (comptime fallbackVariantIndex(T)) |i| return .{ .single = i };
    }
    return choice;
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

    fn getParamDecoded(self: *const Self, name: []const u8) !?std.ArrayList(u8) {
        return Request.getParamDecodedFromQuery(self.alloc, self.query, name);
    }

    fn paramSlices(self: *const Self) Request.ParamSliceIterator {
        return Request.ParamSliceIterator.init(self.query);
    }
};

/// Records a 400 for a query parameter whose value could not be parsed to `ExpectedType`.
fn recordInvalidParamType(ctx: *ParseCtx, comptime ExpectedType: type, field_name: []const u8) !void {
    ctx.fail(try allocPrint(
        ctx.alloc,
        "Incorrect query parameter type for {s} - Expected {any}",
        .{ field_name, ExpectedType },
    ));
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
        } else if (comptime @typeInfo(F) == .@"struct" and !hasParamParse(F)) {
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

        if (comptime !is_optional and field_info == .@"union" and //
            !hasParamParse(FieldType) and unionKind(FieldType) == .composite)
        {
            // Composite unions are key-selected from the variant keys present.
            // Scalar unions fall through to be value-selected against `field.name`,
            // exactly like an `Optional(union)` field.
            const u = (try parseCompositeUnion(FieldType, ctx)) orelse return null;
            @field(result, field.name) = u;
        } else if (comptime field_info == .@"struct" and !hasParamParse(FieldType)) {
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
                        try recordInvalidParamType(ctx, InnerType, field.name);
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
    if (comptime @typeInfo(T) == .@"struct" and !hasParamParse(T)) {
        return try anyLeafPresent(T, ctx);
    }
    if (comptime @typeInfo(T) == .@"union" and !hasParamParse(T) and unionKind(T) == .composite) {
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
    } else if (comptime @typeInfo(V) == .@"struct" and !hasParamParse(V)) {
        const s = try parseFlatStruct(V, ctx) orelse return null;
        if (types.validateConstraints(V, s)) |err_msg| {
            ctx.fail(err_msg);
            return null;
        }
        return @unionInit(T, field.name, s);
    } else if (comptime @typeInfo(V) == .@"union" and !hasParamParse(V) and unionKind(V) == .composite) {
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
                try recordInvalidParamType(ctx, V, field.name);
                return null;
            },
        }
    }
}

/// Resolves a composite tagged union `T` from the query keys.
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
/// Returns null if a failure was recorded on `ctx`.
fn parseCompositeUnion(comptime T: type, ctx: *ParseCtx) !?T {
    var present_keys: std.ArrayList([]const u8) = .empty;
    var it = ctx.paramSlices();
    while (it.next()) |pair| try present_keys.append(ctx.alloc, pair.name);

    switch (selectVariant(T, present_keys.items)) {
        .single => |variant| {
            const fields = @typeInfo(T).@"union".fields;
            inline for (fields) |f| {
                const V = @field(T, f.name);
                if (V == variant) {
                    return try buildVariant(T, f, ctx);
                }
            }
            // Shouldn't be possible, would be a logic error
            return error.Unreachable;
        },
        .multiple => {
            ctx.fail("Query parameters match more than one variant");
            return null;
        },
        .none => {
            ctx.fail("No matching query parameters were provided");
            return null;
        },
    }
}

/// Records a 400 if there are any unexpected query params.
/// Returns true if a failure was recorded on `ctx`.
fn rejectUnexpectedParams(ctx: *ParseCtx) !bool {
    var unexpected: std.ArrayList([]const u8) = .empty;

    // Find any supplied query params that aren't in `ctx.consumed`.
    var it = std.mem.tokenizeScalar(u8, ctx.query, '&');
    while (it.next()) |token| {
        const name = if (std.mem.indexOfScalar(u8, token, '=')) |eq| token[0..eq] else token;
        if (!keysContain(ctx.consumed.items, name)) {
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
        if (comptime @typeInfo(T) == .@"union" and !hasParamParse(T) and unionKind(T) == .composite) {
            if (comptime fallbackVariantIndex(T) != null) continue;
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

    comptime assertNoQueryKeyCollisions(@FieldType(Context, query_params));

    return struct {
        fn parseQueryParams(ctx: *MiddlewareContext(Context)) anyerror!void {
            const QueryType = @FieldType(Context, query_params);
            switch (try parseQueryLeaky(QueryType, ctx.alloc, ctx.req.query orelse "")) {
                .success => |value| @field(ctx.ctx, query_params) = value,
                .fail => |message| try ctx.req.respondWithError(StatusCode.bad_request, message),
            }
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
    pub fn paramParse(_: Allocator, _: []const u8) !@This() {
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
    try std.testing.expect(choice == .single);
    try std.testing.expectEqual(RangeUnion.basic, choice.single);
}

test "selectVariant: most specific match wins (Detailed over Basic)" {
    const choice = selectVariant(RangeUnion, &.{ "start_date", "end_date" });
    try std.testing.expect(choice == .single);
    try std.testing.expectEqual(RangeUnion.detailed, choice.single);
}

test "selectVariant: empty query selects the all optional fallback" {
    const choice = selectVariant(RangeUnion, &.{});
    try std.testing.expect(choice == .single);
    try std.testing.expectEqual(RangeUnion.all, choice.single);
}

test "selectVariant: fallback only keys select the all optional fallback" {
    const choice = selectVariant(RangeUnion, &.{"page"});
    try std.testing.expect(choice == .single);
    try std.testing.expectEqual(RangeUnion.all, choice.single);
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
    try std.testing.expect(selectVariant(Xor, &.{ "x", "y" }) == .multiple);

    const a = selectVariant(Xor, &.{"x"});
    try std.testing.expect(a == .single);
    try std.testing.expectEqual(Xor.a, a.single);

    const b = selectVariant(Xor, &.{"y"});
    try std.testing.expect(b == .single);
    try std.testing.expectEqual(Xor.b, b.single);
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
    try std.testing.expect(both == .single);
    try std.testing.expectEqual(U.nested, both.single);
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
        try std.testing.expect(choice == .single);
        try std.testing.expectEqual(U.custom, choice.single);
    }

    {
        const choice = selectVariant(U, &.{"flag"});
        try std.testing.expect(choice == .single);
        try std.testing.expectEqual(U.flag, choice.single);
    }

    {
        const choice = selectVariant(U, &.{"z"});
        try std.testing.expect(choice == .single);
        try std.testing.expectEqual(U.other, choice.single);
    }
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

test "scalar struct type" {
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

test "edge cases" {
    {
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
}
