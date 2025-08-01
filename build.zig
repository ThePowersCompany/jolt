const std = @import("std");
const build_facilio = @import("facil.io/build.zig").build_facilio;

// Although this function looks imperative, note that its job is to
// declaratively construct a build graph that will be executed by an external
// runner.
pub fn build(b: *std.Build) !void {
    const target = b.standardTargetOptions(.{ .default_target = .{ .cpu_arch = .x86_64 } });
    if (target.result.os.tag == .windows) {
        std.log.err("\x1b[31mPlatform Not Supported\x1b[0m\nCurrently, Facil.io and Zap are not compatible with Windows. Consider using Linux or Windows Subsystem for Linux (WSL) instead.\nFor more information, please see:\n- https://github.com/zigzap/zap#most-faq\n- https://facil.io/#forking-contributing-and-all-that-jazz\n", .{});
        std.process.exit(1);
    }

    // Standard optimization options allow the person running `zig build` to select
    // between Debug, ReleaseSafe, ReleaseFast, and ReleaseSmall. Here we do not
    // set a preferred release mode, allowing the user to decide how to optimize.
    const optimize = b.standardOptimizeOption(.{});

    const facilio = try build_facilio("facil.io", b, target, optimize, true);

    const jolt_module = b.addModule("jolt", .{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
    });
    jolt_module.linkLibrary(facilio);

    const types_exe = b.addExecutable(.{
        .name = "types",
        .root_source_file = b.path("src/typegen.zig"),
        .target = target,
        .optimize = optimize,
    });

    const types_cmd = b.addRunArtifact(types_exe);

    if (b.args) |args| {
        types_cmd.addArgs(args);
    }

    const types_step = b.step("types", "Builds typescript definitions");
    types_step.dependOn(&types_cmd.step);

    // Creates a step for unit testing. This only builds the test executable
    // but does not run it.
    const unit_tests = b.addTest(.{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
    });

    unit_tests.addIncludePath(.{
        .src_path = .{
            .owner = b,
            .sub_path = "facil.io/lib/facil",
        },
    });

    unit_tests.linkLibrary(facilio);

    const run_unit_tests = b.addRunArtifact(unit_tests);

    // Similar to creating the run step earlier, this exposes a `test` step to
    // the `zig build --help` menu, providing a way for the user to request
    // running the unit tests.
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&run_unit_tests.step);
}
