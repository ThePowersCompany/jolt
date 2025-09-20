const std = @import("std");
const Allocator = std.mem.Allocator;
const Value = std.json.Value;
const base64url = std.base64.url_safe_no_pad;

const stringify = @import("json.zig").stringify;

const Algorithm = enum {
    const Self = @This();

    HS256,
    HS384,
    HS512,

    pub fn jsonStringify(
        value: Self,
        options: std.json.Stringify.Options,
        writer: anytype,
    ) @TypeOf(writer).Error!void {
        try std.json.stringify(@tagName(value), options, writer);
    }

    pub fn CryptoFn(comptime self: Self) type {
        return switch (self) {
            .HS256 => std.crypto.auth.hmac.sha2.HmacSha256,
            .HS384 => std.crypto.auth.hmac.sha2.HmacSha384,
            .HS512 => std.crypto.auth.hmac.sha2.HmacSha512,
        };
    }
};

const JWTType = enum {
    JWS,
    JWE,
};

pub const SignatureOptions = struct {
    key: []const u8,
    kid: ?[]const u8 = null,
};

pub fn encode(
    alloc: Allocator,
    comptime alg: Algorithm,
    payload: anytype,
    signature_options: SignatureOptions,
) ![]const u8 {
    const json = try stringify(alloc, payload, .{});
    defer alloc.free(json);
    return try encodeMessage(alloc, alg, json, signature_options);
}

pub fn encodeMessage(
    allocator: std.mem.Allocator,
    comptime alg: Algorithm,
    message: []const u8,
    signature_options: SignatureOptions,
) ![]const u8 {
    var protected_header = std.json.ObjectMap.init(allocator);
    defer protected_header.deinit();
    try protected_header.put("alg", .{ .string = @tagName(alg) });
    try protected_header.put("typ", .{ .string = "JWT" });
    if (signature_options
        .kid) |kid|
    {
        try protected_header.put("kid", .{ .string = kid });
    }

    var protected_header_json = std.Io.Writer.Allocating.init(allocator);
    defer protected_header_json.deinit();

    var s = std.json.Stringify{
        .writer = &protected_header_json.writer,
        .options = .{},
    };
    try s.write(Value{ .object = protected_header });

    const message_base64_len = base64url.Encoder.calcSize(message.len);
    const protected_header_base64_len = base64url.Encoder.calcSize(protected_header_json.written().len);

    var jwt_text = std.Io.Writer.Allocating.init(allocator);
    defer jwt_text.deinit();
    try jwt_text.ensureTotalCapacity(message_base64_len + 1 + protected_header_base64_len);

    const signature = blk: {
        const protected_header_base64 = jwt_text.writer.buffer[0..protected_header_base64_len];
        const message_base64 = jwt_text.writer.buffer[protected_header_base64_len + 1 ..][0..message_base64_len];

        _ = base64url.Encoder.encode(protected_header_base64, protected_header_json.written());
        jwt_text.writer.buffer[protected_header_base64_len] = '.';
        _ = base64url.Encoder.encode(message_base64, message);
        jwt_text.writer.end = protected_header_base64_len + 1 + message_base64_len;

        break :blk generate_signature(alg, signature_options
            .key, protected_header_base64, message_base64);
    };
    const signature_base64_len = base64url.Encoder.calcSize(signature.len);

    try jwt_text.ensureTotalCapacity(message_base64_len + 1 + protected_header_base64_len + 1 + signature_base64_len);
    const signature_base64 = jwt_text.writer.buffer[message_base64_len + 1 + protected_header_base64_len + 1 ..][0..signature_base64_len];

    jwt_text.writer.buffer[message_base64_len + 1 + protected_header_base64_len] = '.';
    _ = base64url.Encoder.encode(signature_base64, &signature);
    jwt_text.writer.end += 1 + signature_base64_len;

    return jwt_text.toOwnedSlice();
}

pub fn validate(
    comptime P: type,
    alloc: Allocator,
    comptime alg: Algorithm,
    token_text: []const u8,
    signature_options: SignatureOptions,
) !std.json.Parsed(P) {
    const message = try validateMessage(alloc, alg, token_text, signature_options);
    defer alloc.free(message);

    // 10.  Verify that the resulting octet sequence is a UTF-8-encoded
    //      representation of a completely valid JSON object conforming to
    //      RFC 7159 [RFC7159]; let the JWT Claims Set be this JSON object.
    return std.json.parseFromSlice(P, alloc, message, .{ .allocate = .alloc_always, .ignore_unknown_fields = true });
}

