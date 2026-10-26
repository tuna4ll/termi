//! # Language scripts
//!
//! **Purpose:** read a language definition out of a Lua script.
//!
//! **Responsibility:** run the script in a small sandbox, turn the table it
//! returns into a [`Language`], and — when the script asks for complete
//! control — keep its `highlight` function to call once per line.
//!
//! A script returns a table. Every field is optional except `name`:
//!
//! ```lua
//! return {
//!   name = "toml",
//!   extensions = { "toml" },          -- matched without the dot
//!   filenames = { "cargo.lock" },     -- matched case-insensitively
//!   shebangs = { "python3" },         -- interpreters named on a `#!` line
//!   keywords = words "if else for",   -- `words` splits on whitespace
//!   types = { "u8", "String" },
//!   constants = { "true", "false" },
//!   line_comment = "#",
//!   block_comment = { "/*", "*/" },
//!   nested_comments = false,
//!   macro_suffix = false,             -- `name!` is a macro call
//!   capitalised_types = false,        -- `Name` is a type
//!   prose = false,                    -- turn the generic code rules off
//!   rules = {                         -- tried first, in order
//!     { [=[^\s*\[[^\]]*\]]=], "heading" },
//!   },
//!   -- Optional, replaces everything above: called with each line and the
//!   -- integer state the previous line left behind; returns spans
//!   -- { first_byte, last_byte, kind } (1-based, inclusive, as
//!   -- `string.find` reports them) and the state for the next line.
//!   highlight = function(line, state) return {}, state end,
//! }
//! ```
//!
//! Patterns are Rust regular expressions, not Lua patterns — and because a
//! regex often contains `]]`, which ends a plain `[[...]]` Lua string, write
//! them as `[=[...]=]`. `kind` is any
//! syntax slot a theme can colour: `keyword`, `type`, `function`, `string`,
//! `number`, `comment`, `constant`, `operator`, `punctuation`, `attribute`,
//! `macro`, `heading`, `emphasis`, `link` and the four `diff_*` slots.
//!
//! **Public API:** [`Script`], [`CustomHighlight`], [`load`].

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};

use mlua::{Function, HookTriggers, Lua, LuaOptions, StdLib, Table, Value, VmState};

use super::{HighlightKind, Language};

/// How many thousand Lua instructions one call of a `highlight` function may
/// run before it is stopped. A runaway script must cost a dropped highlight,
/// not a frozen editor.
const INSTRUCTION_BUDGET: u32 = 2_000;

/// A loaded definition: the language, and its own highlighter if it has one.
pub struct Script {
    /// What the script described.
    pub language: Language,
    /// The script's `highlight` function, when it took over highlighting.
    pub custom: Option<CustomHighlight>,
}

/// A `highlight` function, and the Lua state it lives in.
pub struct CustomHighlight {
    lua: Lua,
    function: Function,
    spent: std::sync::Arc<AtomicU32>,
}

impl std::fmt::Debug for CustomHighlight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CustomHighlight")
    }
}

/// A span in byte offsets, as the engine works before converting.
pub type ByteSpan = (usize, usize, HighlightKind);

impl CustomHighlight {
    /// Run the function over one line.
    ///
    /// Returns `None` when the script fails or overruns its budget; the line
    /// is then drawn without highlighting rather than not at all.
    #[must_use]
    pub fn call(&self, line: &str, state: i64) -> Option<(Vec<ByteSpan>, i64)> {
        self.spent.store(0, Ordering::Relaxed);
        let (spans, next): (Option<Table>, Option<i64>) = self.function.call((line, state)).ok()?;
        let mut result = Vec::new();
        if let Some(spans) = spans {
            for span in spans.sequence_values::<Table>().flatten() {
                let (Ok(first), Ok(last), Ok(kind)) = (
                    span.get::<usize>(1),
                    span.get::<usize>(2),
                    span.get::<String>(3),
                ) else {
                    continue;
                };
                let Some(kind) = HighlightKind::from_name(&kind) else {
                    continue;
                };
                let start = first.saturating_sub(1).min(line.len());
                let end = last.min(line.len());
                if start < end && line.is_char_boundary(start) && line.is_char_boundary(end) {
                    result.push((start, end, kind));
                }
            }
        }
        // The Lua state is only kept alive to own the function.
        let _ = &self.lua;
        Some((result, next.unwrap_or(0)))
    }
}

/// A fresh sandbox: strings, tables, maths and UTF-8, and nothing that can
/// reach the filesystem or the network. Language definitions describe text;
/// they have no business doing anything else.
fn sandbox() -> mlua::Result<Lua> {
    let lua = Lua::new_with(
        StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8,
        LuaOptions::new(),
    )?;
    let words = lua.create_function(|_, text: String| {
        Ok(text
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>())
    })?;
    lua.globals().set("words", words)?;
    let termi = lua.create_table()?;
    termi.set("version", env!("CARGO_PKG_VERSION"))?;
    lua.globals().set("termi", termi)?;
    Ok(lua)
}

