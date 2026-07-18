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
    io: std.Io,
    ts_file_name: []const u8,
    comptime endpoints: []const EndpointDef,
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

    const file = try std.Io.Dir.cwd().createFile(io, ts_file_name, .{});
    defer file.close(io);
    try file.writeStreamingAll(io, ts.items);

    try formatWithPrettier(arena_alloc, io, ts_file_name);
}

/// Uses prettier to format the given TS file.
fn formatWithPrettier(alloc: Allocator, io: std.Io, file_name: []const u8) !void {
    const result = try std.process.run(alloc, io, .{
        .argv = &[_][]const u8{
            "npx",
            "prettier",
            "--write",
            file_name,
        },
    });
    const status_code: u32 = switch (result.term) {
        .exited => |e| e,
        .stopped, .signal => 1,
        .unknown => |u| u,
    };
    if (status_code != 0) {
        std.log.err("{s}", .{result.stderr});
        return error.PrettierError;
    }
}
