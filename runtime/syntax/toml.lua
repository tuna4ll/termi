-- TOML.
return {
  name = "toml",
  extensions = { "toml" },
  filenames = { "cargo.lock", "pipfile", "poetry.lock" },
  constants = words "true false inf nan",
  line_comment = "#",
  prose = true,
  rules = {
    { [==[^\s*\[\[?[^\]]*\]\]?]==], "heading" },
    { [=[^\s*[A-Za-z0-9_.\-"' ]+\s*=]=], "function" },
    { [=["""(?:[^"]|"[^"]|""[^"])*"""|'''[^']*''']=], "string" },
    { [=["(?:[^"\\]|\\.)*"?|'[^']*'?]=], "string" },
    { [=[\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?)?]=], "number" },
    { [=[[+-]?\b(?:0x[0-9a-fA-F_]+|0o[0-7_]+|0b[01_]+|\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?)\b]=], "number" },
    { [=[\b(?:true|false|inf|nan)\b]=], "constant" },
    { [=[#.*]=], "comment" },
  },
}
