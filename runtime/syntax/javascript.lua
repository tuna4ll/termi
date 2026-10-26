-- JavaScript.
return {
  name = "javascript",
  extensions = { "js", "mjs", "cjs", "jsx" },
  shebangs = { "node", "deno", "bun" },
  keywords = words [[
    async await break case catch class const continue debugger default delete
    do else export extends finally for from function get if import in
    instanceof let new of return set static super switch this throw try typeof
    var void while with yield
  ]],
  types = words [[
    Array ArrayBuffer BigInt Boolean Date Error Function Map Math Number Object
    Promise Proxy Reflect RegExp Set String Symbol WeakMap WeakSet JSON console
  ]],
  constants = words "true false null undefined NaN Infinity",
  line_comment = "//",
  block_comment = { "/*", "*/" },
  capitalised_types = true,
  rules = {
    -- Template literals, as far as one line goes.
    { [=[`(?:[^`\\]|\\.)*`?]=], "string" },
    { [=['(?:[^'\\]|\\.)*'?]=], "string" },
    { [=[@[A-Za-z_]\w*]=], "attribute" },
  },
}
