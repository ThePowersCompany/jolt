const std = @import("std");
const Type = std.builtin.Type;
const activeTag = std.meta.activeTag;

const zap = @import("../zap/zap.zig");
const MiddlewareContext = zap.Endpoint.MiddlewareContext;
const MiddlewareResult = zap.Endpoint.MiddlewareResult;

const types_mod = @import("../utils/types.zig");
const Unwrap = types_mod.Unwrap;
const unwrapPtr = types_mod.unwrapPtr;

// Built-in middleware
const cors = @import("cors.zig");
const parseQueryParams = @import("parse-query-params.zig").parseQueryParams;
const parseBody = @import("parse-body.zig").parseBody;

pub fn auto(comptime Context: type, ctx: *MiddlewareContext(Context)) !void {
    if (@typeInfo(Context) != .@"struct") {
        @compileError("Endpoint context must be a struct");
    }

    if (cors.isEnabled(Context) or ctx.server.cors) {
        try cors.setCors(&ctx.req);
    }

    if (@hasField(Context, "req")) {
        @field(ctx.deps, "req") = ctx.req;
    }

    try execute(Context, ctx);

    if (@hasField(Context, "query_params")) {
        try parseQueryParams(Context, ctx);
    }

    if (@hasField(Context, "body")) {
        try parseBody(Context, ctx);
    }
}

fn execute(comptime Context: type, ctx: *MiddlewareContext(Context)) !void {
    comptime if (validate(Context)) |d| @compileError(d.message);

    if (!try run(Context, ctx, .{})) return error.MiddlewareError;
}

pub const ValidationError = enum {
    /// A dependency exists only inside a union variant,
    /// so it is not guaranteed to run and cannot satisfy the dependency.
    conditional_dependency,
    /// A dependency is not declared in this context
    /// or any guaranteed enclosing context.
    missing_dependency,
    /// The same middleware type is declared more than once as a guaranteed field of one context,
    /// so a dependency lookup would be ambiguous.
    duplicate_dependency,
    /// A middleware type declared as a guaranteed field is already guaranteed by an enclosing context,
    /// which would shadow it and run it twice.
    shadowed_dependency,
    /// A union middleware field is untagged (union without an enum tag),
    /// so the active variant cannot be discriminated.
    untagged_union,
    /// A required (non-optional) dependency is only satisfied by an optional field,
    /// which may be null at runtime, so the requirement is not guaranteed.
    optional_provider,
};

pub const Diagnostic = struct {
    err: ValidationError,
    message: []const u8,
};

/// Validate that every middleware dependency in a context can be resolved from a guaranteed scope.
/// Returns null when the context is valid.
fn validate(comptime Context: type) ?Diagnostic {
    return validateContextChain(Context, &.{});
}

/// `parent_contexts` - The list of guaranteed enclosing context types,
/// used to resolve dependencies that live outside the current Context's scope.
fn validateContextChain(comptime Context: type, comptime parent_contexts: []const type) ?Diagnostic {
    return comptime blk: {
        if (validateLocalContext(Context, parent_contexts)) |d| break :blk d;

        const contexts_chain = parent_contexts ++ .{Context};

        // Every middleware reachable from this context (fields and their dependencies)
        // must be guaranteed here or in an enclosing scope,
        // exactly as `run` requires before executing.
        for (orderedDependencies(Context, &.{})) |M| {
            if (hasFieldOfType(Context, M)) {
                // Dependencies must be guaranteed from inside of Context.
                // If M itself is stored optionally and a required dependency is null at runtime,
                // M is simply skipped, so an optional provider is fine.
                // Only enforce non-optional providers for a non-optional M.
                if (fieldProvidingIsOptional(Context, M) == false) {
                    for (dependencyFields(dependencies(M))) |f| {
                        if (optionalProviderError(f.type, contexts_chain)) |d| break :blk d;
                    }
                }
                continue;
            }
            // Not guaranteed from Context, it must come from an enclosing scope.
            if (typeListProvides(parent_contexts, M)) continue;
            if (appearsInUnion(Context, M)) break :blk .{
                .err = .conditional_dependency,
                .message = "Middleware '" ++ @typeName(M) ++ "' is only present inside a union variant, " ++
                    "so it is not guaranteed to run and cannot satisfy a dependency. " ++
                    "Declare a guaranteed instance outside the union.",
            };
            break :blk .{
                .err = .missing_dependency,
                .message = "Middleware '" ++ @typeName(M) ++ "' is required as a dependency but is not " ++
                    "declared in this context or any guaranteed enclosing context.",
            };
        }

        // Validate union fields of `Context`
        for (unionFields(Context)) |uf| {
            for (@typeInfo(unwrapOptional(uf.type)).@"union".fields) |vf| {
                const V = vf.type;
                if (std.meta.hasFn(V, "middleware")) {
                    // Each direct dependency must be guaranteed by the current context or an enclosing one.
                    for (dependencyFields(dependencies(V))) |f| {
                        const F: type = extractDependency(f.type, false);
                        if (!typeListProvides(contexts_chain, F)) break :blk .{
                            .err = .missing_dependency,
                            .message = "Union variant '" ++ vf.name ++ "' depends on '" ++ @typeName(F) ++
                                "' which is not guaranteed in this or an enclosing scope.",
                        };
                        // A required dependency needs a non-optional provider
                        if (optionalProviderError(f.type, contexts_chain)) |d| break :blk d;
                    }
                } else {
                    // V is not directly middleware, recurse into the type
                    if (validateContextChain(V, contexts_chain)) |d| break :blk d;
                }
            }
        }
        break :blk null;
    };
}

