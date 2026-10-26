-- Go.
return {
  name = "go",
  extensions = { "go" },
  keywords = words [[
    break case chan const continue default defer else fallthrough for func go
    goto if import interface map package range return select struct switch
    type var
  ]],
  types = words [[
    any bool byte comparable complex64 complex128 error float32 float64 int
    int8 int16 int32 int64 rune string uint uint8 uint16 uint32 uint64 uintptr
  ]],
  constants = words "true false nil iota",
  line_comment = "//",
  block_comment = { "/*", "*/" },
  rules = {
    -- Raw strings in back quotes ignore escapes.
    { [=[`[^`]*`?]=], "string" },
    { [=[\b(?:append|cap|clear|close|copy|delete|len|make|max|min|new|panic|print|println|recover)\b]=], "macro" },
  },
}
