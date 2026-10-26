-- Makefiles. Recipe lines must start with a tab, and toggling comments keeps
-- them that way.
return {
  name = "makefile",
  extensions = { "mk", "mak" },
  filenames = { "makefile", "gnumakefile", "bsdmakefile" },
  line_comment = "#",
  prose = true,
  rules = {
    { [=[^\s*(?:ifeq|ifneq|ifdef|ifndef|else|endif|include|-include|define|endef|export|override|vpath)\b]=], "keyword" },
    { [=[^[A-Za-z0-9_./%$(){} -]+::?(?:[^=]|$)]=], "function" },
    { [=[^\s*[A-Za-z_][\w.]*\s*(?:[:+?!]?=)]=], "constant" },
    { [=[\$\([^)]*\)|\$\{[^}]*\}|\$[@<^+?*%]]=], "macro" },
    { [=["(?:[^"\\]|\\.)*"|'[^']*']=], "string" },
    { [=[#.*]=], "comment" },
  },
}
