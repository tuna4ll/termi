-- TypeScript: JavaScript with types on top.
return {
  name = "typescript",
  extensions = { "ts", "tsx", "mts", "cts" },
  shebangs = { "ts-node", "tsx" },
  keywords = words [[
    abstract as asserts async await break case catch class const continue
    declare default delete do else enum export extends finally for from
    function get if implements import in infer instanceof interface is keyof
    let module namespace new of override private protected public readonly
    return satisfies set static super switch this throw try type typeof unique
    var void while with yield
  ]],
  types = words [[
    any bigint boolean never number object string symbol unknown void Array
    Date Error Map Partial Pick Omit Promise Readonly Record Required Set
    ReturnType console
  ]],
  constants = words "true false null undefined NaN Infinity",
  line_comment = "//",
  block_comment = { "/*", "*/" },
  capitalised_types = true,
  rules = {
    { [=[`(?:[^`\\]|\\.)*`?]=], "string" },
    { [=['(?:[^'\\]|\\.)*'?]=], "string" },
    { [=[@[A-Za-z_]\w*]=], "attribute" },
  },
}