/// `parent_contexts` - The guaranteed enclosing context types.
fn validateLocalContext(comptime Context: type, comptime parent_contexts: []const type) ?Diagnostic {
    return comptime blk: {
        // Structural checks on this context's fields
        var seen: []const type = &.{};
        for (dependencyFields(Context)) |f| {
            if (isReservedField(f.name)) continue;
            // Untagged or optional union fields cannot be discriminated
            if (unionShapeError(f)) |d| break :blk d;
            // A middleware may be declared at most once as a guaranteed field
            // so that a dependency lookup for it is unambiguous.
            if (@typeInfo(f.type) != .@"union") {
                if (extractMiddleware(f.type, false)) |M| {
                    if (std.mem.indexOfScalar(type, seen, M) != null) break :blk .{
                        .err = .duplicate_dependency,
                        .message = "Middleware '" ++ @typeName(M) ++
                            "' is declared more than once in this context",
                    };
                    // The same type guaranteed by an enclosing context would be shadowed and run twice.
                    if (typeListProvides(parent_contexts, M)) break :blk .{
                        .err = .shadowed_dependency,
                        .message = "Middleware '" ++ @typeName(M) ++
                            "' is already guaranteed by an enclosing context, so declaring it again " ++
                            "here would shadow it and run it twice.",
                    };
                    seen = seen ++ .{M};
                }
            }
        }
        break :blk null;
    };
}

/// Returns whether any type in `types` has a guaranteed (non-union) field `M`.
fn typeListProvides(comptime types: []const type, comptime M: type) bool {
    inline for (types) |C| {
        if (hasFieldOfType(C, M)) return true;
    }
    return false;
}

/// Checks one middleware dependency: if it is required (non-optional),
/// the field providing it must also be non-optional,
/// otherwise it may be null at runtime and the requirement is not guaranteed.
///
/// `DeclaredType` is the dependency field's declared type (e.g. `Auth`, `?Auth`).
/// Its optionality tells us whether the dependency is required,
/// and stripping its `?`/`*` wrappers gives us the middleware type used to find the providing field.
/// `context_chain` - The chain of context types that may provide the dependency.
/// Returns a diagnostic when a required dependency is only provided optionally.
fn optionalProviderError(
    comptime DeclaredType: type,
    comptime context_chain: []const type,
) ?Diagnostic {
    // An optional dependency tolerates a null provider, so only required ones matter.
    if (@typeInfo(DeclaredType) == .optional) return null;
    const Middleware = extractDependency(DeclaredType, false);
    inline for (context_chain) |C| {
        if (fieldProvidingIsOptional(C, Middleware)) |is_optional| {
            if (is_optional) return .{
                .err = .optional_provider,
                .message = "Dependency '" ++ @typeName(Middleware) ++ "' is required, but the field providing" ++
                    " it is optional and may be null at runtime. Make the providing field non-optional, " ++
                    "or make the dependency optional (?" ++ @typeName(Middleware) ++ ").",
            };
            return null;
        }
    }
    return null;
}

/// Whether the guaranteed (non-union) field of `Context` whose type unwraps to `M` is declared optional.
/// Returns null when `Context` has no such field.
fn fieldProvidingIsOptional(comptime Context: type, comptime M: type) ?bool {
    inline for (@typeInfo(Context).@"struct".fields) |f| {
        if (@typeInfo(f.type) == .@"union") continue;
        if (Unwrap(f.type) == M) return @typeInfo(f.type) == .optional;
    }
    return null;
}

/// Run a context's middleware in dependency order, then resolve its union fields.
/// Returns false when a required middleware returns an error.
/// At the top level this will abort, inside a union it tries the next variant.
/// Optional fields tolerate failure as null, and real Zig errors always propagate.
///
/// `parents` is a tuple of pointers to the enclosing contexts, nearest first.
/// A dependency is looked up in `Context` first, then up the parent context chain.
fn run(comptime Context: type, ctx: *MiddlewareContext(Context), parents: anytype) !bool {
    inline for (orderedDependencies(Context, &.{})) |M| {
        // `orderedDependencies` lists transitive dependencies too.
        // A middleware that is not a field of this context
        // is provided by a guaranteed enclosing context that already ran it, so skip it here.
        // `validate` has already proven every middleware is reachable, so no error handling is needed.
        if (comptime !hasFieldOfType(Context, M)) continue;

        const ctx_field = comptime field(Context, M);
        const is_optional = comptime @typeInfo(ctx_field.type) == .optional;

        // Resolve dependencies against this context first, then the parent chain.
        // A required dependency that resolves to null makes buildDeps fail.
        // An optional field tolerates that as null and a guaranteed field propagates it.
        const maybe_deps: ?dependencies(M) = if (comptime is_optional)
            buildDeps(M, .{&ctx.deps} ++ parents) catch |err|
                if (err == error.MiddlewareError) null else return err
        else
            try buildDeps(M, .{&ctx.deps} ++ parents);

        // Run the middleware, reducing to the value to store.
        // `null` means the middleware did not run (no deps) or failed (.err).
        const res = if (maybe_deps) |deps| switch (try M.middleware(&.{
            .deps = deps,
            .alloc = ctx.alloc,
            .server = ctx.server,
            .req = ctx.req,
        })) {
            .ok => |v| v,
            .err => null,
        } else null;

        // An optional field keeps null, a guaranteed field must have a value.
        @field(ctx.deps, ctx_field.name) = if (comptime is_optional)
            res
        else
            res orelse return false;
    }

    inline for (unionFields(Context)) |uf| {
        if (!try runUnion(Context, uf, ctx, parents)) return false;
    }
    return true;
}

