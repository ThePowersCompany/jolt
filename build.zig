const std = @import("std");
const build_facilio = @import("facil.io/build.zig").build_facilio;

var deps = [_]std.Build.Module.Import{};

fn define_deps(
    b: *std.Build,
    target: std.Build.ResolvedTarget,
    optimize: std.builtin.OptimizeMode,
) void {
    for (&deps) |*dep_type| {
        dep_type.module = b.dependency(dep_type.name, .{
            .target = target,
            .optimize = optimize,
        }).module(dep_type.name);
    }
}

fn import_deps(
    unit: *std.Build.Step.Compile,
) void {
    for (deps) |dep_type| {
        unit.root_module.addImport(dep_type.name, dep_type.module);
    }
}

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

    define_deps(b, target, optimize);

    const use_llvm = b.option(bool, "llvm", "if llvm backend should be used (default: true)") orelse true;
    const exe = b.addExecutable(.{
        .name = "server",
        // In this case the main source file is merely a path, however, in more
        // complicated build scripts, this could be a generated file.
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
        .use_llvm = use_llvm,
    });

    exe.linker_allow_shlib_undefined = true;

    const facilio = try build_facilio("facil.io", b, target, optimize, true);
    exe.linkLibrary(facilio);
    exe.addIncludePath(.{
        .src_path = .{
            .owner = b,
            .sub_path = "facil.io/lib/facil",
        },
    });

    import_deps(exe);

    // Required by "dotenv" library
    exe.linkSystemLibrary("c");

    const no_bin = b.option(bool, "no-bin", "skip emitting binary") orelse false;
    if (no_bin) {
        b.getInstallStep().dependOn(&exe.step);
    } else {
        // This declares intent for the executable to be installed into the
        // standard location when the user invokes the "install" step (the default
        // step when running `zig build`).
        b.installArtifact(exe);
    }

    // This *creates* a Run step in the build graph, to be executed when another
    // step is evaluated that depends on it. The next line below will establish
    // such a dependency.
    const run_cmd = b.addRunArtifact(exe);

    // By making the run step depend on the install step, it will be run from the
    // installation directory rather than directly from within the cache directory.
    // This is not necessary, however, if the application depends on other installed
    // files, this ensures they will be present and in the expected location.
    run_cmd.step.dependOn(b.getInstallStep());

    // This allows the user to pass arguments to the application in the build
    // command itself, like this: `zig build run -- arg1 arg2 etc`
    if (b.args) |args| {
        run_cmd.addArgs(args);
    }

    // This creates a build step. It will be visible in the `zig build --help` menu,
    // and can be selected like this: `zig build run`
    // This will evaluate the `run` step rather than the default, which is "install".
    const run_step = b.step("run", "Run the app");
    run_step.dependOn(&run_cmd.step);

    const types_exe = b.addExecutable(.{
        .name = "types",
        .root_source_file = b.path("src/typegen.zig"),
        .target = target,
        .optimize = optimize,
    });

    import_deps(types_exe);

    const types_cmd = b.addRunArtifact(types_exe);

    if (b.args) |args| {
        types_cmd.addArgs(args);
    }

    const types_step = b.step("types", "Builds typescript definitions");
    types_step.dependOn(&types_cmd.step);

    _ = b.addModule("jolt", .{
        .target = target,
        .optimize = optimize,
        .root_source_file = b.path("src/main.zig"),
        .imports = &deps,
    });

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

    import_deps(unit_tests);

    const run_unit_tests = b.addRunArtifact(unit_tests);

    // Similar to creating the run step earlier, this exposes a `test` step to
    // the `zig build --help` menu, providing a way for the user to request
    // running the unit tests.
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&run_unit_tests.step);
}
