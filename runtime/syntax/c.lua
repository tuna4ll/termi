-- C.
return {
  name = "c",
  extensions = { "c", "h" },
  keywords = words [[
    auto break case const continue default do else enum extern for goto if
    inline register restrict return sizeof static struct switch typedef union
    volatile while _Alignas _Alignof _Atomic _Generic _Noreturn _Static_assert
    _Thread_local
  ]],
  types = words [[
    char double float int long short signed unsigned void _Bool bool size_t
    ssize_t ptrdiff_t wchar_t intptr_t uintptr_t int8_t int16_t int32_t int64_t
    uint8_t uint16_t uint32_t uint64_t FILE va_list
  ]],
  constants = words "NULL true false EOF EXIT_SUCCESS EXIT_FAILURE",
  line_comment = "//",
  block_comment = { "/*", "*/" },
  rules = {
    -- The include target is matched as part of the directive, so the angle
    -- brackets are never read as comparison operators.
    { [=[^\s*#\s*include\s*(?:<[^>]*>|"[^"]*")]=], "attribute" },
    { [=[^\s*#\s*[a-z]+]=], "attribute" },
  },
}
