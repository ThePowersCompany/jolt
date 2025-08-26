const std = @import("std");
const json = std.json;
const Allocator = std.mem.Allocator;
const zap = @import("../zap/zap.zig");
const MiddlewareFn = zap.Endpoint.MiddlewareFn;
const Request = zap.Request;
const HttpError = zap.HttpError;
const StatusCode = zap.StatusCode;

/// Various ways to represent tagged unions in JSON.
/// Reference (Rust): https://serde.rs/enum-representations.html
pub const UnionRepr = union(enum) {
    external,
    internal: struct {
        discriminator: []const u8,
    },
    adjacently: struct {
        discriminator: []const u8,
    },
    untagged,
};

/// Parses the body of the request and attaches it to the given Context.
/// Context must have a member named "body" which resolves to the type meant to be parsed into an object.
///
/// Note: Enable optional fields by setting default value in struct.
pub fn parseBody(comptime Context: type) MiddlewareFn(Context) {
    if (!@hasField(Context, "body")) {
        @compileError("\"body\" property was not provided for parse body middleware.");
    }

    const body_type_info = @typeInfo(@FieldType(Context, "body"));
    switch (body_type_info) {
        .@"struct", .@"union" => {
            return struct {
                fn parseBody(
                    context: *Context,
                    arena_alloc: Allocator,
                    req: Request,
                ) !void {
                    if (req.body) |body| {
                        const parsed_body = json.parseFromSliceLeaky(
                            @TypeOf(context.body),
                            arena_alloc,
                            body,
                            .{},
                        ) catch |err| {
                            std.log.info("Invalid body sent: {}\n", .{err});
                            return try req.respondWithError(
                                StatusCode.bad_request,
                                "Unexpected body structure",
                            );
                        };
                        context.body = parsed_body;
                    } else {
                        try req.respondWithError(
                            StatusCode.bad_request,
                            "Body was not provided",
                        );
                    }
                }
            }.parseBody;
        },
        .pointer => {
            if (body_type_info.pointer.child != u8) {
                @compileError("Body was a pointer but not a string");
            }
            return struct {
                fn parseBody(
                    context: *Context,
                    _: Allocator,
                    req: Request,
                ) !void {
                    if (req.body) |body| {
                        context.body = body;
                    } else {
                        try req.respondWithError(
                            StatusCode.bad_request,
                            "Body was not provided",
                        );
                    }
                }
            }.parseBody;
        },
        else => {
            const err = std.fmt.comptimePrint("Unsupported body type: {}", .{body_type_info});
            @compileError(err);
        },
    }
}
