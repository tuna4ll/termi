# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/tuna4ll/termi/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/tuna4ll/termi/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/tuna4ll/termi/releases/tag/v0.1.0
