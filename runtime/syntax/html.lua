-- HTML and XML. Comments have no line form, so toggling wraps each line.
return {
  name = "html",
  extensions = { "html", "htm", "xhtml", "xml", "svg", "vue", "svelte" },
  block_comment = { "<!--", "-->" },
  prose = true,
  rules = {
    { [=[<!DOCTYPE[^>]*>]=], "attribute" },
    { [=[</?[A-Za-z][\w:.-]*|/?>]=], "keyword" },
    { [=[\b[A-Za-z_:][\w:.-]*=]=], "function" },
    { [=["[^"]*"|'[^']*']=], "string" },
    { [=[&[A-Za-z0-9#]+;]=], "constant" },
  },
}
