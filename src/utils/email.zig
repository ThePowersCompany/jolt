const std = @import("std");

const code_alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

pub fn generateVerificationCode() [32]u8 {
    var code: [32]u8 = undefined;
    for (0..code.len) |i| {
        code[i] = code_alphabet[std.crypto.random.intRangeLessThan(usize, 0, code_alphabet.len)];
    }
    return code;
}
