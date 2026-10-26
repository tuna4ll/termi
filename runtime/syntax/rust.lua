-- Rust.
--
-- The extra rules exist because the generic scanner would get three things
-- wrong: attributes look like comments-that-are-not, raw strings ignore
-- backslash escapes, and a lifetime 'a looks like the start of a character
-- literal. Putting the character-literal pattern before the lifetime pattern
-- is what tells 'a' from 'a.
return {
  name = "rust",
  extensions = { "rs" },
  keywords = words [[
    as async await break const continue crate dyn else enum extern fn for if
    impl in let loop match mod move mut pub ref return self static struct super
    trait type union unsafe use where while yield
  ]],
  types = words [[
    bool char f32 f64 i8 i16 i32 i64 i128 isize str u8 u16 u32 u64 u128 usize
    String Vec Option Result Box Rc Arc Cow HashMap HashSet BTreeMap VecDeque Self
  ]],
  constants = words "true false None Some Ok Err",
  line_comment = "//",
  block_comment = { "/*", "*/" },
  nested_comments = true,
  macro_suffix = true,
  capitalised_types = true,
  rules = {
    -- Doc comments before ordinary ones, so they can be told apart.
    { [=[///.*|//!.*]=], "comment" },
    { [=[#!?\[[^\]]*\]?]=], "attribute" },
    -- Raw strings: no escape processing, any number of hashes.
    { [=[r#*"[^"]*"#*]=], "string" },
    { [=[b"(?:[^"\\]|\\.)*"]=], "string" },
    -- Character literal, ahead of the lifetime rule below.
    { [=[b?'(?:[^'\\]|\\.)']=], "string" },
    { [=['(?:static|_|[a-z][a-z0-9_]*)]=], "type" },
  },
}