/// Try each variant in declaration order and commit the first that succeeds.
/// A middleware .err falls through to the next variant, a real Zig error aborts the union.
/// If no variant succeeds, an optional union field is left null and treated as success,
/// otherwise the union fails and we return false.
fn runUnion(
    comptime Context: type,
    comptime uf: Type.StructField,
    ctx: *MiddlewareContext(Context),
    parents: anytype,
) !bool {
    const is_optional = comptime @typeInfo(uf.type) == .optional;
    const U = comptime unwrapOptional(uf.type);
    inline for (@typeInfo(U).@"union".fields) |vf| {
        const V = vf.type;
        if (comptime std.meta.hasFn(V, "middleware")) {
            const deps = try buildDeps(V, .{&ctx.deps} ++ parents);
            const result: MiddlewareResult(V) = try V.middleware(&.{
                .deps = deps,
                .alloc = ctx.alloc,
                .server = ctx.server,
                .req = ctx.req,
            });
            switch (result) {
                .ok => |v| {
                    @field(ctx.deps, uf.name) = @unionInit(U, vf.name, v);
                    return true;
                },
                // Variant failed, so fall through and try the next one
                .err => {},
            }
        } else {
            // Not directly middleware, recurse
            var sub: MiddlewareContext(V) = .{
                .deps = undefined,
                .alloc = ctx.alloc,
                .server = ctx.server,
                .req = ctx.req,
            };
            if (try run(V, &sub, .{&ctx.deps} ++ parents)) {
                @field(ctx.deps, uf.name) = @unionInit(U, vf.name, sub.deps);
                return true;
            }
        }
    }
    // No variant succeeded - an optional union accepts this as null.
    if (is_optional) {
        @field(ctx.deps, uf.name) = null;
        return true;
    }
    return false;
}

/// Whether Context has a guaranteed (non-union) field whose type unwraps to T.
fn hasFieldOfType(comptime Context: type, comptime T: type) bool {
    inline for (@typeInfo(Context).@"struct".fields) |f| {
        if (@typeInfo(f.type) == .@"union") continue;
        if (Unwrap(f.type) == T) return true;
    }
    return false;
}

/// Build the dependency struct for middleware M
/// by resolving each field against the given context_chain, nearest first.
/// `validate` guarantees each dependency resolves,
/// so a null here for a required dependency is an internal error.
fn buildDeps(comptime M: type, context_chain: anytype) !dependencies(M) {
    const D = dependencies(M);
    var deps: D = undefined;
    inline for (dependencyFields(D)) |f| {
        const F: type = extractDependency(f.type, false);
        const p: ?*F = resolveInParents(F, context_chain);
        @field(deps, f.name) = switch (@typeInfo(f.type)) {
            .pointer => p orelse return error.MiddlewareError,
            .optional => |O| switch (@typeInfo(O.child)) {
                .pointer => p,
                else => if (p) |pp| pp.* else null,
            },
            else => if (p) |pp| pp.* else return error.MiddlewareError,
        };
    }
    return deps;
}

/// Resolve F by walking the scope chain, nearest first.
fn resolveInParents(comptime F: type, parents: anytype) ?*F {
    inline for (parents) |pp| {
        const P = @TypeOf(pp.*);
        if (comptime hasFieldOfType(P, F)) {
            if (ptr(P, F, pp)) |p| return p;
        }
    }
    return null;
}

fn extractDependency(comptime T: type, comptime top_level: bool) type {
    return switch (@typeInfo(T)) {
        .pointer => |P| {
            if (@typeInfo(P.child) == .optional) @compileError(
                "Middleware dependency cannot be a pointer to optional",
            );
            if (top_level) @compileError("Pointers are not supported in endpoint context");
            return P.child;
        },
        .optional => |O| {
            if (@typeInfo(O.child) == .optional) @compileError("Middleware dependency cannot be double optional");
            return extractDependency(O.child, top_level);
        },
        else => T,
    };
}

fn extractMiddleware(comptime T: type, comptime top_level: bool) ?type {
    const B: type = extractDependency(T, top_level);
    return if (@typeInfo(B) == .@"struct" and std.meta.hasFn(B, "middleware")) B else null;
}

/// Fields the framework populates directly, not through middleware.
/// Skipped when scanning a context so their types never reach extractDependency,
/// which matters because a special field may be a pointer (otherwise rejected).
const reserved_field_names = [_][]const u8{ "query_params", "body", "req" };

fn isReservedField(comptime name: []const u8) bool {
    for (reserved_field_names) |reserved| {
        if (std.mem.eql(u8, name, reserved)) return true;
    }
    return false;
}

