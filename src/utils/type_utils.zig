const std = @import("std");
const expect = std.testing.expect;
const expectEqual = std.testing.expectEqual;

/// Checks if two types are equivalent (have the same fields).
pub inline fn eql(comptime A: type, comptime B: type) bool {
    comptime {
        if (A == B) return true;
        const info_a = switch (@typeInfo(A)) {
            .@"struct" => |struct_info| struct_info,
            else => return false,
        };
        const info_b = switch (@typeInfo(B)) {
            .@"struct" => |struct_info| struct_info,
            else => return false,
        };

        if (info_a.fields.len != info_b.fields.len) {
            return false;
        }

        const FieldTagB = std.meta.FieldEnum(B);
        @setEvalBranchQuota(info_a.fields.len + 1);
        for (info_a.fields) |field_a| {
            if (!@hasField(B, field_a.name)) return false;
            const field_tag_b = @field(FieldTagB, field_a.name);
            const field_b = info_b.fields[@intFromEnum(field_tag_b)];
            if (field_a.type != field_b.type) {
                const field_info_a = @typeInfo(field_a.type);
                const field_info_b = @typeInfo(field_b.type);
                if (field_info_a == .Optional and field_info_b == .Optional) {
                    return eql(field_info_a.Optional.child, field_info_b.Optional.child);
                } else if (field_info_a != .@"struct" or field_info_b != .@"struct") {
                    return false;
                }
                return eql(field_a.type, field_b.type);
            }
            if (field_a.is_comptime != field_b.is_comptime) return false;
        }
        return true;
    }
}

pub fn Union(comptime T: type, comptime U: type) type {
    const t = @typeInfo(T);
    var u = @typeInfo(U);
    switch (t) {
        .@"struct" => |ts| {
            u.@"struct".fields = ts.fields ++ u.@"struct".fields;
            return @Type(u);
        },
        .@"enum" => |te| {
            const t_fields = te.fields;
            const u_fields = u.@"enum".fields;

            // Create an array that is big enough for all fields
            var fields: [t_fields.len + u_fields.len]std.builtin.Type.EnumField = undefined;

            // Copy the T fields
            for (t_fields, 0..) |f, i| {
                fields[i] = .{ .name = f.name, .value = i };
            }

            // Copy the U fields
            // (we start our counter iterator, i, at t_fields.len)
            for (u_fields, t_fields.len..) |f, i| {
                fields[i] = .{ .name = f.name, .value = i };
            }

            // Create new enum type
            return @Type(.{
                .@"enum" = .{
                    .decls = &.{},
                    .tag_type = std.math.IntFittingRange(0, fields.len - 1),
                    .fields = &fields,
                    .is_exhaustive = true,
                },
            });
        },
        else => @compileError("unsupported union subtype"),
    }
}

pub fn ExcludeFields(comptime T: type, comptime fields: []const []const u8) type {
    const t = @typeInfo(T).@"struct";
    var new_fields: std.BoundedArray(std.builtin.Type.StructField, t.fields.len) = .{};

    for (t.fields) |field| {
        for (fields) |exclude_field_name| {
            if (std.mem.eql(u8, field.name, exclude_field_name)) break;
        } else {
            new_fields.appendAssumeCapacity(field);
        }
    }

    var new_info = t;
    new_info.fields = new_fields.constSlice();
    return @Type(.{ .@"struct" = new_info });
}

pub fn merge(base: anytype, additional: anytype) Union(@TypeOf(base), @TypeOf(additional)) {
    const Base = @TypeOf(base);
    const Additional = @TypeOf(additional);

    var result: Union(Base, Additional) = undefined;
    inline for (@typeInfo(Base).@"struct".fields) |field| {
        @field(result, field.name) = @field(base, field.name);
    }

    inline for (@typeInfo(Additional).@"struct".fields) |field| {
        @field(result, field.name) = @field(additional, field.name);
    }

    return result;
}

test {
    const record = .{ .id = 1, .created_on = "2025-01-01" };
    const data = merge(record, .{ .updated_on = "2025-01-02" });
    try expectEqual(record.id, data.id);
    try expectEqual(record.created_on, data.created_on);
    try expectEqual("2025-01-02", data.updated_on);

    const record_info = @typeInfo(@TypeOf(record));
    try expect(record_info == .@"struct");
    try expectEqual(record_info.@"struct".fields.len, 2);

    const data_info = @typeInfo(@TypeOf(data));
    try expect(data_info == .@"struct");
    try expectEqual(data_info.@"struct".fields.len, 3);
}
