# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-10-29

### Added

- **vi's grammar** — counts, the `d` `c` `y` `>` `<` `gu` `gU` `g~` and `gc`
  operators with any motion or text object, text objects for words, WORDs,
  paragraphs, brackets, quotes and tags, `f` `t` `F` `T` with `;` and `,`, `%`,
  `{` `}`, `W` `B` `E`, `H` `M` `L`, `zt` `zz` `zb`, `r`, `~`, `J`, `gJ`, `P`,
  `x` `X` `s` `S` `C` `D` `Y`, `*` and `#`, and `.` to repeat the last change.
- **Registers, macros, marks and jumps** — `"a`–`"z` (upper case appends), the
  yank register `"0`, the delete history `"1`–`"9`, `"_`, `"+`, `"%`, `":` and
  `"/`; macros recorded with `q` and played with `@`, `@@` and `@:`; marks
  `a`–`z` per file and `A`–`Z` across files; a jump list on `Alt+←`/`Alt+→`.
- **Visual block mode** on `Alt+v`, with `I` and `A` typing on every line and
  block-wise yanks that put back as rectangles.
- **An undo tree** — undoing and typing something new starts a branch rather
  than losing the undone edits; `g-` `g+`, `:earlier` and `:later` walk every
  state. Undoing back to the saved text makes the buffer clean, and each file's
  history is kept on disk across restarts (`persistent_undo`, `undo_limit`).
- **Languages as Lua scripts** — the highlighter has no languages compiled into
  it. The built-in ones are scripts in `runtime/syntax/`, and a script in
  `<config-dir>/termi/syntax/` adds a language or replaces one, either as rules
  or with a `highlight` function of its own. Fourteen new languages: Go,
  JavaScript, TypeScript, Java and Kotlin, Lua, shell, SQL, HTML and XML, CSS,
  JSON, TOML, YAML, Makefiles and Dockerfiles. Extensionless scripts are
  recognised by their `#!` line.
- **Language servers** — diagnostics, completion, hover (`K`), definitions
  (`gd`), references (`gr`), rename (`F2`, `:rename`) and formatting
  (`:format`, or a configured `formatter`), from the usual server for each
  language when it is installed, or the one `[languages.<name>]` names.
- **Git** — change signs in a new sign column, `]c` `[c`, `:blame`, the branch
  in the status bar and file status in the tree.
- **Project search** — `Ctrl+G` and `:grep` search every file as you type;
  `:greplace/a/b/` replaces across the project and leaves the files unsaved.
- **Ranged and confirmed substitution** — `:5,9s`, `:'<,'>s` (`:` from a
  selection fills it in), `:.,+3s`, `:'a,$s`, and the `c`, `i` and `I` flags.
- **Recovery** — swap files for modified buffers, `:recover` and `:dropswap`.
- **Files** — UTF-8 with a BOM and Latin-1 open and save byte for byte,
  `:set fileformat` and `:set encoding` change how a file is written, binary
  files are refused, read-only files need `:w!`, `:sudow` writes through sudo,
  `.editorconfig` is honoured, indentation is detected, `insert_final_newline`,
  and large files and very long lines skip highlighting.
- **Configuration** — key mappings per mode in `[keys.normal]`, `[keys.insert]`
  and `[keys.visual]`; settings per language in `[languages.<name>]`; `:set!`
  saves a setting to the config file without disturbing its comments;
  `:config` and `:config reload`; `transparent_background`.
- **Sessions** — started without files, termi reopens the last session in the
  directory, splits and carets included.
- **Command line** — Tab completion of commands, paths, options and values,
  history on `↑` `↓` for commands and searches, kept across sessions; `:wa`,
  `:qa`, `:wqa`, `:wq <path>`; quitting names the unsaved files.
- **Display** — indentation folds (`za` `zc` `zo` `zR` `zM`), `:set list`,
  indent guides, matching-bracket highlight, a braille minimap, and the exit
  code of a finished terminal command. Comment toggling on `gc` and `Ctrl+/`.
- **Startup options** — `file:line:column`, `+line`, `-R`, `--config`,
  `--clean`, `--no-session` and `-` for standard input.
- `:help [topic]` opens the reference as a searchable read-only buffer.
- In the file tree, `h` closes a folder or steps out to the one holding the
  selection, and `Backspace` shows the directory above the tree's root.

### Changed

- `c` in visual mode changes the selection, as in vi; `Ctrl+X` still cuts.
- `↑` and `↓` in the search prompt walk the search history; `Ctrl+N` and
  `Ctrl+P` still step through matches.