/// Recursively processes an endpoint context struct and extracts all of the middleware types into a flat list.
/// The resulting flat list is a topological sort (reverse postorder traversal)
/// of the middleware dependency graph.
/// If the middleware is executed in this order,
/// dependencies will always be resolved before their dependents require their result.
/// Union fields are skipped here and resolved separately by runUnion.
fn orderedDependencies(comptime Context: type, comptime stack: []const type) []const type {
    comptime {
        var middlewares: []const type = &.{};
        for (dependencyFields(Context)) |f| {
            if (isReservedField(f.name)) continue;
            if (extractMiddleware(f.type, stack.len == 0)) |M| {
                // Check if middleware recursive dependencies form a cycle
                for (stack) |C| {
                    if (C == M) @compileError("Middleware contains recursive dependency cycle");
                }
                // Check if middleware has already been discovered
                if (std.mem.indexOfScalar(type, middlewares, M) == null) {
                    // Recursively analyze dependencies
                    for (orderedDependencies(dependencies(M), stack ++ .{M})) |A| {
                        if (std.mem.indexOfScalar(type, middlewares, A) == null) {
                            middlewares = middlewares ++ .{A};
                        }
                    }
                    // Add middleware
                    middlewares = middlewares ++ .{M};
                }
            }
        }
        return middlewares;
    }
}

/// Collect the union fields of a context.
fn unionFields(comptime Context: type) []const Type.StructField {
    comptime {
        var fields: []const Type.StructField = &.{};
        for (dependencyFields(Context)) |f| {
            if (isReservedField(f.name)) continue;
            if (@typeInfo(unwrapOptional(f.type)) == .@"union") {
                fields = fields ++ .{f};
            }
        }
        return fields;
    }
}

/// Removes an optional wrapper (`?T`) if it exists, otherwise returns T.
fn unwrapOptional(comptime T: type) type {
    return switch (@typeInfo(T)) {
        .optional => |o| o.child,
        else => T,
    };
}

/// Structural diagnostic for a single context field whose type is a union,
/// or `null` if the field can be used.
/// A union field must be tagged so its active variant can be discriminated.
/// An optional union is allowed and means "run a variant if one succeeds, otherwise leave the field null".
fn unionShapeError(comptime f: Type.StructField) ?Diagnostic {
    const inner = switch (@typeInfo(f.type)) {
        .optional => |O| O.child,
        else => f.type,
    };
    return switch (@typeInfo(inner)) {
        .@"union" => |u| if (u.tag_type == null) .{
            .err = .untagged_union,
            .message = "Union middleware field '" ++ f.name ++
                "' must be a tagged union, union(enum)",
        } else null,
        else => null,
    };
}

/// Get the dependencies of a middleware type.
/// The dependencies are defined at comptime in a struct type, or void.
fn dependencies(comptime M: type) type {
    if (!std.meta.hasFn(M, "middleware")) @compileError("Invalid middleware: Missing function");
    const fnInfo = @typeInfo(@TypeOf(M.middleware)).@"fn";
    if (fnInfo.params.len != 1) @compileError("Invalid middleware: Incorrect number of function params");
    const ctxPtrInfo = @typeInfo(fnInfo.params[0].type orelse @compileError("Null context type!"));
    if (ctxPtrInfo != .pointer) @compileError("Invalid middleware: First param is not a context pointer");
    if (!ctxPtrInfo.pointer.is_const) @compileError("Invalid middleware: Context pointer must be const");
    const Ctx: type = ctxPtrInfo.pointer.child;
    if (@typeInfo(Ctx) != .@"struct" or !@hasField(Ctx, "deps")) @compileError(
        "Invalid middleware: First param is not a context: " ++ @typeName(Ctx),
    );
    const Deps: type = @FieldType(Ctx, "deps");
    if (@typeInfo(Deps) != .@"struct" and Deps != void) @compileError(
        "Invalid middleware: Context dependencies are not a struct",
    );
    return Deps;
}

fn dependencyFields(comptime D: type) []const Type.StructField {
    return switch (@typeInfo(D)) {
        .void => &.{},
        .@"struct" => |S| S.fields,
        else => @compileError("Middleware dependencies must be a struct or void"),
    };
}

/// Search context for middleware and return a read-only pointer to the middleware result memory.
fn ptr(comptime Context: type, comptime Middleware: type, ctx: *Context) ?*Middleware {
    const f = field(Context, Middleware);
    return unwrapPtr(f.type, &(@field(ctx, f.name)));
}

test "ptr" {
    const E = struct {
        i: i32,
    };
    const S = struct {
        e: E,
    };
    var s: S = .{ .e = .{ .i = 123 } };
    var p: ?*E = ptr(S, E, &s);
    p.?.i = 456;
    try std.testing.expectEqual(456, s.e.i);
}

fn field(comptime Context: type, comptime Middleware: type) Type.StructField {
    comptime {
        const ctxInfo = @typeInfo(Context);
        if (ctxInfo != .@"struct") @compileError("Context must be a struct");
        for (ctxInfo.@"struct".fields) |f| {
            if (@typeInfo(f.type) == .@"union") continue;
            if (Unwrap(f.type) == Middleware) return f;
        }
        @compileError(
            "Unable to find middleware defined in endpoint context: " ++ @typeName(Middleware),
        );
    }
}

