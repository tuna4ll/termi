# termi

[![CI](https://github.com/tuna4ll/termi/actions/workflows/ci.yml/badge.svg)](https://github.com/tuna4ll/termi/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/termi.svg)](https://crates.io/crates/termi)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A modal terminal code editor written in Rust, built on [ratatui] and
[crossterm]. Inspired by Helix and Kilo: modal like the first, small enough to
read end to end like the second.

## Install

Linux and macOS:

```sh
curl -fsSL https://github.com/tuna4ll/termi/releases/latest/download/install.sh | sh
```

Windows PowerShell:

```powershell
irm https://github.com/tuna4ll/termi/releases/latest/download/install.ps1 | iex
```

The installers select the right prebuilt binary and verify its SHA-256 checksum.
Linux and macOS install to `~/.local/bin`; Windows installs under Local AppData
and adds that directory to the user `PATH`.

Alternatively, install from crates.io or download an archive from the
[releases page][releases]:

```sh
cargo install termi
```

Building from source needs Rust 1.88 or newer and a C compiler, for the Lua
that language scripts run on:

```sh
cargo run --release -- src/main.rs
```

## What it does

**Text** — UTF-8 throughout, a [ropey] rope underneath, so editing a large file
costs the same as editing a small one. Tabs, wide glyphs and mixed line endings
are handled at the edges: everything in between works in plain character
indices.

**Modal editing** — normal, insert, visual, visual-line, visual-block,
command and search modes, and vi's whole grammar: counts (`3j`, `5G`),
operators (`d` `c` `y` `>` `<` `gu` `gU` `g~` `gc`) combined with any motion
or text object (`dw`, `c$`, `y3j`, `ci(`, `da"`, `gUiw`, `>ip`, `dit`), `f`
`t` `F` `T` and `;` `,` on the line, `%` between brackets, `{` `}` between
paragraphs, `.` to repeat the last change, named and numbered registers,
macros (`qa` … `q`, `@a`, `@@`), marks within and across files, and a jump
list (`Alt+←` / `Alt+→`). Multiple cursors (`Alt+↑` / `Alt+↓`) are a
first-class part of the editing core rather than a bolted-on mode, and every
operator works at all of them.

**Undo** — a tree rather than a stack: undoing and typing something new starts
a branch instead of throwing the undone edits away. `u` and `Ctrl+R` walk the
current branch; `g-` and `g+` (`:earlier`, `:later`) walk every state the
buffer has been in. Undoing back to the saved text makes the buffer clean
again, and each file's history is kept on disk across restarts.

**Selecting** — `v` and the motions, or the ways a modeless editor does it:
hold Shift and press an arrow, click and drag, or `Ctrl+A` for all of it.
Those leave the mode
alone — you stay in normal or insert mode, the status bar counts what is
selected, and Backspace, Delete, `Ctrl+C`, `Ctrl+X`, `Ctrl+V` or simply typing
act on the selection. A plain motion drops it again.

The two conventions differ where they have to: a selection dragged out with
Shift or the mouse is exclusive, so one press covers one character, while
visual mode stays inclusive of the character under the caret. Each behaves the
way the people who reach for it expect.

**Word-wise keys** — `Ctrl+←` and `Ctrl+→` jump a word at a time,
`Ctrl+Backspace` and `Ctrl+Delete` remove one. A run of spaces or tabs counts
as a word of its own, so a single press clears the gap between two words, or a
line's indentation, without swallowing the word beside it.

**Typing** — indentation carried onto new lines, `}` pulled back to line up
with its opener, and brackets and quotes closed as you type them. Typing the
closing half steps over it instead of doubling it, backspace between the halves
takes both, and Enter between `{` and `}` opens the block out over three
lines. `:set autopairs off` if you would rather type them yourself.

**Search** — incremental, literal or regex, smart case, with matches
highlighted as you type and a history on `↑` `↓`. `*` and `#` search for the
word under the caret. `:s/a/b/[gciI]` takes a range — `%`, `5,9`, `.,+3`,
`'a,$`, and `'<,'>` straight from a selection — asks about each match with
`c`, and is always a single undo step. `Ctrl+G` (or `:grep`) searches the
whole project as you type, and `:greplace/a/b/` replaces across it, leaving
the changed files open and unsaved for review.

**Syntax highlighting** — languages are Lua scripts, not code: 21 ship
built in (Rust, C, C++, Zig, Python, Go, JavaScript, TypeScript, Java and
Kotlin, Lua, shell, SQL, HTML and XML, CSS, JSON, TOML, YAML, Markdown,
Makefiles, Dockerfiles and diffs), and a script of your own adds another or
replaces one — see [Language scripts](#language-scripts). Block-comment state
is cached per line, so scrolling deep into a file does not rescan it, and an
extensionless script is recognised by its `#!` line.

**Language servers** — diagnostics as you type (a sign in the gutter, the
span underlined, the message at the bottom when the caret is on the line,
`]d` `[d` between them), completion, hover (`K`), go to definition (`gd`),
references (`gr`), rename (`F2`, `:rename`) and formatting (`:format`), from
whichever server is installed — see [Language servers](#language-servers).

**Git** — a sign beside every line changed since the last commit, `]c` `[c`
between the changes, `:blame` for the caret's line, the branch in the status
bar, and each changed file marked in the tree.

**Files** — `Ctrl+O` fuzzy-finds project files while respecting `.gitignore`;
the same picker searches buffers, commands and themes. Multiple buffers have a
tab strip, and the lazily expanded file tree (`Ctrl+B`) can create, rename,
copy, move and safely delete entries or reveal hidden files. Atomic saves and a
watcher reload clean buffers when they change on disk and warn rather than
clobber modified ones. The watcher also keeps the tree current, and `Ctrl+B`
re-reads the directory as it opens the panel. `:w name.c` names an unnamed
buffer and highlights it as C from that moment on.

Files open in whatever encoding they are in — UTF-8, UTF-8 with a BOM, or
Latin-1 for anything else — and are written back byte for byte, permissions
included; binary files are refused rather than mangled. `:set ff=dos` and
`:set fenc=latin1` change how a file is written. A file you cannot write opens
read-only: `:w!` tries anyway and `:sudow` writes it through `sudo`. Each
project's `.editorconfig` is honoured, and a file's own indentation is
detected when it opens. Files larger than 16 MB, and lines longer than 10,000
characters, skip highlighting so they stay quick.

**Recovery and sessions** — unsaved changes are written to a swap file every
few seconds; after a crash, reopening the file offers `:recover`. Started
without files, termi reopens the last session in that directory: the files,
the splits and where each caret was.

**Windows** — split the screen as many ways as you like (`Ctrl+W s` / `Ctrl+W
v`). A buffer holds the text, the undo history and the highlighting; a window
holds a viewport and its cursors. The same file can therefore be open in two
windows at once, scrolled to different places, with an edit in one appearing
immediately in the other and a single undo stack behind both.

**Terminal** — `:terminal` opens the platform shell in a new window below the
editor; `:terminal <command>` runs one command there instead. It is a real PTY,
so interactive programs, ANSI colours, alternate screens, resized panes and
bracketed paste work inside Termi. Terminal input goes straight to the child;
use `Ctrl+W` followed by the usual window key to move, split, resize or close
the pane. `Ctrl+W t` opens another terminal. The mouse wheel moves through
10,000 lines of terminal history.

**Looks** — dark and light themes built in, plus TOML themes that override only
the slots you care about, and `transparent_background` to let the terminal's
own background — a colour, an image, transparency — show through. Indentation
folds (`za` `zc` `zo` `zR` `zM`), visible whitespace (`:set list`), indent
guides, a highlighted matching bracket and an optional minimap.

**Command line** — `Tab` completes commands, paths, options and their values,
showing the choices above the line; `↑` `↓` walk the history, which is kept
across sessions.

## Keys

Press `:help` inside the editor for the same list.

| | |
|---|---|
| `i` `a` `I` `A` `o` `O` | enter insert mode |
| `v` `V` `Alt+v` | character-wise / line-wise / block visual mode |
| `h j k l` `w b e` `W B E` `0 ^ $` `gg G` | motions, with a count: `3j`, `5G` |
| `f t F T` `;` `,` `%` `{` `}` `H M L` | find on the line, brackets, paragraphs, screen |
| `d c y > < gu gU g~ gc` + motion or object | operators: `dw` `ci(` `ya"` `gUiw` `gcc` |
| `iw aw` `ip ap` `i( a(` `i{` `i[` `i<` `i"` `i'` `it at` | text objects |
| `.` `q` `@` `m` `'` `` ` `` | repeat, macros, marks |
| `"a` … `"z` `"0` `"1`…`"9` `"_` `"+` | registers |
| `u` `Ctrl+R` `g-` `g+` | undo, redo, back and forward through every branch |
| `za zc zo zR zM` `zt zz zb` | folds, scrolling the caret's line |
| `K` `gd` `gr` `]d` `[d` `F2` `Ctrl+Space` | hover, definition, references, diagnostics, rename, complete |
| `]c` `[c` | next / previous git change |
| `Alt+←` `Alt+→` | back / forward through the jump list |
| `Ctrl+←→` `Ctrl+Home/End` | jump by word, to the start or end of the file |
| `Shift+←↑↓→` `Shift+Home/End` | select; `Ctrl+Shift+←→` by word |
| `Ctrl+A` | select the whole file |
| click, drag | place the caret, select |
| `Backspace` `Delete` | remove the selection, or one character |
| `Ctrl+Backspace` `Ctrl+Del` | remove a whole word, or a run of blanks |
| `Ctrl+C` `Ctrl+X` `Ctrl+V` | copy, cut, paste — the selection, or the line |
| `x` `dd` `yy` `p` `P` `J` `r` `~` | delete, yank, put, join, replace, case |
| `/` `?` `n` `N` `*` `#` | search forwards, backwards, repeat, word under the caret |
| `Ctrl+/` | comment or uncomment the line or selection |
| `Alt+↑` `Alt+↓` `Esc` | add a cursor above/below, collapse to one |
| `Ctrl+B` | file tree |
| `l` `h` (in the tree) | open a folder or file / close a folder, or go to the one holding it |
| `Backspace` `-` (in the tree) | show the directory above the tree's root |
| `a` `A` (in the tree) | create a file / directory at the selection |
| `r` `c` `m` (in the tree) | rename / copy / move the selection |
| `d` `.` (in the tree) | delete with confirmation / toggle hidden files |
| `Ctrl+O` | fuzzy-find a project file |
| `Ctrl+G` | search the whole project |
| `Ctrl+N` `Ctrl+P` | next / previous buffer |
| `Ctrl+W` `s` `v` | split the window across / down |
| `Ctrl+W` `h j k l` `w` | move the focus between windows |
| `Ctrl+W` `c` `o` | close this window / close all the others |
| `Ctrl+W` `t` | open a terminal window |
| `Ctrl+W` `+` `-` `<` `>` `=` | resize windows, or even them up |
| `:terminal [command]` | open a shell or command in a new terminal window |
| `Ctrl+S` `Ctrl+Q` | save, quit |
| `:` | command line |

Commands: `:w[!] [path]` `:q[!]` `:wq` `:wa` `:qa[!]` `:wqa` `:sudow`
`:e[!] path` `:touch path` `:mkdir path` `:find` `:buffers` `:commands`
`:themes` `:bn` `:bp` `:<line>` `:sp` `:vs` `:clo` `:on`
`:set[!] <option> [value]` `:theme <name>` `:terminal [command]`
`:[range]s/pattern/replacement/[gciI]` `:grep [text]` `:greplace/a/b/`
`:earlier [n]` `:later [n]` `:recover` `:dropswap` `:blame` `:format`
`:rename <name>` `:diagnostics` `:lsp [restart]` `:config [reload]`
`:help [topic]`

On the command line: `termi file.rs:42:7`, `termi +42 file.rs`, `termi -R`
(read-only), `termi --config path`, `termi --clean`, and `command | termi -`.

## Configuration

See [`config.example.toml`](config.example.toml), or run `:config` to open your
own — a commented copy of the example is created the first time. `:set!`
changes a setting and writes it there too, keeping the file's comments;
`:config reload` reads it again.

Keys are remapped per mode, in the notation the help uses. A mapping names an
action (`:help` lists them), runs a command, or types other keys:

```toml
[keys.normal]
"<Space>f" = "file_picker"
"<Space>w" = ":w"
"<C-o>" = "jump_back"
Q = "@q"

[keys.insert]
jk = "normal_mode"
```

Settings can differ per language, between the global ones and each file's own
`.editorconfig`:

```toml
[languages.go]
expand_tabs = false

[languages.python]
tab_width = 4
formatter = "black --quiet -"
language_server = { command = "pyright-langserver", args = ["--stdio"] }
```

Themes go in `<config-dir>/termi/themes/<name>.toml` and layer over a built-in
base; `transparent = true` in a theme drops its background:

```toml
name = "midnight"
base = "dark"

[comment]
fg = "#4a5058"
italic = true

[selection]
bg = "bright-blue"
```

## Language scripts

A language is a Lua script that returns a table. Put one in
`<config-dir>/termi/syntax/` to add a language, or give it the `name` of a
built-in one to replace it; the built-in scripts in
[`runtime/syntax/`](runtime/syntax) are the examples. Patterns are Rust
regular expressions, written in `[=[ ]=]` so a `]]` inside them is harmless:

```lua
return {
  name = "ini",
  extensions = { "ini", "cfg" },
  line_comment = ";",
  prose = true,                        -- no generic code rules
  rules = {
    { [=[^\s*\[[^\]]*\]]=], "heading" },
    { [=[^\s*[\w.-]+\s*=]=], "function" },
  },
}
```

Word lists (`keywords`, `types`, `constants`), `block_comment`,
`nested_comments`, `filenames` and `shebangs` are there for languages that want
them. For anything rules cannot express, a script can take over entirely with
`highlight = function(line, state) … return spans, state end`, where `state`
is an integer carried from one line to the next. Scripts run in a sandbox with
only the `string`, `table`, `math` and `utf8` libraries, and a `highlight`
function that runs too long is stopped rather than allowed to freeze the
editor. A script that fails to load is reported in the status bar.

## Language servers

termi starts a language server the first time a file in its language opens,
when the program is installed: rust-analyzer, clangd, pylsp, gopls,
typescript-language-server, zls, lua-language-server, bash-language-server,
the vscode JSON, HTML and CSS servers, yaml-language-server, taplo and
marksman. `[languages.<name>] language_server` picks another, and `lsp =
false` turns them off. `:lsp` shows what is running and why anything is not.

Completion opens as you type an identifier (`auto_complete`) or on
`Ctrl+Space`; `Tab` or `↓` move through it, `Enter` accepts, `Esc` closes it.
Renames and formatting are single undo steps per file, and a rename leaves the
other files it changed open and unsaved, for review before `:wa`.

## Architecture

Layers depend downwards only:

```
app/         event loop, state, dispatch, ex commands, sessions, lsp glue
├── ui/      layout and widgets; renderer/ owns the terminal
├── input/   keys → actions: vi grammar, key notation, user mappings
└── editor/  text, with no knowledge of terminals
    ├── document/   rope, file, dirty state, indentation
    ├── cursor/     positions, motions, word boundaries
    ├── selection/  character ranges
    ├── buffer/     a file: document + history + highlighting
    ├── window/     a view: viewport + cursors, and the split tree
    ├── edit/       changes a buffer's text through a window
    ├── textobject/ the spans iw, a(, it and friends name
    ├── fold/       which lines a closed fold hides
    └── command/    ex-command parsing and line ranges

terminal/  PTY process + ANSI screen model
lsp/       language server protocol: transport, client, messages
syntax/    highlight engine and Lua language scripts (runtime/syntax/)
git/       signs, status, branch and blame from the git binary
config/  theme/  search/  undo/  clipboard/  filesystem/
```

The editor core is UI-free and the UI layer is read-only, so a render pass is a
pure function of the state plus the terminal size. Every module's header
documents its purpose, its responsibility and its public API.

`unsafe_code` is forbidden crate-wide.

The editor is a library with a thin binary on top, which is what lets the
integration tests, benchmarks and fuzz targets drive exactly the code the
terminal does.

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all
cargo bench                       # timings on a 100,000-line file
cargo +nightly fuzz run keys      # or highlight, decode
```

CI runs the first three on Linux, macOS and Windows, builds against the
minimum supported Rust version, and checks that the benchmarks and fuzz
targets still compile. See [CONTRIBUTING.md] for what the code expects of a
change.

## License

MIT

[ratatui]: https://ratatui.rs
[crossterm]: https://github.com/crossterm-rs/crossterm
[ropey]: https://github.com/cessen/ropey
[releases]: https://github.com/tuna4ll/termi/releases
[CONTRIBUTING.md]: CONTRIBUTING.md
