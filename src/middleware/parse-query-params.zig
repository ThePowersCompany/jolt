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
        fn _handleQueryParam(
            ctx: *Context,
            alloc: Allocator,
            req: Request,
            comptime FieldType: type,
            comptime field_name: []const u8,
            param: []const u8,
        ) !bool {
            const T = @typeInfo(FieldType);
            switch (T) {
                .bool => {
                    if (std.mem.eql(u8, param, "true")) {
                        @field(@field(ctx, query_params), field_name) = true;
                    } else if (std.mem.eql(u8, param, "false")) {
                        @field(@field(ctx, query_params), field_name) = false;
                    } else {
                        try sendInvalidParamTypeResponse(alloc, req, FieldType, field_name);
                        return true;
                    }
                },
                .int => {
                    const val = parseInt(FieldType, param, 10) catch {
                        try sendInvalidParamTypeResponse(alloc, req, FieldType, field_name);
                        return true;
                    };
                    @field(@field(ctx, query_params), field_name) = val;
                },
                .float => {
                    const val = parseFloat(FieldType, param) catch {
                        try sendInvalidParamTypeResponse(alloc, req, FieldType, field_name);
                        return true;
                    };
                    @field(@field(ctx, query_params), field_name) = val;
                },
                .pointer => {
                    const ChildT = T.pointer.child;
                    if (ChildT == u8) {
                        // Strings arrive here
                        @field(@field(ctx, query_params), field_name) = param;
                    } else {
                        const value = parseArrayFromString(alloc, ChildT, param) catch {
                            try sendInvalidParamTypeResponse(alloc, req, FieldType, field_name);
                            return true;
                        };
                        @field(@field(ctx, query_params), field_name) = value;
                    }
                },
                else => {
                    try sendInvalidParamTypeResponse(alloc, req, FieldType, field_name);
                    return true;
                },
            }
            return false;
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
                    else => @compileError(std.fmt.comptimePrint("Unsupported query param array child type: {s}", .{@typeName(T)})),
                }
                try list.append(alloc, val);
            }
            return try list.toOwnedSlice(
                alloc,
            );
        }

        /// Returns true if the middleware should exit early.
        fn handleQueryParam(ctx: *Context, alloc: Allocator, req: Request, comptime field: Type.StructField) !bool {
            const FieldType = @typeInfo(field.type);
            const param_opt = try req.getParamDecoded(alloc, field.name);
            if (param_opt) |param| {
                const T = if (FieldType == .optional) FieldType.optional.child else field.type;
                return try _handleQueryParam(ctx, alloc, req, T, field.name, param.items);
            } else if (field.defaultValue()) |default_value| {
                // Param is missing, but has a default value. Set to the default value.
                @field(@field(ctx, query_params), field.name) = default_value;
            } else if (FieldType == .optional) {
                // Param is missing, but optional. Set it to null
                @field(@field(ctx, query_params), field.name) = null;
            } else {
                try req.respondWithError(
                    StatusCode.bad_request,
                    try allocPrint(
                        alloc,
                        "Missing query parameter: {s}",
                        .{field.name},
                    ),
                );
                return true;
            }
            return false;
        }

        fn parseQueryParams(ctx: *MiddlewareContext(Context)) anyerror!void {
            var all_fields_are_optional = true;
            outer: inline for (@typeInfo(Context).@"struct".fields) |ctx_field| {
                if (comptime std.mem.eql(u8, ctx_field.name, query_params)) {
                    inline for (@typeInfo(ctx_field.type).@"struct".fields) |field| {
                        if (@typeInfo(field.type) != .optional and field.defaultValue() == null) {
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

            outer: inline for (@typeInfo(Context).@"struct".fields) |ctx_field| {
                if (comptime std.mem.eql(u8, ctx_field.name, query_params)) {
                    inline for (@typeInfo(ctx_field.type).@"struct".fields) |field| {
                        if (try handleQueryParam(ctx.ctx, ctx.alloc, ctx.req, field)) {
                            break :outer;
                        }
                    }
                }
            }
        }
    }.parseQueryParams;
}
