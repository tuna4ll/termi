-- Lua — which is also what these files are written in.
return {
  name = "lua",
  extensions = { "lua", "rockspec" },
  shebangs = { "lua", "luajit" },
  keywords = words [[
    and break do else elseif end for function goto if in local not or repeat
    return then until while
  ]],
  types = words "string table math io os coroutine utf8 debug package",
  constants = words "true false nil self _G _ENV",
  line_comment = "--",
  block_comment = { "--[[", "]]" },
  rules = {
    { [==[\[(=*)\[]==], "string" },
    { [=['(?:[^'\\]|\\.)*'?]=], "string" },
  },
}
