-- Java and Kotlin share enough to read well with one definition.
return {
  name = "java",
  extensions = { "java", "kt", "kts" },
  keywords = words [[
    abstract assert break case catch class const continue default do else enum
    extends final finally for fun goto if implements import instanceof
    interface native new object override package private protected public
    record return sealed static strictfp super switch synchronized this throw
    throws transient try val var void volatile when while yield
  ]],
  types = words [[
    boolean byte char double float int long short String Object Integer Long
    Double Boolean List Map Set Optional Unit Any
  ]],
  constants = words "true false null",
  line_comment = "//",
  block_comment = { "/*", "*/" },
  capitalised_types = true,
  rules = {
    { [=[@[A-Za-z_][\w.]*]=], "attribute" },
    { [=["""]=], "string" },
  },
}
