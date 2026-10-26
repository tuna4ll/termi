-- C++. Listed before C, so a header shared by both resolves here: the larger
-- keyword set degrades more gracefully.
return {
  name = "cpp",
  extensions = { "cpp", "cc", "cxx", "hpp", "hh", "hxx", "ipp" },
  keywords = words [[
    alignas alignof and asm auto break case catch class co_await co_return
    co_yield concept const consteval constexpr constinit const_cast continue
    decltype default delete do dynamic_cast else enum explicit export extern
    final for friend goto if inline mutable namespace new noexcept not operator
    or override private protected public register reinterpret_cast requires
    return sizeof static static_assert static_cast struct switch template this
    thread_local throw try typedef typeid typename union using virtual volatile
    while xor
  ]],
  types = words [[
    bool char char8_t char16_t char32_t double float int long short signed
    unsigned void wchar_t size_t ssize_t ptrdiff_t int8_t int16_t int32_t
    int64_t uint8_t uint16_t uint32_t uint64_t string string_view vector map
    unordered_map set unordered_set array pair tuple optional variant span
    unique_ptr shared_ptr weak_ptr function std
  ]],
  constants = words "true false nullptr NULL",
  line_comment = "//",
  block_comment = { "/*", "*/" },
  capitalised_types = true,
  rules = {
    -- A standalone <...> rule would swallow vector<int>, so the include target
    -- is matched as part of the directive.
    { [=[^\s*#\s*include\s*(?:<[^>]*>|"[^"]*")]=], "attribute" },
    { [=[^\s*#\s*[a-z]+]=], "attribute" },
    { [==[\[\[[A-Za-z_:, ]+\]\]]==], "attribute" },
    -- Raw string literals ignore escapes entirely.
    { [=[R"\([^)]*\)"]=], "string" },
  },
}