- The gutter has a sign column; `sign_column = false` takes it away.
- Saving keeps the permissions of the file it replaces.
- The editor is now a library with a thin binary, and has integration,
  property and fuzz tests and benchmarks.

### Fixed

- Undo after reloading a file that changed on disk replayed old edits into the
  new text.
- `:s` changes could not be undone.
- With several cursors, an edit that added or removed a line at one of them
  left the cursors after it on the wrong line.
- A highlighting rule with capture groups of its own shifted every rule after
  it, and could crash on some lines.
- Language server answers about a file reached through a symlink — every
  temporary directory on macOS — and every file on Windows never found their
  buffer, so diagnostics, renames and jumps went nowhere.

## [0.1.8] - 2026-09-17

### Added

- A reusable fuzzy picker for project files, open buffers, commands and themes.
  Project discovery respects `.gitignore`, and recently opened files are shown
  first until a query is entered.
- The file tree can rename, copy, move and permanently delete entries with an
  explicit confirmation. It can also toggle hidden files, and newly created
  files open immediately.

## [0.1.7] - 2026-09-09

### Added

- **Embedded terminal windows** — `:terminal` opens the platform shell below
  the editor, `Ctrl+W t` provides a direct shortcut, and `:terminal <command>`
  runs a command in the same kind of pane.
  Each session uses a real pseudo-terminal, renders ANSI colours and text
  attributes, follows window resizes, supports application cursor mode and
  bracketed paste, and keeps 10,000 lines of mouse-scrollable history.
  `Ctrl+W` remains the escape into Termi's existing focus, split, resize and
  close commands while the child process owns every other key.

## [0.1.6] - 2026-09-08

### Added

- **Diff and patch highlighting** — `.diff`, `.patch` and `.rej` files are
  read as diffs: added lines green, removed lines red, hunk headers picked out,
  and file headers, mode and index lines set apart from the text they describe.
  `--- a/file` and `+++ b/file` are headers rather than a removed and an added
  line, and the older `diff` output — a `3,4c3,4` range with `<` and `>` lines —
  is coloured the same way. The four colours are theme slots like any other:
  `diff_added`, `diff_removed`, `diff_hunk` and `diff_meta`.

### Fixed

- **`:w <path>` now picks the language up.** Writing an unnamed buffer to
  `hello.c` named the buffer but left it unhighlighted until it was reopened;
  the highlighter is chosen again whenever a write renames the buffer.
- **The file tree notices changes made outside the editor.** The watcher now
  covers the directories the tree has expanded as well as the files that are
  open, so a file added or removed by another program appears or disappears
  where it belongs, with the highlighted row following its own file rather than
  its old position. `Ctrl+B` also re-reads the tree as it opens the panel, which
  covers the case where the watcher is off or never saw the change.

## [0.1.5] - 2026-09-07

### Added

- **Word-wise deletion** — `Ctrl+Backspace` and `Ctrl+Delete` remove a whole
  word, at every cursor and as one undo step. A run of spaces or tabs is a unit
  of its own, so one press clears the gap between two words or a line's
  indentation without also taking the word beside it; at the edge of a line the
  chord joins it to its neighbour instead of crossing into another word.
- **Word-wise motion** — `Ctrl+←` and `Ctrl+→` jump a word at a time, and
  `Ctrl+Home` / `Ctrl+End` reach the start and end of the file. Adding Shift
  selects the same span, which `Ctrl+Shift+←/→` already did.
- Terminals that report `Ctrl+Backspace` as a bare `0x08` — which arrives as
  `Ctrl+H` — get the same binding, so the chord works without the kitty
  keyboard protocol.

## [0.1.4] - 2026-09-06

### Added

- **Selecting with Shift and the arrow keys** — Shift with an arrow, `Home` or
  `End` starts a selection and extends it; `Ctrl+Shift+←/→` moves by word. The
  mode does not change: the selection lives in normal or insert mode, is painted
  there, and a plain motion drops it. Backspace and Delete remove it, typing
  replaces it, and pasting over it swaps it out.
- **Selecting with the mouse** — a click now places the caret on the character
  under the pointer as well as focusing the window, and dragging selects. A drag
  that leaves the window keeps selecting along the edge it left by. Tabs, wide
  glyphs and wrapped lines all map back to the right character.
- `Ctrl+A` selects the whole buffer, collapsing to one cursor and reaching the
  final character.
- `Ctrl+C`, `Ctrl+X` and `Ctrl+V` copy, cut and paste. With a selection they act
  on it; with none they fall back to the current line, as `yy` and `dd` do.
- The status bar counts the selected characters, and the caret is drawn as a bar
  while a selection stands — the mode badge cannot report either, because these
  selections are not a mode.
