# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/tuna4ll/termi/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/tuna4ll/termi/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/tuna4ll/termi/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/tuna4ll/termi/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/tuna4ll/termi/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/tuna4ll/termi/releases/tag/v0.1.0
