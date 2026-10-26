-- CSS, and the indented-brace dialects close enough to share it.
return {
  name = "css",
  extensions = { "css", "scss", "less" },
  block_comment = { "/*", "*/" },
  prose = true,
  rules = {
    { [=[@[A-Za-z-]+]=], "keyword" },
    { [=["(?:[^"\\]|\\.)*"?|'(?:[^'\\]|\\.)*'?]=], "string" },
    { [=[#[0-9a-fA-F]{3,8}\b]=], "number" },
    { [=[-?\b\d+(?:\.\d+)?(?:px|em|rem|vh|vw|vmin|vmax|%|s|ms|deg|fr|ch|pt)?\b]=], "number" },
    { [=[[A-Za-z-]+\s*:(?:\s|$)]=], "function" },
    { [=[[.#][A-Za-z_-][\w-]*]=], "type" },
    { [=[::?[A-Za-z-]+]=], "attribute" },
    { [=[\$[A-Za-z_-][\w-]*|--[A-Za-z_-][\w-]*]=], "constant" },
    { [=[\b(?:!important|inherit|initial|unset|none|auto)\b]=], "constant" },
    { [=[//.*]=], "comment" },
    { [=[[{}();,]]=], "punctuation" },
  },
}
