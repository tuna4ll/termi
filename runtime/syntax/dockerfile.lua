-- Dockerfiles and Containerfiles.
return {
  name = "dockerfile",
  extensions = { "dockerfile", "containerfile" },
  filenames = { "dockerfile", "containerfile" },
  line_comment = "#",
  prose = true,
  rules = {
    { [=[^\s*(?i:FROM|RUN|CMD|LABEL|MAINTAINER|EXPOSE|ENV|ADD|COPY|ENTRYPOINT|VOLUME|USER|WORKDIR|ARG|ONBUILD|STOPSIGNAL|HEALTHCHECK|SHELL)\b]=], "keyword" },
    { [=[\bAS\b]=], "keyword" },
    { [=[\$\{[^}]*\}|\$[A-Za-z_]\w*]=], "constant" },
    { [=["(?:[^"\\]|\\.)*"|'[^']*']=], "string" },
    { [=[--[a-z-]+(?:=\S*)?]=], "attribute" },
    { [=[^\s*#.*]=], "comment" },
  },
}
