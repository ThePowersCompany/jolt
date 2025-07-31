const std = @import("std");
const Allocator = std.mem.Allocator;

const http = std.http;

const code_alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

pub fn generateVerificationCode() [32]u8 {
    var code: [32]u8 = undefined;
    for (0..code.len) |i| {
        code[i] = code_alphabet[std.crypto.random.intRangeLessThan(usize, 0, code_alphabet.len)];
    }
    return code;
}

const resend_url = "https://api.resend.com/emails";

pub fn sendEmail(
    alloc: Allocator,
    auth: []const u8,
    from: []const u8,
    to: []const u8,
    subject: []const u8,
    email_content: []const u8,
) !void {
    // Don't send emails if auth is not set.
    if (auth.len == 0) return;

    var client = http.Client{ .allocator = alloc };
    defer client.deinit();

    const payload = try std.json.stringifyAlloc(alloc, .{
        .from = from,
        .to = to,
        .subject = subject,
        .html = email_content,
    }, .{});
    defer alloc.free(payload);

    var response_buffer = std.ArrayList(u8).init(alloc);
    defer response_buffer.deinit();

    const response = try client.fetch(std.http.Client.FetchOptions{
        .method = .POST,
        .location = .{ .url = resend_url },
        .headers = .{
            .content_type = .{ .override = "application/json" },
            .authorization = .{ .override = auth },
        },
        .payload = payload,
        .response_storage = .{ .dynamic = &response_buffer },
    });

    if (response.status != .ok) {
        std.log.err("Failed - Status {}: {s}", .{ response.status, response_buffer.items });
        return error.FailedToSendEmail;
    }
}
