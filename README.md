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

Building from source needs Rust 1.88 or newer:

```sh
cargo run --release -- src/main.rs
```

## What it does

**Text** — UTF-8 throughout, a [ropey] rope underneath, so editing a large file
costs the same as editing a small one. Tabs, wide glyphs and mixed line endings
are handled at the edges: everything in between works in plain character
indices.

**Modal editing** — normal, insert, visual, visual-line, command and search
modes. `hjkl` and word motions, `dd`/`yy`/`p`, undo and redo with typing merged
into sensible steps, and multiple cursors (`Alt+↑` / `Alt+↓`) as a first-class
part of the editing core rather than a bolted-on mode.

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

**Search** — incremental, literal or regex, smart case, with matches highlighted
as you type and `:%s/a/b/g` for replacement.

**Syntax highlighting** — regex based, for Rust, C, C++, Zig, Python,
Markdown, and diffs. Block-comment state is cached per line, so scrolling deep
into a file does not rescan it. A `.diff`, `.patch` or `.rej` file is coloured
the way a diff wants to be read: added lines green, removed lines red, hunk
headers and file headers apart from both.

**Files** — multiple buffers with a tab strip, a lazily expanded file tree
(`Ctrl+B`) that can create files and directories, atomic saves, and a watcher
that reloads clean buffers when they change on disk and warns rather than
clobbers when they do not.

**Windows** — split the screen as many ways as you like (`Ctrl+W s` / `Ctrl+W
v`). A buffer holds the text, the undo history and the highlighting; a window
holds a viewport and its cursors. The same file can therefore be open in two
windows at once, scrolled to different places, with an edit in one appearing
immediately in the other and a single undo stack behind both.

**Looks** — dark and light themes built in, plus TOML themes that override only
the slots you care about.

## Keys

Press `:help` inside the editor for the same list.

| | |
|---|---|
| `i` `a` `I` `A` `o` `O` | enter insert mode |
| `v` `V` | character-wise / line-wise visual mode |
| `h j k l` `w b e` `0 ^ $` `gg G` | motions |
| `Ctrl+←→` `Ctrl+Home/End` | jump by word, to the start or end of the file |
| `Shift+←↑↓→` `Shift+Home/End` | select; `Ctrl+Shift+←→` by word |
| `Ctrl+A` | select the whole file |
| click, drag | place the caret, select |
| `Backspace` `Delete` | remove the selection, or one character |
| `Ctrl+Backspace` `Ctrl+Del` | remove a whole word, or a run of blanks |
| `Ctrl+C` `Ctrl+X` `Ctrl+V` | copy, cut, paste — the selection, or the line |
| `x` `dd` `yy` `p` `u` `Ctrl+R` | delete, yank, paste, undo, redo |
| `/` `?` `n` `N` | search forwards, backwards, repeat |
| `Alt+↑` `Alt+↓` `Esc` | add a cursor above/below, collapse to one |
| `Ctrl+B` | file tree |
| `a` `A` (in the tree) | create a file / directory at the selection |
| `Ctrl+N` `Ctrl+P` | next / previous buffer |
| `Ctrl+W` `s` `v` | split the window across / down |
| `Ctrl+W` `h j k l` `w` | move the focus between windows |
| `Ctrl+W` `c` `o` | close this window / close all the others |
| `Ctrl+W` `+` `-` `<` `>` `=` | resize windows, or even them up |
| `Ctrl+S` `Ctrl+Q` | save, quit |
| `:` | command line |

Commands: `:w [path]` `:q[!]` `:wq` `:e[!] path` `:touch path` `:mkdir path`
`:bn` `:bp` `:<line>` `:sp` `:vs` `:clo` `:on` `:set <option> [value]`
`:theme <name>` `:%s/pattern/replacement/g`

## Configuration

See [`config.example.toml`](config.example.toml). Themes go in
`<config-dir>/termi/themes/<name>.toml` and layer over a built-in base:

```toml
name = "midnight"
base = "dark"

[comment]
fg = "#4a5058"
italic = true

[selection]
bg = "bright-blue"
```

## Architecture

Layers depend downwards only:

```
app/         event loop, state, action dispatch, ex commands
├── ui/      layout and widgets; renderer/ owns the terminal
├── input/   keys → actions
└── editor/  text, with no knowledge of terminals
    ├── document/   rope, file, dirty state, indentation
    ├── cursor/     positions, motions, word boundaries
    ├── selection/  character ranges
    ├── buffer/     a file: document + history + highlighting
    ├── window/     a view: viewport + cursors, and the split tree
    ├── edit/       changes a buffer's text through a window
    └── command/    ex-command parsing

config/  theme/  syntax/  search/  undo/  clipboard/  filesystem/
```

The editor core is UI-free and the UI layer is read-only, so a render pass is a
pure function of the state plus the terminal size. Every module's header
documents its purpose, its responsibility and its public API.

`unsafe_code` is forbidden crate-wide.

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

CI runs these plus the test suite on Linux, macOS and Windows and a build
against the minimum supported Rust version. See [CONTRIBUTING.md] for what the
code expects of a change.

## Roadmap

Syntax highlighting is deliberately regex based for now. The next step is
tree-sitter behind the same `Highlight` span interface, which the renderer and
themes already consume — no changes above the `syntax` module.

## License

MIT

[ratatui]: https://ratatui.rs
[crossterm]: https://github.com/crossterm-rs/crossterm
[ropey]: https://github.com/cessen/ropey
[releases]: https://github.com/tuna4ll/termi/releases
[CONTRIBUTING.md]: CONTRIBUTING.md