/// Whether Middleware appears inside any union variant of Context.
/// Used to explain that a dependency exists only conditionally, behind a union.
fn appearsInUnion(comptime Context: type, comptime Middleware: type) bool {
    return comptime blk: {
        for (@typeInfo(Context).@"struct".fields) |f| {
            if (@typeInfo(f.type) != .@"union") continue;
            for (@typeInfo(f.type).@"union".fields) |vf| {
                if (Unwrap(vf.type) == Middleware) break :blk true;
                if (@typeInfo(vf.type) == .@"struct") {
                    for (@typeInfo(vf.type).@"struct".fields) |sf| {
                        if (Unwrap(sf.type) == Middleware) break :blk true;
                    }
                }
            }
        }
        break :blk false;
    };
}

test "field" {
    const E = struct {
        i: i32,
    };
    const S = struct {
        e: E,
    };
    const T = struct {
        e: ?E,
    };
    const p: Type.StructField = field(S, E);
    try std.testing.expectEqualStrings("e", p.name);
    const q: Type.StructField = field(T, E);
    try std.testing.expectEqualStrings("e", q.name);
}

const Auth = struct {
    user: u32,
    pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
        return .{ .ok = .{ .user = 7 } };
    }
};

const Role = struct {
    const D = struct {
        auth: Auth,
    };
    pub fn middleware(_: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
        return .{ .ok = .{} };
    }
};

const Kiosk = struct {
    pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
        return .{ .ok = .{} };
    }
};

test "appearsInUnion - only counts middleware behind a union" {
    const Absent = struct {};
    const Context = struct {
        auth: union(enum) {
            user: Auth,
            kiosk: Kiosk,
        },
    };

    try std.testing.expect(appearsInUnion(Context, Auth));
    try std.testing.expect(appearsInUnion(Context, Kiosk));
    try std.testing.expect(!appearsInUnion(Context, Absent));

    const Nested = struct {
        auth: union(enum) {
            user: struct {
                auth: Auth,
            },
            kiosk: Kiosk,
        },
    };
    try std.testing.expect(appearsInUnion(Nested, Auth));

    const Guaranteed = struct {
        auth: Auth,
    };
    try std.testing.expect(!appearsInUnion(Guaranteed, Auth));
}

test "validate - accepts a context whose dependencies are all guaranteed" {
    const C = struct {
        auth: Auth,
        role: Role,
    };
    try std.testing.expectEqual(null, validate(C));
}

test "validate - rejects a dependency that lives only inside a union" {
    // Role depends on Auth, but Auth only exists inside a union so it is not guaranteed to run
    const C = struct {
        role: Role,
        auth: union(enum) {
            user: Auth,
            kiosk: Kiosk,
        },
    };
    const d = validate(C);
    try std.testing.expect(d != null);
    try std.testing.expectEqual(ValidationError.conditional_dependency, d.?.err);
}

test "validate - rejects a missing dependency" {
    const C = struct {
        role: Role,
    };
    const d = validate(C);
    try std.testing.expect(d != null);
    try std.testing.expectEqual(ValidationError.missing_dependency, d.?.err);
}

test "validate - rejects two sibling union variants that depend on each other" {
    // Role and Auth are mutually exclusive variants of the same union, so Role can never see Auth.
    const C = struct {
        auth: union(enum) {
            role: Role,
            user: Auth,
            kiosk: Kiosk,
        },
    };
    const d = validate(C);
    try std.testing.expect(d != null);
    try std.testing.expectEqual(ValidationError.missing_dependency, d.?.err);
}

test "validate - accepts a variant that walks up to a guaranteed enclosing dependency" {
    // Auth is guaranteed at the top level,
    // so the admin variant's Role can resolve its dependency on Auth.
    const C = struct {
        auth_base: Auth,
        gate: union(enum) {
            admin: struct { role: Role },
            kiosk: Kiosk,
        },
    };
    try std.testing.expectEqual(null, validate(C));
}

test "validate - rejects a nested variant whose dependency is not guaranteed" {
    // Role is two levels deep and Auth exists nowhere guaranteed above it.
    const C = struct {
        outer: union(enum) {
            only: struct {
                inner: union(enum) {
                    x: struct { role: Role },
                },
            },
        },
    };
    const d = validate(C);
    try std.testing.expect(d != null);
    try std.testing.expectEqual(ValidationError.missing_dependency, d.?.err);
}

test "validate - rejects an untagged union middleware field" {
    // Written as `union` instead of `union(enum)`,
    // so the active variant could not be discriminated at runtime.
    const C = struct {
        auth: union {
            user: Auth,
            kiosk: Kiosk,
        },
    };
    const d = validate(C);
    try std.testing.expect(d != null);
    try std.testing.expectEqual(ValidationError.untagged_union, d.?.err);
}

test "validate - accepts an optional union middleware field" {
    const C = struct {
        auth: ?union(enum) {
            user: Auth,
            kiosk: Kiosk,
        },
    };
    try std.testing.expectEqual(null, validate(C));
}

test "validate - rejects an optional untagged union middleware field" {
    const C = struct {
        auth: ?union {
            user: Auth,
            kiosk: Kiosk,
        },
    };
    const d = validate(C);
    try std.testing.expect(d != null);
    try std.testing.expectEqual(ValidationError.untagged_union, d.?.err);
}

