const std = @import("std");
const ArenaAllocator = std.heap.ArenaAllocator;
const Allocator = std.mem.Allocator;
const ArrayList = std.ArrayList;
const EndpointDef = @import("main.zig").EndpointDef;
const TypeGenerator = @import("typegen/generator.zig").TypeGenerator;

/// Private TS utility types used by the generated definitions below.
const PrivateUtilityTypes =
    \\type SetRequired<T, K extends keyof T> = Omit<T, K> & Required<Pick<T, K>>;
    \\
    \\type AnyOf<T, K extends keyof T = keyof T> = {
    \\  [P in K]: SetRequired<T, P>;
    \\}[K];
    \\
    \\type Without<T, U> = { [P in Exclude<keyof T, keyof U>]?: never };
    \\
    \\type XOR<T, U> = (T | U) extends object ? (Without<T, U> & U) | (Without<U, T> & T) : T | U;
;

pub fn generateTypesFile(
    alloc: Allocator,
    ts_file_name: []const u8,
    endpoints: []const EndpointDef,
) !void {
    var arena = ArenaAllocator.init(alloc);
    defer arena.deinit();
    const arena_alloc = arena.allocator();

    var ts: ArrayList(u8) = .empty;
    defer ts.deinit(alloc);

    try ts.appendSlice(alloc,
        \\ // === DO NOT MODIFY ===
        \\ //
        \\ // Auto-generated type definitions
        \\ //
        \\ // === DO NOT MODIFY ===
        \\
        \\
    );

    // Emit the private TS utility types used by the generated definitions below.
    try ts.appendSlice(alloc, PrivateUtilityTypes);
    try ts.appendSlice(alloc, "\n\n");

    var type_generator = try TypeGenerator.init(arena_alloc);
    defer type_generator.deinit();

    try ts.appendSlice(alloc, try type_generator.generateTypes(endpoints));

    const file = try std.fs.cwd().createFile(ts_file_name, .{ .read = true });
    defer file.close();
    try file.writeAll(ts.items);

    try formatWithPrettier(arena_alloc, ts_file_name);
}

/// Uses prettier to format the given TS file.
fn formatWithPrettier(alloc: Allocator, file_name: []const u8) !void {
    const result = try std.process.Child.run(.{
        .allocator = alloc,
        .argv = &[_][]const u8{
            "npx",
            "prettier",
            "--write",
            file_name,
        },
    });
    const status_code: u32 = switch (result.term) {
        .Exited => |e| e,
        .Stopped => |s| s,
        .Unknown => |u| u,
        else => 0,
    };
    if (status_code != 0) {
        std.log.err("{s}", .{result.stderr});
        return error.PrettierError;
    }
}
