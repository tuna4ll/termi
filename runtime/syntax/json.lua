-- JSON, and JSON with comments.
return {
  name = "json",
  extensions = { "json", "jsonc", "json5", "geojson", "webmanifest" },
  filenames = { ".babelrc", ".eslintrc", "flake.lock" },
  constants = words "true false null",
  line_comment = "//",
  block_comment = { "/*", "*/" },
  rules = {
    -- A string followed by a colon is a key.
    { [=["(?:[^"\\]|\\.)*"\s*:]=], "function" },
  },
}
