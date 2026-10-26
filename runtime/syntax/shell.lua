-- POSIX shell, bash and zsh.
return {
  name = "shell",
  extensions = { "sh", "bash", "zsh", "ksh", "bats" },
  filenames = { ".bashrc", ".bash_profile", ".zshrc", ".profile", "pkgbuild", ".envrc" },
  shebangs = { "sh", "bash", "zsh", "ksh", "dash" },
  keywords = words [[
    if then else elif fi case esac for while until do done in function select
    time return break continue local export readonly declare typeset unset
    shift trap source alias
  ]],
  types = words "echo printf read cd test exit eval exec set",
  constants = words "true false",
  line_comment = "#",
  rules = {
    { [=[^#!.*]=], "attribute" },
    { [=[\$\{[^}]*\}|\$[A-Za-z_]\w*|\$[0-9@#?*$!-]]=], "constant" },
    { [=['[^']*'?]=], "string" },
    { [=[\$\(]=], "operator" },
    { [=[\s-{1,2}[A-Za-z][\w-]*]=], "attribute" },
  },
}