test "validate - accepts a properly tagged union middleware field" {
    const C = struct {
        auth: union(enum) {
            user: Auth,
            kiosk: Kiosk,
        },
    };
    try std.testing.expectEqual(null, validate(C));
}

test "validate - rejects the same middleware declared twice as guaranteed fields" {
    const C = struct {
        a: Auth,
        b: Auth,
    };
    const d = validate(C);
    try std.testing.expect(d != null);
    try std.testing.expectEqual(ValidationError.duplicate_dependency, d.?.err);
}

test "validate - rejects a variant field that shadows a middleware guaranteed by an enclosing context" {
    const C = struct {
        auth: Auth,
        gate: union(enum) {
            admin: struct { auth: Auth },
            kiosk: Kiosk,
        },
    };
    const d = validate(C);
    try std.testing.expect(d != null);
    try std.testing.expectEqual(ValidationError.shadowed_dependency, d.?.err);
}

test "validate - rejects shadowing across multiple levels of nesting" {
    const C = struct {
        auth: Auth,
        outer: union(enum) {
            only: struct {
                inner: union(enum) {
                    x: struct { auth: Auth },
                },
            },
        },
    };
    const d = validate(C);
    try std.testing.expect(d != null);
    try std.testing.expectEqual(ValidationError.shadowed_dependency, d.?.err);
}

test "validate - rejects a required dependency satisfied by an optional field" {
    // Role requires Auth, but the context provides Auth optionally,
    // so it may be null at runtime and the requirement is not guaranteed.
    const C = struct {
        auth: ?Auth,
        role: Role,
    };
    const d = validate(C);
    try std.testing.expect(d != null);
    try std.testing.expectEqual(ValidationError.optional_provider, d.?.err);
}

test "validate - rejects a union variant requiring a dependency provided optionally by an enclosing context" {
    const C = struct {
        auth: ?Auth,
        gate: union(enum) {
            admin: struct { role: Role },
            kiosk: Kiosk,
        },
    };
    const d = validate(C);
    try std.testing.expect(d != null);
    try std.testing.expectEqual(ValidationError.optional_provider, d.?.err);
}

const OptRole = struct {
    const D = struct { auth: ?Auth };
    pub fn middleware(_: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
        return .{ .ok = .{} };
    }
};

test "validate - accepts an optional dependency satisfied by an optional field" {
    const C = struct {
        auth: ?Auth,
        role: OptRole,
    };
    try std.testing.expectEqual(null, validate(C));
}

test "validate - accepts a required dependency satisfied by a non-optional field" {
    const C = struct {
        auth: Auth,
        role: Role,
    };
    try std.testing.expectEqual(null, validate(C));
}

test "validate - accepts an optional dependency for an optional field" {
    const C = struct {
        auth: ?Auth,
        role: ?OptRole,
    };
    try std.testing.expectEqual(null, validate(C));
}

test "validate - accepts an optional provider for an optional dependent middleware" {
    const C = struct {
        auth: ?Auth,
        role: ?Role,
    };
    try std.testing.expectEqual(null, validate(C));
}

test "orderedDependencies and execute" {
    const M1 = struct {
        pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            return .{ .ok = .{} };
        }
    };
    const M2 = struct {
        const D = struct {
            m: M1,
        };

        pub fn middleware(_: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
            return .{ .ok = .{} };
        }
    };
    const M3 = struct {
        const D = struct {
            m: M1,
            m2: *M2,
        };

        pub fn middleware(_: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
            return .{ .ok = .{} };
        }
    };

    const C = struct {
        m: M3,
        m1: M1,
        m2: M2,
    };

    const t = orderedDependencies(C, &.{});
    try std.testing.expectEqual(3, t.len);
    try std.testing.expectEqual(M1, t[0]);
    try std.testing.expectEqual(M2, t[1]);
    try std.testing.expectEqual(M3, t[2]);

    var ctx: MiddlewareContext(C) = .{
        .deps = undefined,
        .alloc = std.testing.allocator,
        .server = undefined,
        .req = undefined,
    };
    try execute(C, &ctx);
}

test "union - first successful variant wins" {
    const A = struct {
        pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            return .{ .ok = .{} };
        }
    };
    const B = struct {
        pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            return .{ .ok = .{} };
        }
    };
    const C = struct {
        auth: union(enum) {
            a: A,
            b: B,
        },
    };

    var ctx: MiddlewareContext(C) = .{
        .deps = undefined,
        .alloc = std.testing.allocator,
        .server = undefined,
        .req = undefined,
    };
    try execute(C, &ctx);
    try std.testing.expect(activeTag(ctx.deps.auth) == .a);
}

test "union - falls through to a later variant" {
    const Fails = struct {
        pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            return .{ .err = .{ .status = .unauthorized, .msg = "nope" } };
        }
    };
    const Succeeds = struct {
        pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            return .{ .ok = .{} };
        }
    };
    const C = struct {
        auth: union(enum) {
            first: Fails,
            second: Succeeds,
        },
    };

    var ctx: MiddlewareContext(C) = .{
        .deps = undefined,
        .alloc = std.testing.allocator,
        .server = undefined,
        .req = undefined,
    };
    try execute(C, &ctx);
    try std.testing.expectEqual(.second, activeTag(ctx.deps.auth));
}

