-- Python. Docstrings ride on the block-comment machinery.
return {
  name = "python",
  extensions = { "py", "pyi", "pyw" },
  shebangs = { "python", "python3", "python2" },
  keywords = words [[
    and as assert async await break case class continue def del elif else
    except finally for from global if import in is lambda match nonlocal not or
    pass raise return try while with yield
  ]],
  types = words [[
    bool bytearray bytes complex dict float frozenset int list object set str
    tuple type Any Callable Dict Iterable Iterator List Optional Sequence Set
    Tuple Union
  ]],
  constants = words [[
    True False None NotImplemented Ellipsis self cls __name__ __file__
  ]],
  line_comment = "#",
  block_comment = { '"""', '"""' },
  capitalised_types = true,
  rules = {
    { [=[^\s*@[A-Za-z_][\w.]*]=], "attribute" },
    -- The ''' form of a docstring, when it opens and closes on one line.
    { [=['''[^']*''']=], "comment" },
    -- Prefixed string literals: f-strings, byte strings, raw strings.
    { [=[(?:[fFrRbBuU]{1,2})?"(?:[^"\\]|\\.)*"|(?:[fFrRbBuU]{1,2})?'(?:[^'\\]|\\.)*']=], "string" },
  },
}
