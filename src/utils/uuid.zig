// NOTE: From https://github.com/karlseguin/zul/blob/master/src/uuid.zig

const std = @import("std");

const fmt = std.fmt;
const crypto = std.crypto;
const Allocator = std.mem.Allocator;

var clock_sequence: u16 = 0;
var last_timestamp: u64 = 0;

pub const UUID = struct {
    bin: [16]u8,

    pub fn seed() void {
        var b: [2]u8 = undefined;
        crypto.random.bytes(&b);
        @atomicStore(u16, *clock_sequence, std.mem.readInt(u16, &b, .big), .monotonic);
    }

    pub fn v4() UUID {
        var bin: [16]u8 = undefined;
        crypto.random.bytes(&bin);
        bin[6] = (bin[6] & 0x0f) | 0x40;
        bin[8] = (bin[8] & 0x3f) | 0x80;
        return .{ .bin = bin };
    }

    pub fn v7() UUID {
        const ts: u64 = @intCast(std.time.milliTimestamp());
        const last = @atomicRmw(u64, &last_timestamp, .Xchg, ts, .monotonic);
        const sequence = if (ts <= last)
            @atomicRmw(u16, &clock_sequence, .Add, 1, .monotonic) + 1
        else
            @atomicLoad(u16, &clock_sequence, .monotonic);

        var bin: [16]u8 = undefined;
        const ts_buf = std.mem.asBytes(&ts);
        bin[0] = ts_buf[5];
        bin[1] = ts_buf[4];
        bin[2] = ts_buf[3];
        bin[3] = ts_buf[2];
        bin[4] = ts_buf[1];
        bin[5] = ts_buf[0];

        const seq_buf = std.mem.asBytes(&sequence);
        // sequence + version
        bin[6] = (seq_buf[1] & 0x0f) | 0x70;
        bin[7] = seq_buf[0];

        crypto.random.bytes(bin[8..]);

        //variant
        bin[8] = (bin[8] & 0x3f) | 0x80;

        return .{ .bin = bin };
    }

    pub fn random() UUID {
        var bin: [16]u8 = undefined;
        crypto.random.bytes(&bin);
        return .{ .bin = bin };
    }

    pub fn parse(hex: []const u8) !UUID {
        var bin: [16]u8 = undefined;

        if (hex.len != 36 or hex[8] != '-' or hex[13] != '-' or hex[18] != '-' or hex[23] != '-') {
            return error.InvalidUUID;
        }

        inline for (encoded_pos, 0..) |i, j| {
            const hi = hex_to_nibble[hex[i + 0]];
            const lo = hex_to_nibble[hex[i + 1]];
            if (hi == 0xff or lo == 0xff) {
                return error.InvalidUUID;
            }
            bin[j] = hi << 4 | lo;
        }
        return .{ .bin = bin };
    }

    pub fn binToHex(bin: []const u8, case: std.fmt.Case) ![36]u8 {
        if (bin.len != 16) {
            return error.InvalidUUID;
        }
        var hex: [36]u8 = undefined;
        b2h(bin, &hex, case);
        return hex;
    }

    pub fn eql(self: UUID, other: UUID) bool {
        inline for (self.bin, other.bin) |a, b| {
            if (a != b) return false;
        }
        return true;
    }

    pub fn toHexAlloc(self: UUID, allocator: std.mem.Allocator, case: std.fmt.Case) ![]u8 {
        const hex = try allocator.alloc(u8, 36);
        _ = self.toHexBuf(hex, case);
        return hex;
    }

    pub fn toHex(self: UUID, case: std.fmt.Case) [36]u8 {
        var hex: [36]u8 = undefined;
        _ = self.toHexBuf(&hex, case);
        return hex;
    }

    pub fn toHexBuf(self: UUID, hex: []u8, case: std.fmt.Case) []u8 {
        std.debug.assert(hex.len >= 36);
        b2h(&self.bin, hex, case);
        return hex[0..36];
    }

    pub fn jsonStringify(self: UUID, out: anytype) !void {
        var hex: [38]u8 = undefined;
        hex[0] = '"';
        _ = self.toHexBuf(hex[1..37], .lower);
        hex[37] = '"';
        try out.print("{s}", .{hex});
    }

    pub fn jsonParse(allocator: std.mem.Allocator, source: anytype, options: std.json.ParseOptions) !UUID {
        const hex = try std.json.innerParse([]const u8, allocator, source, options);
        return UUID.parse(hex) catch error.UnexpectedToken;
    }

    pub fn format(self: UUID, comptime layout: []const u8, options: fmt.FormatOptions, out: anytype) !void {
        _ = options;

        const casing: std.fmt.Case = blk: {
            if (layout.len == 0) break :blk .lower;
            break :blk switch (layout[0]) {
                's', 'x' => .lower,
                'X' => .upper,
                else => @compileError("Unsupported format specifier for UUID: " ++ layout),
            };
        };

        const hex = self.toHex(casing);
        return std.fmt.format(out, "{s}", .{hex});
    }
};

fn b2h(bin: []const u8, hex: []u8, case: std.fmt.Case) void {
    const alphabet = if (case == .lower) "0123456789abcdef" else "0123456789ABCDEF";

    hex[8] = '-';
    hex[13] = '-';
    hex[18] = '-';
    hex[23] = '-';

    inline for (encoded_pos, 0..) |i, j| {
        hex[i + 0] = alphabet[bin[j] >> 4];
        hex[i + 1] = alphabet[bin[j] & 0x0f];
    }
}

const encoded_pos = [16]u8{ 0, 2, 4, 6, 9, 11, 14, 16, 19, 21, 24, 26, 28, 30, 32, 34 };

const hex_to_nibble = [_]u8{0xff} ** 48 ++ [_]u8{
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
    0x08, 0x09, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0xff,
} ++ [_]u8{0xff} ** 152;
