-- Markdown.
--
-- Prose: the generic identifier, operator and punctuation rules are switched
-- off, because running them over English turns every full stop into a token.
-- Fenced code blocks ride on the block-comment machinery: they open and close
-- with the same token and run over many lines, which is exactly its shape.
return {
  name = "markdown",
  extensions = { "md", "markdown", "mdown", "mkd" },
  filenames = { "readme", "changelog" },
  block_comment = { "```", "```" },
  prose = true,
  rules = {
    { [=[^#{1,6}\s.*]=], "heading" },
    -- Setext headings and horizontal rules.
    { [=[^(?:={3,}|-{3,}|\*{3,})\s*$]=], "heading" },
    { [=[^>\s?.*]=], "comment" },
    { [=[`[^`]+`]=], "string" },
    { [=[\*\*[^*]+\*\*|__[^_]+__]=], "emphasis" },
    { [=[\*[^*\s][^*]*\*|_[^_\s][^_]*_]=], "emphasis" },
    { [=[!?\[[^\]]*\]\([^)]*\)]=], "link" },
    { [=[<?https?://[^\s>)]+>?]=], "link" },
    -- List markers and task boxes, at the start of a line only.
    { [=[^\s*(?:[-*+]|\d+[.)])\s]=], "punctuation" },
    { [=[^\s*(?:[-*+])\s\[[ xX]\]]=], "constant" },
  },
}