- **Automatic bracket and quote closing** — typing `(`, `[`, `{`, `"`, `'` or
  `` ` `` inserts the closing half and leaves the caret between the two. Typing
  the closing half steps over it instead of doubling it, backspace between the
  halves removes both, and Enter between them opens the block out over three
  lines. Brackets are not closed in front of a word, and a quote after one is
  left alone so apostrophes still work. `auto_pairs = false`, or
  `:set autopairs off`, turns it off.

### Changed

- Clicking in a window now places the caret as well as moving the focus. The
  wheel still scrolls whichever window is under the pointer without focusing it.
- `d`, `y` and the clipboard chords act on a standing selection when there is
  one, and fall back to the current line when there is not.
- `i` drops a standing selection rather than keeping it alive unseen; replacing
  a selection is what typing over it does.
- A character-wise copy reports characters rather than lines, and says "copied"
  where it used to say "yanked".

## [0.1.3] - 2026-09-03

### Added

- One-line installers now select and verify prebuilt binaries for Linux,
  macOS and Windows. Release builds publish the installers beside the archives.
- Files and directories can be created from the file tree with `a` and `A`, or
  from the command line with `:touch` and `:mkdir`. Existing paths are never
  overwritten.

## [0.1.2] - 2026-08-11

### Added

- **Split windows** — the screen can be divided any number of ways, horizontally
  and vertically, with `Ctrl+W` followed by `s`, `v`, `c`, `o`, `w`, a motion key
  or one of `+ - < > =`. `:split`, `:vsplit`, `:close` and `:only` do the same
  from the command line, and `:q` now closes the focused window before it
  closes the buffer.
- The same file can be open in several windows at once, each scrolled where it
  likes and carrying its own cursors, while sharing one text and one undo
  history — an edit in one window shows up in the others as it is typed.
- Each window remembers where it was in every buffer it has shown, so switching
  files and coming back returns to the same line rather than the top.
- **Mouse** — clicking a window focuses it and the wheel scrolls whichever
  window is under the pointer. Set `mouse = false` to leave the mouse to the
  terminal.

### Changed

- A buffer is now the file — its text, undo history and highlighting — and a
  window is the view onto one. Cursors and the scroll position moved from the
  first to the second.

### Fixed

- `:%s/…/…/g` left the syntax highlighter holding state derived from the text it
  had just replaced, so colours below the substitution could be wrong until the
  file was reloaded.
- Scrolling without moving the caret (`Ctrl+E`, and now the wheel) snapped
  straight back on the next frame once the caret left the window. The caret is
  now carried along at the edge instead.

## [0.1.1] - 2026-08-08

### Fixed

- Characters that need AltGr — `#`, `$`, `{`, `}`, `[`, `]`, `\`, `@`, `€` and
  the rest, depending on the layout — could not be typed on Windows. The console
  reports AltGr as Ctrl+Alt alongside the character the layout produced, so every
  keymap rejected the key as a modified one and dropped it. The same characters
  displayed correctly when loaded from a file, which made the bug look like a
  rendering problem rather than an input one.

## [0.1.0] - 2026-07-26

First release.

### Added

- **Text** — UTF-8 throughout, backed by a rope, so edit cost does not grow with
  file size. Tabs, double-width glyphs and mixed line endings are normalised at
  the edges.
- **Modal editing** — normal, insert, visual, visual-line, command and search
  modes, with vi-style motions and operators.
- **Multiple cursors** — modelled in the editing core rather than bolted on;
  `Alt+↑` / `Alt+↓` to add, `Esc` to collapse.
- **Undo** — inverse-operation history with consecutive typing merged into one
  step.
- **Search** — incremental, literal or regex, smart case, matches highlighted
  while typing, and `:%s/pattern/replacement/g`.
- **Syntax highlighting** — regex based, for Rust, C, C++, Zig, Python and
  Markdown, with per-line block-comment state cached so deep scrolling does not
  rescan the file.
- **Files** — multiple buffers with a tab strip, a lazily expanded file tree,
  atomic saves, and a watcher that reloads clean buffers and warns on dirty
  ones.
- **Theming** — dark and light built in, plus TOML themes that override only the
  slots they name.
- **Configuration** — TOML, with every key optional and unknown keys reported.

[Unreleased]: https://github.com/tuna4ll/termi/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/tuna4ll/termi/compare/v0.1.8...v0.2.0
[0.1.8]: https://github.com/tuna4ll/termi/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/tuna4ll/termi/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/tuna4ll/termi/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/tuna4ll/termi/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/tuna4ll/termi/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/tuna4ll/termi/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/tuna4ll/termi/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/tuna4ll/termi/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/tuna4ll/termi/releases/tag/v0.1.0
