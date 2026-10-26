-- Diffs and patches.
--
-- A diff is line oriented: what a line means is decided by its first
-- character, so every rule is anchored at the start of the line and paints the
-- whole of it. Headers come first, so the three that begin with - or + are
-- not read as removed or added lines.
return {
  name = "diff",
  extensions = { "diff", "patch", "rej" },
  prose = true,
  rules = {
    { [=[^(?:---|\+\+\+)(?:\s.*)?$]=], "diff_meta" },
    { [=[^diff .*]=], "diff_meta" },
    { [=[^(?:index|old mode|new mode|new file mode|deleted file mode|similarity index|dissimilarity index|copy from|copy to|rename from|rename to|Binary files|GIT binary patch) .*]=], "diff_meta" },
    -- The mail headers git format-patch writes above the diff itself.
    { [=[^(?:From|Date|Subject)(?::| [0-9a-f]{7,}).*]=], "diff_meta" },
    -- Two @ for a unified hunk, three for a combined one from a merge.
    { [=[^@{2,3} .*]=], "diff_hunk" },
    { [=[^\+.*]=], "diff_added" },
    { [=[^-.*]=], "diff_removed" },
    -- The older diff output: a 3,4c3,4 range, then < and > lines.
    { [=[^\d+(?:,\d+)?[acd]\d+(?:,\d+)?$]=], "diff_hunk" },
    { [=[^>.*]=], "diff_added" },
    { [=[^<.*]=], "diff_removed" },
    -- \ No newline at end of file
    { [=[^\\ .*]=], "comment" },
  },
}