pub fn validateMessage(
    alloc: Allocator,
    comptime expected_alg: Algorithm,
    token_text: []const u8,
    signature_options: SignatureOptions,
) ![]const u8 {
    // 1.   Verify that the JWT contains at least one period ('.')
    //      character.
    // 2.   Let the Encoded JOSE Header be the portion of the JWT before the
    //      first period ('.') character.
    const end_of_jose_base64 = std.mem.indexOfScalar(u8, token_text, '.') orelse return error.InvalidFormat;
    const jose_base64 = token_text[0..end_of_jose_base64];

    // 3.   Base64url decode the Encoded JOSE Header following the
    //      restriction that no line breaks, whitespace, or other additional
    //      characters have been used.
    const jose_json = try alloc.alloc(u8, try base64url.Decoder.calcSizeForSlice(jose_base64));
    defer alloc.free(jose_json);
    try base64url.Decoder.decode(jose_json, jose_base64);

    // 4.   Verify that the resulting octet sequence is a UTF-8-encoded
    //      representation of a completely valid JSON object conforming to
    //      RFC 7159 [RFC7159]; let the JOSE Header be this JSON object.

    // TODO: Make sure the JSON parser confirms everything above

    const cty_opt = @as(?[]const u8, null);
    defer if (cty_opt) |cty| alloc.free(cty);

    var jwt_tree = try std.json.parseFromSlice(std.json.Value, alloc, jose_json, .{});
    defer jwt_tree.deinit();

    // 5.   Verify that the resulting JOSE Header includes only parameters
    //      and values whose syntax and semantics are both understood and
    //      supported or that are specified as being ignored when not
    //      understood.

    var jwt_root = jwt_tree.value;
    if (jwt_root != .object) return error.InvalidFormat;

    {
        const alg_val = jwt_root.object.get("alg") orelse return error.InvalidFormat;
        if (alg_val != .string) return error.InvalidFormat;
        const alg = std.meta.stringToEnum(Algorithm, alg_val.string) orelse return error.InvalidAlgorithm;

        // Make sure that the algorithm matches: https://auth0.com/blog/critical-vulnerabilities-in-json-web-token-libraries/
        if (alg != expected_alg) return error.InvalidAlgorithm;

        // TODO: Determine if "jku"/"jwk" need to be parsed and validated

        if (jwt_root.object.get("crit")) |crit_val| {
            if (crit_val != .array) return error.InvalidFormat;
            const crit = crit_val.array;
            if (crit.items.len == 0) return error.InvalidFormat;

            // TODO: Implement or allow extensions?
            return error.UnknownExtension;
        }
    }

    // 6.   Determine whether the JWT is a JWS or a JWE using any of the
    //      methods described in Section 9 of [JWE].

    const jwt_type = determine_jwt_type: {
        // From Section 9 of the JWE specification:
        // > o  If the object is using the JWS Compact Serialization or the JWE
        // >    Compact Serialization, the number of base64url-encoded segments
        // >    separated by period ('.') characters differs for JWSs and JWEs.
        // >    JWSs have three segments separated by two period ('.') characters.
        // >    JWEs have five segments separated by four period ('.') characters.
        switch (std.mem.count(u8, token_text, ".")) {
            2 => break :determine_jwt_type JWTType.JWS,
            4 => break :determine_jwt_type JWTType.JWE,
            else => return error.InvalidFormat,
        }
    };

    // 7.   Depending upon whether the JWT is a JWS or JWE, there are two
    //      cases:
    const message_base64 = get_message: {
        switch (jwt_type) {
            // If the JWT is a JWS, follow the steps specified in [JWS] for
            // validating a JWS.  Let the Message be the result of base64url
            // decoding the JWS Payload.
            .JWS => {
                var section_iter = std.mem.splitSequence(u8, token_text, ".");
                std.debug.assert(section_iter.next() != null);
                const payload_base64 = section_iter.next().?;
                const signature_base64 = section_iter.rest();

                const signature = try alloc.alloc(u8, try base64url.Decoder.calcSizeForSlice(signature_base64));
                defer alloc.free(signature);
                try base64url.Decoder.decode(signature, signature_base64);

                const gen_sig = &generate_signature(expected_alg, signature_options.key, jose_base64, payload_base64);
                if (!std.mem.eql(u8, signature, gen_sig)) {
                    return error.InvalidSignature;
                }

                break :get_message try alloc.dupe(u8, payload_base64);
            },
            .JWE => {
                // Else, if the JWT is a JWE, follow the steps specified in
                // [JWE] for validating a JWE.  Let the Message be the resulting
                // plaintext.
                return error.Unimplemented;
            },
        }
    };
    defer alloc.free(message_base64);

    // 8.   If the JOSE Header contains a "cty" (content type) value of
    //      "JWT", then the Message is a JWT that was the subject of nested
    //      signing or encryption operations.  In this case, return to Step
    //      1, using the Message as the JWT.
    if (jwt_root.object.get("cty")) |cty_val| {
        if (cty_val != .string) return error.InvalidFormat;
        return error.Unimplemented;
    }

    // 9.   Otherwise, base64url decode the Message following the
    //      restriction that no line breaks, whitespace, or other additional
    //      characters have been used.
    const message = try alloc.alloc(u8, try base64url.Decoder.calcSizeForSlice(message_base64));
    errdefer alloc.free(message);
    try base64url.Decoder.decode(message, message_base64);

    return message;
}

