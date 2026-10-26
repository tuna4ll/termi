-- SQL. Keywords are matched in any case, the way SQL itself reads them.
return {
  name = "sql",
  extensions = { "sql", "psql", "ddl" },
  line_comment = "--",
  block_comment = { "/*", "*/" },
  rules = {
    { [=[(?i)\b(?:select|from|where|insert|into|values|update|set|delete|create|alter|drop|table|index|view|join|inner|left|right|outer|full|cross|on|as|and|or|not|in|is|null|like|between|group|by|order|having|limit|offset|union|all|distinct|case|when|then|else|end|exists|primary|key|foreign|references|default|unique|constraint|begin|commit|rollback|transaction|with|returning|asc|desc|if)\b]=], "keyword" },
    { [=[(?i)\b(?:int|integer|bigint|smallint|serial|bigserial|text|varchar|char|boolean|bool|date|time|timestamp|timestamptz|numeric|decimal|real|float|double|json|jsonb|uuid|blob|bytea)\b]=], "type" },
    { [=[(?i)\b(?:true|false)\b]=], "constant" },
    { [=['(?:[^']|'')*'?]=], "string" },
  },
}
