-- YAML.
return {
  name = "yaml",
  extensions = { "yaml", "yml" },
  filenames = { ".clang-format", ".clang-tidy" },
  line_comment = "#",
  prose = true,
  rules = {
    { [=[^---\s*$|^\.\.\.\s*$]=], "punctuation" },
    { [=[^\s*-?\s*[A-Za-z0-9_.\-/"' ]+:(?:\s|$)]=], "function" },
    { [=["(?:[^"\\]|\\.)*"?|'[^']*'?]=], "string" },
    { [=[[&*][A-Za-z0-9_\-]+]=], "attribute" },
    { [=[!![A-Za-z]+|![A-Za-z]+]=], "type" },
    { [=[\b(?:true|false|yes|no|on|off|null|~)\b]=], "constant" },
    { [=[[+-]?\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b]=], "number" },
    { [=[(?:^|\s)#.*]=], "comment" },
    { [=[^\s*-\s]=], "punctuation" },
  },
}
