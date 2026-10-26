//! # Built-in languages
//!
//! **Purpose:** the language definitions that ship with the editor.
//!
//! **Responsibility:** nothing but a list. Each definition is a Lua script in
//! `runtime/syntax/`, compiled into the binary, written in exactly the form a
//! user would put in their own `syntax/` directory — so the built-ins double
//! as the examples, and a user file with the same `name` replaces one.
//!
//! **Public API:** [`BUILTIN`].

/// File name and source of every built-in definition, in the order they are
/// searched. C++ comes before C so that a `.h` shared by both resolves to the
/// definition with the larger keyword set, which degrades more gracefully.
pub const BUILTIN: &[(&str, &str)] = &[
    ("rust.lua", include_str!("../../../runtime/syntax/rust.lua")),
    ("cpp.lua", include_str!("../../../runtime/syntax/cpp.lua")),
    ("c.lua", include_str!("../../../runtime/syntax/c.lua")),
    ("zig.lua", include_str!("../../../runtime/syntax/zig.lua")),
    (
        "python.lua",
        include_str!("../../../runtime/syntax/python.lua"),
    ),
    (
        "markdown.lua",
        include_str!("../../../runtime/syntax/markdown.lua"),
    ),
    ("diff.lua", include_str!("../../../runtime/syntax/diff.lua")),
    (
        "javascript.lua",
        include_str!("../../../runtime/syntax/javascript.lua"),
    ),
    (
        "typescript.lua",
        include_str!("../../../runtime/syntax/typescript.lua"),
    ),
    ("go.lua", include_str!("../../../runtime/syntax/go.lua")),
    ("json.lua", include_str!("../../../runtime/syntax/json.lua")),
    ("toml.lua", include_str!("../../../runtime/syntax/toml.lua")),
    ("yaml.lua", include_str!("../../../runtime/syntax/yaml.lua")),
    ("html.lua", include_str!("../../../runtime/syntax/html.lua")),
    ("css.lua", include_str!("../../../runtime/syntax/css.lua")),
    (
        "shell.lua",
        include_str!("../../../runtime/syntax/shell.lua"),
    ),
    ("lua.lua", include_str!("../../../runtime/syntax/lua.lua")),
    ("java.lua", include_str!("../../../runtime/syntax/java.lua")),
    (
        "makefile.lua",
        include_str!("../../../runtime/syntax/makefile.lua"),
    ),
    (
        "dockerfile.lua",
        include_str!("../../../runtime/syntax/dockerfile.lua"),
    ),
    ("sql.lua", include_str!("../../../runtime/syntax/sql.lua")),
];