pub fn generate_signature(
    comptime algo: Algorithm,
    key: []const u8,
    protected_header_base64: []const u8,
    payload_base64: []const u8,
) [algo.CryptoFn().mac_length]u8 {
    const T = algo.CryptoFn();
    var h = T.init(key);
    h.update(protected_header_base64);
    h.update(".");
    h.update(payload_base64);

    var out: [T.mac_length]u8 = undefined;
    h.final(&out);

    return out;
}

test "generate jws based tokens" {
    const payload = TestPayload{
        .sub = "1234567890",
        .name = "John Doe",
        .iat = 1516239022,
    };

    try test_generate(
        .HS256,
        payload,
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SVT7VUK8eOve-SCacPaU_bkzT3SFr9wk5EQciofG4Qo",
        "AyM1SysPpbyDfgZld3umj1qzKObwVMkoqQ-EstJQLr_T-1qS0gZH75aKtMN3Yj0iPS4hcgUuTwjAzZr1Z9CAow",
    );
    try test_generate(
        .HS384,
        payload,
        "eyJhbGciOiJIUzM4NCIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.MSnfJgb61edr7STbvEqi4Mj3Vvmb8Kh3lsnlXacv0cDAGYhBOpNmOrhWwQgTJCKj",
        "AyM1SysPpbyDfgZld3umj1qzKObwVMkoqQ-EstJQLr_T-1qS0gZH75aKtMN3Yj0iPS4hcgUuTwjAzZr1Z9CAow",
    );
    try test_generate(
        .HS512,
        payload,
        "eyJhbGciOiJIUzUxMiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.39Xvky4dIVLaVaOW5BgbO7smTZUyvIcRtBE3i2hVW3GbjSeUFmpwRbMy94CfvgHC3KHT6V4-pnkNTotCWer-cw",
        "AyM1SysPpbyDfgZld3umj1qzKObwVMkoqQ-EstJQLr_T-1qS0gZH75aKtMN3Yj0iPS4hcgUuTwjAzZr1Z9CAow",
    );
}

