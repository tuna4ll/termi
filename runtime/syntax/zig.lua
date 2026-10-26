-- Zig.
return {
  name = "zig",
  extensions = { "zig", "zon" },
  keywords = words [[
    align allowzero and anyframe anytype asm async await break callconv catch
    comptime const continue defer else enum errdefer error export extern fn for
    if inline linksection noalias noinline nosuspend opaque or orelse packed
    pub resume return struct suspend switch test threadlocal try union
    unreachable usingnamespace var volatile while
  ]],
  types = words [[
    anyerror anyopaque bool c_char c_int c_long c_longdouble c_longlong c_short
    c_uint c_ulong c_ulonglong c_ushort comptime_float comptime_int f16 f32 f64
    f80 f128 i8 i16 i32 i64 i128 isize noreturn type u8 u16 u32 u64 u128 usize
    void
  ]],
  constants = words "true false null undefined",
  line_comment = "//",
  capitalised_types = true,
  rules = {
    { [=[///.*]=], "comment" },
    { [=[@[A-Za-z_]\w*]=], "macro" },
    -- Multiline string literals are line-scoped in Zig, which suits a per-line
    -- scanner exactly.
    { [=[\\\\.*]=], "string" },
  },
}
