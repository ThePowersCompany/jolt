# Source Directory

Source code goes here, obviously.

## Query Parameter Parsing Algorithm

Jolt includes powerful query parameter parsing middleware out of the box.
This section documents the parsing behavior and provides useful samples.

All samples below define a `QP` type that would be assigned to the `query_params` field in an endpoint's context type.

### Sample 1: Simple

In the most simple case, query params are declared in a flat struct and input query params will be automatically deserialized into this type:

```zig
const QP = struct {
    a: i32,                             // required: a=123
    b: ?i32,                            // required: b=123 or b=null
    c: ?i32 = null,                     // optional: c=123 or c=null
    d: Optional(i32) = .not_provided,   // optional: d=123
    s: []const u8,                      // required: s=abc
    t: ?[]const u8,                     // required: t=abc or t=null (not recommended, unable to parse `t` as "null")
    x: []const u8 = "sample",           // optional: x=abc
};
```

Fields must have a default value provided for them to be considered "optional": `a: ?T = null`. A nullable field (`?T`) on its own is not optional and is instead parsed from a `null` sentinel string. If this is paired with a string type, then it's impossible to represent a string `"null"` because it's taken up by the parsed `null` value.

### Sample 2: Unions

Zig tagged unions allow declaration of sets of mutually exclusive parameters.

```zig
const QP = union(enum) {
    id: i32,
    date_range: struct {
        start_date: []const u8,
        end_date: []const u8,
    },
    pagination: struct {
        cursor: ?u64 = null,
        limit: ?u32 = null,
    },
};
```

The union is logically "flattened" into the following structure, but the middleware intelligently parses a plain query string (KV pairs) into the original union with nested structs and ensures all the expected constraints are validated and only one enum variant is selected.

```zig
const QP = struct {
    id: Optional(i32) = .not_provided,
    start_date: Optional([]const u8) = .not_provided,
    end_date: Optional([]const u8) = .not_provided,
    cursor: Optional(?u64) = .not_provided,
    limit: Optional(?u32) = .not_provided,
};
```

> `date_range` and `pagination` do not show up in the flattened structure, because they are shadowed by the nested struct's fields.

This can be confusing because usually a tagged union represents a single value.
Continue to the next sample for how to parse a single parameter string into a scalar union value.

### Sample 3: Scalars and `paramParse`

A scalar type is any type that can be parsed from a single query parameter value string. The middleware has built-in support to parse most Zig types (`i32`, `i64`, `u32`, `u64`, `f32`, `f64`, `[]const u8`, etc).

A `struct` is **NOT** a scalar type, _unless_ it has a `T.paramParse(Allocator, []const u8) !T` function declared on it.

A `union(enum)` type is considered scalar if **ALL** of its variant types are also scalar. Each variant type will attempt to parse the query parameter value string (from top to bottom) until one succeeds or all the variants are exhausted.

```zig
const IdOrAuto = union(enum) {
    id: i32,
    auto, // matches "auto" string literal
};

const Number = union(enum) {
    small: i32, // first, attempt to parse into small integer type
    big: i64, // then try to parse into larger integer type
};

const DateStr = struct {
    year: i32,
    month: i32,
    day: i32,

    pub fn paramParse(alloc: Allocator, value: []const u8) !@This() {
        // ...
    }
};

const QP = struct {
    q: IdOrAuto,
    n: Optional(Number) = .not_provided,
    date: ?DateStr = null,
};
```

It's invalid to set a scalar value directly as the query params type:

```zig
const QP = IdOrAuto; // comptime error because `auto`-variant is unrepresentable
```

However, if all of the variants of the scalar type are non-`void`, then the union can be set directly as the query params. These types are called "weak scalars". Types with a `void` variant (e.g. `IdOrAuto`) are called "strong scalars".

```zig
const QP = union(enum) {
    id: i32,
    date: DateStr,
};
```

The logically flattened structure would look like this:

```zig
const QP = struct {
    id: Optional(i32) = .not_provided,
    date: Optional(DateStr) = .not_provided,
};
```

### Sample 5: Require Together

Structs can be used to require multiple query parameters to be provided together, or none at all:

```zig
const QP = struct {
    date_range: ?struct {
        start_date: []const u8,
        end_date: []const u8,
    } = null,
};
```

If only `start_date` is provided, or only `end_date`, the middleware will produce a query param validation error.

### Sample 6: Error - Duplicate Fields

After flattening the structure, there may be duplicate fields:

```zig
const QP = struct {
    foo: struct {
        a: i32,
    },
    bar: struct {
        a: i32,
    },
};
```

It doesn't matter if the fields are different types or different optionality.

Unions can disrupt this requirement because only one variant can be selected at once,
but **ONLY** if the variant can be unambiguously parsed.

### Sample 6: Error - Ambiguous Variants

If two or more union variants have the same required fields,
then a compile error will be raised because it won't be possible to unambiguously select a variant at runtime.

```zig
const QP = union(enum) {
    foo: struct {
        a: []const u8,
    },
    bar: struct {
        a: []const u8,
        b: ?[]const u8 = null,
    },
};
```

If only `a` is provided at runtime, both `foo` and `bar` would successfully parse. An argument could be made that `foo` parses more "completely", but we figured this could lead to bugs if allowed.

If `bar` is changed to require `b`, then the ambiguity is removed:

```zig
const QP = union(enum) {
    foo: struct {
        a: []const u8,
    },
    bar: struct {
        a: []const u8,
        b: []const u8,
    },
};
```

### Sample 7: Complex Nested Structs and Unions

The middleware can handle heavily nested combinations of unions and structs - the flattening logic will be applied recursively.

```zig
const QP = struct {
    filter: union(enum) {
        id: i32,
        name: union(enum) {
            first: []const u8,
            last: []const u8,
            full: struct {
                first: []const u8,
                last: []const u8,
            },
        },
        date_range: struct {
            start_date: DateStr,
            end_date: DateStr,
        },
        pagination: struct {
            cursor: ?u64 = null,
            limit: ?u32 = null,
        },
    },
    order_by: ?enum {
        asc,
        desc,
    } = null,
    inactive: Optional(bool) = .not_provided,
};
```

#### Sample Inputs

Here are some sample inputs and how they are parsed into the nested structure:

##### ID

Input:

> id=123

Output:

```zig
.{
    .filter = .{
        .id = 123,
    },
    ...
}
```

##### Date Range

Input:

> start_date=2026-07-06&end_date=2026-07-08&order_by=asc

Output:

```zig
.{
    .filter = .{
        .date_range = .{
            .start_date = ...,
            .end_date = ...,
        },
    },
    order_by = .asc,
    ...
}
```

##### Empty

Input:

> (empty)

Output:

```zig
.{
    .filter = .{
        .pagination = .{
            .cursor = null,
            .limit = null,
        },
    },
    ...
}
```