test "validate jws based tokens" {
    const expected = TestValidatePayload{
        .iss = "joe",
        .exp = 1300819380,
        .@"http://example.com/is_root" = true,
    };

    try test_validate(
        .HS256,
        expected,
        "eyJ0eXAiOiJKV1QiLA0KICJhbGciOiJIUzI1NiJ9.eyJpc3MiOiJqb2UiLA0KICJleHAiOjEzMDA4MTkzODAsDQogImh0dHA6Ly9leGFtcGxlLmNvbS9pc19yb290Ijp0cnVlfQ.dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
        "AyM1SysPpbyDfgZld3umj1qzKObwVMkoqQ-EstJQLr_T-1qS0gZH75aKtMN3Yj0iPS4hcgUuTwjAzZr1Z9CAow",
    );
    try test_validate(
        .HS384,
        expected,
        "eyJhbGciOiJIUzM4NCIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJqb2UiLCJleHAiOjEzMDA4MTkzODAsImh0dHA6Ly9leGFtcGxlLmNvbS9pc19yb290Ijp0cnVlfQ.2B5ucfIDtuSVRisXjPwZlqPAwgEicFIX7Gd2r8rlAbLukenHTW0Rbx1ca1VJSyLg",
        "AyM1SysPpbyDfgZld3umj1qzKObwVMkoqQ-EstJQLr_T-1qS0gZH75aKtMN3Yj0iPS4hcgUuTwjAzZr1Z9CAow",
    );
    try test_validate(
        .HS512,
        expected,
        "eyJhbGciOiJIUzUxMiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJqb2UiLCJleHAiOjEzMDA4MTkzODAsImh0dHA6Ly9leGFtcGxlLmNvbS9pc19yb290Ijp0cnVlfQ.TrGchM_jCqCTAYUQlFmXt-KOyKO0O2wYYW5fUSV8jtdgqWJ74cqNA1zc9Ix7TU4qJ-Y32rKmP9Xpu99yiShx6g",
        "AyM1SysPpbyDfgZld3umj1qzKObwVMkoqQ-EstJQLr_T-1qS0gZH75aKtMN3Yj0iPS4hcgUuTwjAzZr1Z9CAow",
    );
}

test "generate and then validate jws token" {
    try test_generate_then_validate(.HS256, .{ .key = "a jws hmac sha-256 test key" });
    try test_generate_then_validate(.HS384, .{ .key = "a jws hmac sha-384 test key" });
}

const TestPayload = struct {
    sub: []const u8,
    name: []const u8,
    iat: i64,
};

fn test_generate(
    comptime algorithm: Algorithm,
    payload: TestPayload,
    expected: []const u8,
    key_base64: []const u8,
) !void {
    const alloc = std.testing.allocator;
    const key = try alloc.alloc(u8, try base64url.Decoder.calcSizeForSlice(key_base64));
    defer alloc.free(key);
    try base64url.Decoder.decode(key, key_base64);

    const token = try encode(alloc, algorithm, payload, .{ .key = key });
    defer alloc.free(token);

    try std.testing.expectEqualSlices(u8, expected, token);
}

const TestValidatePayload = struct {
    iss: []const u8,
    exp: i64,
    @"http://example.com/is_root": bool,
};

fn test_validate(
    comptime algorithm: Algorithm,
    expected: TestValidatePayload,
    token: []const u8,
    key_base64: []const u8,
) !void {
    const alloc = std.testing.allocator;
    const key = try alloc.alloc(u8, try base64url.Decoder.calcSizeForSlice(key_base64));
    defer alloc.free(key);
    try base64url.Decoder.decode(key, key_base64);

    var claims_p = try validate(TestValidatePayload, alloc, algorithm, token, .{ .key = key });
    defer claims_p.deinit();
    const claims = claims_p.value;

    try std.testing.expectEqualSlices(u8, expected.iss, claims.iss);
    try std.testing.expectEqual(expected.exp, claims.exp);
    try std.testing.expectEqual(expected.@"http://example.com/is_root", claims.@"http://example.com/is_root");
}

fn test_generate_then_validate(comptime alg: Algorithm, signature_options: SignatureOptions) !void {
    const Payload = struct {
        sub: []const u8,
        name: []const u8,
        iat: i64,
    };
    const payload = Payload{
        .sub = "1234567890",
        .name = "John Doe",
        .iat = 1516239022,
    };

    const alloc = std.testing.allocator;
    const token = try encode(alloc, alg, payload, signature_options);
    defer alloc.free(token);

    var decoded_p = try validate(Payload, alloc, alg, token, signature_options);
    defer decoded_p.deinit();
    const decoded = decoded_p.value;

    try std.testing.expectEqualSlices(u8, payload.sub, decoded.sub);
    try std.testing.expectEqualSlices(u8, payload.name, decoded.name);
    try std.testing.expectEqual(payload.iat, decoded.iat);
}
