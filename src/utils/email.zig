const std = @import("std");
const jolt_io = @import("../io.zig");

const code_alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

pub fn generateVerificationCode() [32]u8 {
    var seed: [std.Random.DefaultCsprng.secret_seed_length]u8 = undefined;
    jolt_io.randomBytes(&seed);
    var csprng = std.Random.DefaultCsprng.init(seed);
    const random = csprng.random();

    var code: [32]u8 = undefined;
    for (0..code.len) |i| {
        code[i] = code_alphabet[random.intRangeLessThan(usize, 0, code_alphabet.len)];
    }
    return code;
}
