/// Expects `Set` to be an error set.
pub fn isErrorOfType(comptime Set: type, err: anyerror) bool {
    if (@typeInfo(Set).error_set) |names| {
        inline for (names) |name| {
            if (err == @field(anyerror, name.name)) {
                return true;
            }
        }
        return false;
    } else {
        // Set is `anyerror`
        return true;
    }
}
