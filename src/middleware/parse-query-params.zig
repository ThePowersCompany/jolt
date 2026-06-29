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
                    if (hasFn(FieldType, "paramParse")) {
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
                const Child = if (is_optional) field.type.childType() else field.type;
                if (comptime !is_optional and @typeInfo(Child) == .@"union" and !hasFn(Child, "paramParse")) {
                    // A lifted union is present if any of its variant keys are.
                    inline for (@typeInfo(Child).@"union".fields) |variant| {
                        if (try variantPresent(variant, alloc, req)) return true;
                    }
                } else if (comptime @typeInfo(Child) == .@"struct" and !hasFn(Child, "paramParse")) {
                    if (try anyLeafPresent(Child, alloc, req)) return true;
                } else if ((try req.getParamDecoded(alloc, field.name)) != null) {
                    return true;
                }
            }
            return false;
        }

        /// Creates a struct from the query's key/value pairs.
        /// Returns null if an error response was sent.
        fn parseFlatStruct(comptime T: type, alloc: Allocator, req: Request) !?T {
            var result: T = undefined;
            inline for (@typeInfo(T).@"struct".fields) |field| {
                const is_optional = comptime isOptional(field.type);
                const Child = if (is_optional) field.type.childType() else field.type;
                const child_info = @typeInfo(Child);

                if (comptime !is_optional and child_info == .@"union" and !hasFn(Child, "paramParse")) {
                    @field(result, field.name) = (try parseFlatUnionValue(Child, alloc, req)) orelse return null;
                } else if (comptime child_info == .@"struct" and !hasFn(Child, "paramParse")) {
                    if (try anyLeafPresent(Child, alloc, req)) {
                        const nested = (try parseFlatStruct(Child, alloc, req)) orelse return null;
                        @field(result, field.name) = if (is_optional) .to(nested) else nested;
                    } else if (is_optional) {
                        @field(result, field.name) = .not_provided;
                    } else if (field.defaultValue()) |v| {
                        @field(result, field.name) = v;
                    } else {
                        // Required nested struct with no keys present.
                        // Parse it anyway so the specific missing leaf error is propagated.
                        @field(result, field.name) = (try parseFlatStruct(Child, alloc, req)) orelse return null;
                    }
                } else {
                    const param_opt = try req.getParamDecoded(alloc, field.name);
                    if (param_opt) |param| {
                        switch (_handleQueryParam(Child, alloc, param.items)) {
                            .value => |v| @field(result, field.name) = if (is_optional) .to(v) else v,
                            .not_provided => {
                                try sendInvalidParamTypeResponse(alloc, req, Child, field.name);
                                return null;
                            },
                        }
                    } else if (field.defaultValue()) |v| {
                        @field(result, field.name) = v;
                    } else if (is_optional) {
                        @field(result, field.name) = .not_provided;
                    } else if (child_info == .optional) {
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
            return result;
        }

        /// Returns whether the keys identifying `variant` are present.
        /// A plain struct variant matches on any of its flattened leaf keys.
        /// Any other variant matches on its variant name used as a key.
        fn variantPresent(comptime variant: Type.UnionField, alloc: Allocator, req: Request) !bool {
            const T = variant.type;
            if (comptime @typeInfo(T) == .@"struct" and !hasFn(T, "paramParse")) {
                return try anyLeafPresent(T, alloc, req);
            }
            return (try req.getParamDecoded(alloc, variant.name)) != null;
        }

        /// Resolves a tagged union `T` from flat query keys.
        /// The active variant is inferred from which keys are present.
        /// Returns null if an error response was sent.
        fn parseFlatUnionValue(comptime T: type, alloc: Allocator, req: Request) !?T {
            const fields = @typeInfo(T).@"union".fields;
            var present = [_]bool{false} ** fields.len;

            inline for (fields, 0..) |variant, i| {
                present[i] = try variantPresent(variant, alloc, req);
            }

            var match_count: usize = 0;
            for (present) |p| {
                if (p) match_count += 1;
            }

            if (match_count == 0) {
                try req.respondWithError(StatusCode.bad_request, "No matching query parameters were provided.");
                return null;
            }

            if (match_count > 1) {
                try req.respondWithError(StatusCode.bad_request, "Query parameters match more than one variant.");
                return null;
            }

            inline for (fields, 0..) |variant, i| {
                if (!present[i]) continue;

                const V = variant.type;
                if (V == void) {
                    return @unionInit(T, variant.name, {});
                } else if (comptime @typeInfo(V) == .@"struct" and !hasFn(V, "paramParse")) {
                    const value = (try parseFlatStruct(V, alloc, req)) orelse return null;
                    if (types.validateConstraints(V, value)) |err_msg| {
                        try req.respondWithError(StatusCode.bad_request, err_msg);
                        return null;
                    }
                    return @unionInit(T, variant.name, value);
                } else {
                    const param = (try req.getParamDecoded(alloc, variant.name)).?;
                    switch (_handleQueryParam(V, alloc, param.items)) {
                        .value => |v| return @unionInit(T, variant.name, v),
                        .not_provided => {
                            try sendInvalidParamTypeResponse(alloc, req, V, variant.name);
                            return null;
                        },
                    }
                }
            }
            return error.Unreachable;
        }

        /// Parses a tagged-union `query_params` and attaches it to the context.
        fn parseUnionQueryParams(comptime T: type, ctx: *Context, alloc: Allocator, req: Request) !void {
            if (try parseFlatUnionValue(T, alloc, req)) |value| {
                @field(ctx, query_params) = value;
            }
        }

        fn parseQueryParams(ctx: *MiddlewareContext(Context)) anyerror!void {
            const QueryType = @FieldType(Context, query_params);

            // `query_params` as a tagged union is parsed from a flat key/value set,
            // with the active variant inferred from which keys are present.
            // `Optional` is also a union, so we have to exclude it.
            if (comptime @typeInfo(QueryType) == .@"union" and !isOptional(QueryType)) {
                try parseUnionQueryParams(QueryType, ctx.ctx, ctx.alloc, ctx.req);
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

            const value = (try parseFlatStruct(QueryType, ctx.alloc, ctx.req)) orelse return;
            @field(ctx.ctx, query_params) = value;

            if (types.validateConstraints(QueryType, value)) |err_msg| {
                return try ctx.req.respondWithError(StatusCode.bad_request, err_msg);
            }
        }
    }.parseQueryParams;
}