test "union - struct variant resolves an in-variant dependency" {
    const Authentication = struct {
        id: u32,
        pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            return .{ .ok = .{ .id = 7 } };
        }
    };
    const AtLeastRole = struct {
        const D = struct {
            auth: Authentication,
        };
        pub fn middleware(ctx: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
            if (ctx.deps.auth.id == 7) return .{ .ok = .{} };
            return .{ .err = .{ .status = .forbidden, .msg = "role" } };
        }
    };
    const C = struct {
        foo: union(enum) {
            user: struct {
                auth: Authentication,
                role: AtLeastRole,
            },
            kiosk: Kiosk,
        },
    };

    var ctx: MiddlewareContext(C) = .{
        .deps = undefined,
        .alloc = std.testing.allocator,
        .server = undefined,
        .req = undefined,
    };
    try execute(C, &ctx);
    try std.testing.expectEqual(.user, activeTag(ctx.deps.foo));
    try std.testing.expectEqual(7, ctx.deps.foo.user.auth.id);
}

test "union - struct variant walks up to a guaranteed enclosing dependency" {
    const Authentication = struct {
        user: u32,
        pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            return .{ .ok = .{ .user = 7 } };
        }
    };
    const AtLeastRole = struct {
        const D = struct {
            auth: Authentication,
        };
        pub fn middleware(ctx: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
            if (ctx.deps.auth.user == 7) return .{ .ok = .{} };
            return .{ .err = .{ .status = .forbidden, .msg = "role" } };
        }
    };

    const C = struct {
        auth: Authentication,
        gate: union(enum) {
            admin: struct { role: AtLeastRole },
            kiosk: Kiosk,
        },
    };

    var ctx: MiddlewareContext(C) = .{
        .deps = undefined,
        .alloc = std.testing.allocator,
        .server = undefined,
        .req = undefined,
    };
    try execute(C, &ctx);
    try std.testing.expectEqual(7, ctx.deps.auth.user);
    try std.testing.expectEqual(.admin, activeTag(ctx.deps.gate));
}

test "union - bare variant walks up to a guaranteed enclosing dependency" {
    const Authentication = struct {
        user: u32,
        pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            return .{ .ok = .{ .user = 7 } };
        }
    };
    const AtLeastRole = struct {
        const D = struct {
            auth: Authentication,
        };
        pub fn middleware(ctx: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
            if (ctx.deps.auth.user == 7) return .{ .ok = .{} };
            return .{ .err = .{ .status = .forbidden, .msg = "role" } };
        }
    };
    const C = struct {
        auth_base: Authentication,
        gate: union(enum) {
            admin: AtLeastRole,
        },
    };

    var ctx: MiddlewareContext(C) = .{
        .deps = undefined,
        .alloc = std.testing.allocator,
        .server = undefined,
        .req = undefined,
    };
    try execute(C, &ctx);
    try std.testing.expectEqual(.admin, activeTag(ctx.deps.gate));
}

const Base = struct {
    v: u32,
    pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
        return .{ .ok = .{ .v = 1 } };
    }
};

const Mid = struct {
    v: u32,
    const D = struct { base: Base };
    pub fn middleware(ctx: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
        // Mid can only run correctly if Base was resolved from an outer scope.
        if (ctx.deps.base.v != 1) return .{ .err = .{ .status = .forbidden, .msg = "base" } };
        return .{ .ok = .{ .v = 2 } };
    }
};

test "nested containers - leaf walks up two levels to a grandparent dependency" {
    const Leaf = struct {
        const D = struct { base: Base };
        pub fn middleware(ctx: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
            if (ctx.deps.base.v == 1) return .{ .ok = .{} };
            return .{ .err = .{ .status = .forbidden, .msg = "leaf" } };
        }
    };
    const C = struct {
        base: Base,
        outer: union(enum) {
            only: struct {
                inner: union(enum) {
                    x: struct { leaf: Leaf },
                },
            },
        },
    };

    var ctx: MiddlewareContext(C) = .{
        .deps = undefined,
        .alloc = std.testing.allocator,
        .server = undefined,
        .req = undefined,
    };
    try execute(C, &ctx);
    try std.testing.expectEqual(1, ctx.deps.base.v);
    try std.testing.expectEqual(.only, activeTag(ctx.deps.outer));
    try std.testing.expectEqual(.x, activeTag(ctx.deps.outer.only.inner));
}

test "nested containers - leaf resolves dependencies from two different ancestor scopes" {
    const Leaf = struct {
        const D = struct { base: Base, mid: Mid };
        pub fn middleware(ctx: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
            if (ctx.deps.base.v == 1 and ctx.deps.mid.v == 2) return .{ .ok = .{} };
            return .{ .err = .{ .status = .forbidden, .msg = "leaf" } };
        }
    };
    const C = struct {
        base: Base,
        outer: union(enum) {
            a: struct {
                mid: Mid,
                inner: union(enum) {
                    x: struct { leaf: Leaf },
                },
            },
        },
    };

    var ctx: MiddlewareContext(C) = .{
        .deps = undefined,
        .alloc = std.testing.allocator,
        .server = undefined,
        .req = undefined,
    };
    try execute(C, &ctx);
    try std.testing.expectEqual(.a, activeTag(ctx.deps.outer));
    try std.testing.expectEqual(.x, activeTag(ctx.deps.outer.a.inner));
    try std.testing.expectEqual(2, ctx.deps.outer.a.mid.v);
}

