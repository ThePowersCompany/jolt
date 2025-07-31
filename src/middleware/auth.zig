const std = @import("std");
const Allocator = std.mem.Allocator;

const zap = @import("../zap/zap.zig");
const Request = zap.Request;
const StatusCode = zap.StatusCode;
const MiddlewareFn = zap.Endpoint.MiddlewareFn;

const jwtlib = @import("../utils/jwt.zig");

pub const JwtPayload = struct {
    iat: i64,
    id: i32,
    username: []u8,
};

pub const Jwt = struct {
    const Self = @This();
    alloc: Allocator,
    payload: JwtPayload,

    pub fn init(
        alloc: Allocator,
        iat: i64,
        id: i32,
        username: []u8,
    ) !Self {
        return .{
            .alloc = alloc,
            .payload = .{
                .iat = iat,
                .id = id,
                .username = try alloc.dupe(u8, username),
            },
        };
    }

    pub fn deinit(self: *Self) void {
        self.alloc.free(self.payload.username);
    }
};

pub const JwtOrResponse = union(enum) {
    jwt: Jwt,
    response: struct {
        status: StatusCode,
        message: []const u8,
    },
};

pub var jwtkey: []const u8 = undefined;

pub fn createJWT(
    alloc: Allocator,
    user_id: i32,
    username: []u8,
) ![]const u8 {
    const payload = JwtPayload{
        .iat = std.time.timestamp(),
        .id = user_id,
        .username = username,
    };
    return try jwtlib.encode(alloc, .HS256, payload, .{ .key = jwtkey });
}

pub fn extractJwt(comptime Context: type) MiddlewareFn(Context) {
    return struct {
        fn extractJwt(
            context: *Context,
            arena_alloc: Allocator,
            req: Request,
        ) anyerror!void {
            const jwt_or_resp = try parseJWT(arena_alloc, req);
            if (jwt_or_resp == .response) {
                return try req.respondWithError(
                    jwt_or_resp.response.status,
                    jwt_or_resp.response.message,
                );
            }
            context.jwt = jwt_or_resp.jwt.payload;
        }
    }.extractJwt;
}

pub fn parseJWT(alloc: Allocator, r: zap.Request) !JwtOrResponse {
    const maybe_auth_header = r.getHeader("authorization");
    if (maybe_auth_header == null) {
        std.log.warn("Request sent without authorization header\n", .{});
        return JwtOrResponse{
            .response = .{
                .status = StatusCode.unauthorized,
                .message = "Authorization header was not provided",
            },
        };
    }
    const auth_header = maybe_auth_header.?;

    var iter = std.mem.splitSequence(u8, auth_header, "Bearer ");
    _ = iter.next();
    const maybe_token = iter.next();
    if (maybe_token == null or maybe_token.?.len == 0) {
        return JwtOrResponse{
            .response = .{
                .status = StatusCode.bad_request,
                .message = "Malformed JWT",
            },
        };
    }

    const parsed = jwtlib.validate(JwtPayload, alloc, .HS256, maybe_token.?, .{ .key = jwtkey }) catch |err| {
        std.log.warn("User provided invalid JWT: {}\n", .{err});
        return JwtOrResponse{
            .response = .{
                .status = StatusCode.bad_request,
                .message = "Invalid JWT",
            },
        };
    };
    defer parsed.deinit();

    // TODO: check for expired token.
    // If expired, send an error message requesting to log in again?
    const jwt = try Jwt.init(
        alloc,
        parsed.value.iat,
        parsed.value.id,
        parsed.value.username,
    );
    return JwtOrResponse{ .jwt = jwt };
}

pub fn extractWsJwt(comptime Context: type) MiddlewareFn(Context) {
    return struct {
        fn extractWsJwt(
            context: *Context,
            arena_alloc: Allocator,
            req: Request,
        ) anyerror!void {
            const jwt_or_resp = try parseWsJWT(arena_alloc, req);
            if (jwt_or_resp == .response) {
                return try req.respondWithError(
                    jwt_or_resp.response.status,
                    jwt_or_resp.response.message,
                );
            }
            context.jwt = jwt_or_resp.jwt.payload;
        }
    }.extractWsJwt;
}

fn parseWsJWT(alloc: Allocator, r: zap.Request) !JwtOrResponse {
    const maybe_auth_header = r.getHeader("sec-websocket-protocol");
    if (maybe_auth_header == null) {
        std.log.warn("WebSocket opened without authorization header\n", .{});
        return JwtOrResponse{
            .response = .{
                .status = StatusCode.unauthorized,
                .message = "WebSocket authorization header was not provided",
            },
        };
    }
    const maybe_token = maybe_auth_header.?;
    if (maybe_token.len == 0) {
        return JwtOrResponse{
            .response = .{
                .status = StatusCode.bad_request,
                .message = "Malformed JWT",
            },
        };
    }

    const parsed = jwtlib.validate(JwtPayload, alloc, .HS256, maybe_token, .{ .key = jwtkey }) catch |err| {
        std.log.warn("User provided invalid JWT: {}\n", .{err});
        return JwtOrResponse{
            .response = .{
                .status = StatusCode.bad_request,
                .message = "Invalid JWT",
            },
        };
    };
    defer parsed.deinit();

    // TODO: check for expired token.
    // If expired, send an error message requesting to log in again?
    const jwt = try Jwt.init(
        alloc,
        parsed.value.iat,
        parsed.value.id,
        parsed.value.username,
    );

    // Automatically respond with JWT in the Sec-WebSocket-Protocol header
    // WebSocket spec requires servers to respond with selected subprotocol (in this case, the auth token)
    r.setHeader(
        "sec-websocket-protocol",
        maybe_token,
    ) catch {
        return JwtOrResponse{
            .response = .{
                .status = StatusCode.internal_server_error,
                .message = "Failed to select WebSocket subprotocol",
            },
        };
    };

    return JwtOrResponse{ .jwt = jwt };
}