/// Run `source` and read the language it returns.
///
/// # Errors
/// Returns a message naming `origin` when the script fails, returns something
/// other than a table, or describes the language wrongly.
pub fn load(source: &str, origin: &str) -> Result<Script, String> {
    let fail = |error: mlua::Error| format!("{origin}: {}", first_line(&error.to_string()));
    let lua = sandbox().map_err(fail)?;
    let value: Value = lua.load(source).set_name(origin).eval().map_err(fail)?;
    let Value::Table(table) = value else {
        return Err(format!("{origin}: the script must return a table"));
    };

    let name: String = table
        .get::<Option<String>>("name")
        .map_err(fail)?
        .ok_or_else(|| format!("{origin}: a language needs a name"))?;
    let strings = |key: &str| -> Result<Vec<String>, String> {
        Ok(table
            .get::<Option<Vec<String>>>(key)
            .map_err(|error| format!("{origin}: {key}: {}", first_line(&error.to_string())))?
            .unwrap_or_default())
    };
    let set =
        |key: &str| -> Result<HashSet<String>, String> { Ok(strings(key)?.into_iter().collect()) };
    let flag = |key: &str| -> Result<bool, String> {
        Ok(table
            .get::<Option<bool>>(key)
            .map_err(|error| format!("{origin}: {key}: {}", first_line(&error.to_string())))?
            .unwrap_or(false))
    };

    let block_comment = match table
        .get::<Option<Vec<String>>>("block_comment")
        .map_err(fail)?
    {
        None => None,
        Some(pair) if pair.len() == 2 => Some((pair[0].clone(), pair[1].clone())),
        Some(_) => {
            return Err(format!(
                "{origin}: block_comment takes an opener and a closer"
            ));
        }
    };

    let mut extra_rules = Vec::new();
    if let Some(rules) = table.get::<Option<Table>>("rules").map_err(fail)? {
        for (index, rule) in rules.sequence_values::<Table>().enumerate() {
            let rule = rule.map_err(fail)?;
            let pattern: String = rule.get(1).map_err(fail)?;
            let kind_name: String = rule.get(2).map_err(fail)?;
            let kind = HighlightKind::from_name(&kind_name).ok_or_else(|| {
                format!("{origin}: rule {}: unknown kind {kind_name:?}", index + 1)
            })?;
            extra_rules.push((pattern, kind));
        }
    }

    let language = Language {
        name,
        extensions: strings("extensions")?
            .into_iter()
            .map(|extension| extension.trim_start_matches('.').to_ascii_lowercase())
            .collect(),
        filenames: strings("filenames")?
            .into_iter()
            .map(|name| name.to_ascii_lowercase())
            .collect(),
        shebangs: strings("shebangs")?,
        keywords: set("keywords")?,
        types: set("types")?,
        constants: set("constants")?,
        line_comment: table.get::<Option<String>>("line_comment").map_err(fail)?,
        block_comment,
        nested_block_comments: flag("nested_comments")?,
        macro_suffix: flag("macro_suffix")?,
        capitalised_types: flag("capitalised_types")?,
        extra_rules,
        prose: flag("prose")?,
        origin: origin.to_string(),
    };

    let custom = match table.get::<Option<Function>>("highlight").map_err(fail)? {
        Some(function) => {
            let spent = std::sync::Arc::new(AtomicU32::new(0));
            let counter = spent.clone();
            lua.set_hook(
                HookTriggers::new().every_nth_instruction(1_000),
                move |_, _| {
                    if counter.fetch_add(1, Ordering::Relaxed) > INSTRUCTION_BUDGET {
                        return Err(mlua::Error::runtime("highlight ran too long"));
                    }
                    Ok(VmState::Continue)
                },
            )
            .map_err(fail)?;
            Some(CustomHighlight {
                lua,
                function,
                spent,
            })
        }
        None => None,
    };
    Ok(Script { language, custom })
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_becomes_a_language() {
        let script = load(
            r##"return { name = "demo", extensions = { ".DM" }, keywords = words "if else",
               line_comment = "#", rules = { { "^%%.*", "attribute" } } }"##,
            "demo.lua",
        )
        .expect("valid script");
        assert_eq!(script.language.name, "demo");
        assert_eq!(script.language.extensions, vec!["dm".to_string()]);
        assert!(script.language.keywords.contains("else"));
        assert_eq!(script.language.extra_rules[0].1, HighlightKind::Attribute);
        assert!(script.custom.is_none());
    }

    #[test]
    fn mistakes_name_the_script() {
        let error = load("return 4", "bad.lua").err().expect("not a table");
        assert!(error.starts_with("bad.lua"), "{error}");
        let error = load(
            r#"return { name = "x", rules = { { "a", "colour" } } }"#,
            "kind.lua",
        )
        .err()
        .expect("unknown kind");
        assert!(error.contains("colour"), "{error}");
        assert!(load("return {}", "nameless.lua").is_err());
        assert!(load("this is not lua", "syntax.lua").is_err());
    }

    #[test]
    fn the_sandbox_has_no_way_out() {
        assert!(load(r#"os.execute("true") return { name = "x" }"#, "os.lua").is_err());
        assert!(load(r#"io.open("x") return { name = "x" }"#, "io.lua").is_err());
    }

    #[test]
    fn a_highlight_function_reports_spans_and_state() {
        let script = load(
            r#"return { name = "upper", highlight = function(line, state)
                 local spans = {}
                 local first, last = line:find("%u+")
                 if first then spans[1] = { first, last, "keyword" } end
                 return spans, state + 1
               end }"#,
            "upper.lua",
        )
        .expect("valid script");
        let custom = script.custom.expect("custom highlighter");
        let (spans, next) = custom.call("ab CD ef", 4).expect("call succeeds");
        assert_eq!(spans, vec![(3, 5, HighlightKind::Keyword)]);
        assert_eq!(next, 5);
    }

    #[test]
    fn a_runaway_highlight_function_is_stopped() {
        let script = load(
            r#"return { name = "loop", highlight = function() while true do end end }"#,
            "loop.lua",
        )
        .expect("valid script");
        assert!(script.custom.expect("custom").call("x", 0).is_none());
    }
}
