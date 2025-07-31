const std = @import("std");
const pbkdf2 = std.crypto.pwhash.pbkdf2;
const HmacSha256 = std.crypto.auth.hmac.sha2.HmacSha256;

pub const minimum_password_length = 10;
pub const salt_length = 8;
const rounds = 5555;

pub const password_validation_text = "Password must have at least one number, lowercase, uppercase, and special character.";

pub fn generateSalt() [salt_length]u8 {
    var buffer: [salt_length]u8 = undefined;
    std.crypto.random.bytes(&buffer);
    return buffer;
}

/// Hashes a password with a given salt.
/// User owns the returned memory, be sure to free the array.
pub fn hash(alloc: std.mem.Allocator, password: []const u8, salt: []const u8) ![]u8 {
    // Concat password and salt
    var salted_password: []u8 = try alloc.alloc(u8, password.len + salt.len);
    @memcpy(salted_password[0..password.len], password);
    @memcpy(salted_password[password.len..], salt);
    defer alloc.free(salted_password);

    var derived_key: [32]u8 = undefined;
    try pbkdf2(
        &derived_key,
        salted_password,
        salt,
        rounds,
        HmacSha256,
    );

    return alloc.dupe(u8, &derived_key);
}

/// Validates that a password:
/// - Is at least minimum_password_length characters long
/// - Contains a lowercase character
/// - Contains an uppercase character
/// - Contains a number
/// - Contains a special character
pub fn isValidPassword(pass: []const u8) bool {
    if (pass.len < minimum_password_length) return false;

    var has_lowercase: bool = false;
    var has_uppercase: bool = false;
    var has_number: bool = false;
    var has_special_char: bool = false;

    for (pass) |c| {
        switch (c) {
            'a'...'z' => has_lowercase = true,
            'A'...'Z' => has_uppercase = true,
            '0'...'9' => has_number = true,
            else => has_special_char = true,
        }
    }
    return has_lowercase and has_uppercase and has_number and has_special_char;
}

test "correctly hashes a password" {
    const alloc = std.testing.allocator;
    const hashed = try hash(alloc, "password", "j42(6;Kw");
    defer alloc.free(hashed);

    const encoder = std.base64.url_safe.Encoder;
    const encoded_length = encoder.calcSize(hashed.len);
    const encoded_buffer = try alloc.alloc(u8, encoded_length);
    defer alloc.free(encoded_buffer);
    _ = encoder.encode(
        encoded_buffer,
        hashed,
    );
    try std.testing.expectEqualStrings("oEUG8JE7tk6O6aSta1ENGzXa0V6lXuHQK7uVKxtnw_4=", encoded_buffer);
}

test "validates passwords" {
    try std.testing.expect(!isValidPassword("pass"));
    try std.testing.expect(!isValidPassword("password"));
    try std.testing.expect(!isValidPassword("Password!"));
    try std.testing.expect(!isValidPassword("password1"));
    try std.testing.expect(!isValidPassword("password1!"));

    try std.testing.expect(isValidPassword("Password1!"));
    // TODO: Thoughts about spaces?
    try std.testing.expect(isValidPassword(" Password1! "));
}