test "nested containers - inner union falls through then walks up" {
    const Fails = struct {
        pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            return .{ .err = .{ .status = .unauthorized, .msg = "nope" } };
        }
    };
    const Leaf = struct {
        const D = struct { base: Base };
        pub fn middleware(ctx: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
            if (ctx.deps.base.v == 1) return .{ .ok = .{} };
            return .{ .err = .{ .status = .forbidden, .msg = "leaf" } };
        }
    };
    const C = struct {
        base: Base,
        outer: union(enum) {
            a: struct {
                inner: union(enum) {
                    first: Fails,
                    second: struct { leaf: Leaf },
                },
            },
        },
    };

    var ctx: MiddlewareContext(C) = .{
        .deps = undefined,
        .alloc = std.testing.allocator,
        .server = undefined,
        .req = undefined,
    };
    try execute(C, &ctx);
    try std.testing.expectEqual(.second, activeTag(ctx.deps.outer.a.inner));
}

test "nested containers - deep bare variant walks up to a grandparent dependency" {
    const Leaf = struct {
        const D = struct { base: Base };
        pub fn middleware(ctx: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
            if (ctx.deps.base.v == 1) return .{ .ok = .{} };
            return .{ .err = .{ .status = .forbidden, .msg = "leaf" } };
        }
    };
    const C = struct {
        base: Base,
        outer: union(enum) {
            a: struct {
                inner: union(enum) {
                    only: Leaf,
                },
            },
        },
    };

    var ctx: MiddlewareContext(C) = .{
        .deps = undefined,
        .alloc = std.testing.allocator,
        .server = undefined,
        .req = undefined,
    };
    try execute(C, &ctx);
    try std.testing.expectEqual(.only, activeTag(ctx.deps.outer.a.inner));
}

test "optional - a failed optional middleware is tolerated and dependents see null" {
    const Flaky = struct {
        pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            return .{ .err = .{ .status = .unauthorized, .msg = "down" } };
        }
    };
    const Dependent = struct {
        saw_null: bool,
        const D = struct { flaky: ?Flaky };
        pub fn middleware(ctx: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
            return .{ .ok = .{ .saw_null = ctx.deps.flaky == null } };
        }
    };

    const C = struct {
        flaky: ?Flaky,
        dep: Dependent,
    };

    var ctx: MiddlewareContext(C) = .{
        .deps = undefined,
        .alloc = std.testing.allocator,
        .server = undefined,
        .req = undefined,
    };
    // `execute` succeeds even though Flaky failed, because the field is optional.
    try execute(C, &ctx);
    try std.testing.expectEqual(null, ctx.deps.flaky);
    try std.testing.expect(ctx.deps.dep.saw_null);
}

test "optional - an optional middleware whose required dependency is null is skipped" {
    const FailAuth = struct {
        v: u32,
        pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            return .{ .err = .{ .status = .unauthorized, .msg = "no auth" } };
        }
    };
    const NeedsAuth = struct {
        const D = struct { auth: FailAuth };
        pub fn middleware(_: *const MiddlewareContext(D)) !MiddlewareResult(@This()) {
            return .{ .ok = .{} };
        }
    };
    const C = struct {
        auth: ?FailAuth,
        role: ?NeedsAuth,
    };

    var ctx: MiddlewareContext(C) = .{
        .deps = undefined,
        .alloc = std.testing.allocator,
        .server = undefined,
        .req = undefined,
    };
    try execute(C, &ctx);
    try std.testing.expectEqual(null, ctx.deps.auth);
    try std.testing.expectEqual(null, ctx.deps.role);
}

test "optional union - leaves the field null when no variant succeeds" {
    const FailA = struct {
        pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            return .{ .err = .{ .status = .unauthorized, .msg = "a" } };
        }
    };
    const FailB = struct {
        pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            return .{ .err = .{ .status = .unauthorized, .msg = "b" } };
        }
    };
    const C = struct {
        auth: ?union(enum) {
            a: FailA,
            b: FailB,
        },
    };

    var ctx: MiddlewareContext(C) = .{
        .deps = undefined,
        .alloc = std.testing.allocator,
        .server = undefined,
        .req = undefined,
    };
    try execute(C, &ctx);
    try std.testing.expectEqual(null, ctx.deps.auth);
}

test "optional union - commits the first variant that succeeds" {
    const FailA = struct {
        pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            return .{ .err = .{ .status = .unauthorized, .msg = "a" } };
        }
    };
    const OkB = struct {
        v: u32 = 7,
        pub fn middleware(_: *const MiddlewareContext(void)) !MiddlewareResult(@This()) {
            return .{ .ok = .{} };
        }
    };
    const C = struct {
        auth: ?union(enum) {
            a: FailA,
            b: OkB,
        },
    };

    var ctx: MiddlewareContext(C) = .{
        .deps = undefined,
        .alloc = std.testing.allocator,
        .server = undefined,
        .req = undefined,
    };
    try execute(C, &ctx);
    try std.testing.expect(ctx.deps.auth != null);
    try std.testing.expectEqual(.b, activeTag(ctx.deps.auth.?));
}
