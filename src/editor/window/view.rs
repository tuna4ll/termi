//! # Viewport
//!
//! **Purpose:** remember which slice of a document is on screen.
//!
//! **Responsibility:** two scroll offsets and the rules for keeping the caret
//! inside the visible region. The view stores *text* coordinates only; it never
//! learns the terminal size permanently, because the size can change between any
//! two frames — the renderer passes the current dimensions in on each call.
//!
//! **Public API:** [`View`].

/// The scrolled position of a buffer within one window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct View {
    /// First document line drawn at the top of the text area.
    pub top_line: usize,
    /// First display column drawn at the left edge, for horizontal scrolling.
    pub left_col: usize,
}

impl View {
    /// Scroll the minimum amount needed to bring `(line, col)` into view.
    ///
    /// `scrolloff` keeps that many lines of context above and below the caret,
    /// which is what stops the cursor from sticking to the window edge while
    /// scrolling. It is clamped to half the height so it still behaves on tiny
    /// windows.
    pub fn scroll_to(
        &mut self,
        line: usize,
        col: usize,
        height: usize,
        width: usize,
        scrolloff: usize,
    ) {
        if height == 0 || width == 0 {
            return;
        }
        let margin = scrolloff.min(height.saturating_sub(1) / 2);

        // Vertical.
        let top_limit = line.saturating_sub(margin);
        if top_limit < self.top_line {
            self.top_line = top_limit;
        }
        let bottom_limit = line + margin + 1;
        if bottom_limit > self.top_line + height {
            self.top_line = bottom_limit - height;
        }

        // Horizontal: no margin, because a wide caret jump should not drag the
        // whole line sideways more than necessary.
        if col < self.left_col {
            self.left_col = col;
        } else if col >= self.left_col + width {
            self.left_col = col - width + 1;
        }
    }

    /// Scroll `delta` lines without moving the caret, clamped to the document.
    pub fn scroll_lines(&mut self, delta: isize, last_line: usize) {
        let top = if delta < 0 {
            self.top_line.saturating_sub(delta.unsigned_abs())
        } else {
            self.top_line.saturating_add(delta.unsigned_abs())
        };
        self.top_line = top.min(last_line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_down_keeps_the_margin() {
        let mut view = View::default();
        view.scroll_to(20, 0, 10, 80, 2);
        // Line 20 must sit 2 lines above the bottom of a 10 line window.
        assert_eq!(view.top_line, 13);
    }

    #[test]
    fn scrolling_up_keeps_the_margin() {
        let mut view = View {
            top_line: 50,
            left_col: 0,
        };
        view.scroll_to(52, 0, 10, 80, 3);
        assert_eq!(view.top_line, 49);
    }

    #[test]
    fn margin_is_capped_on_short_windows() {
        let mut view = View::default();
        view.scroll_to(5, 0, 3, 80, 10);
        assert_eq!(view.top_line, 4);
    }

    #[test]
    fn horizontal_scrolling_follows_the_caret() {
        let mut view = View::default();
        view.scroll_to(0, 100, 10, 40, 0);
        assert_eq!(view.left_col, 61);
        view.scroll_to(0, 3, 10, 40, 0);
        assert_eq!(view.left_col, 3);
    }

    #[test]
    fn a_visible_caret_does_not_scroll() {
        let mut view = View {
            top_line: 10,
            left_col: 0,
        };
        view.scroll_to(14, 5, 10, 80, 2);
        assert_eq!(view.top_line, 10);
    }
}
