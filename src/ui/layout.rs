//! # Window layout
//!
//! **Purpose:** turn the window tree into rectangles.
//!
//! **Responsibility:** the editor layer describes the arrangement as nested
//! splits with relative weights and knows nothing about cells; this is where
//! that becomes screen geometry. Splits are drawn with a one-cell rule between
//! neighbours, because two text areas that meet with no gap read as one badly
//! wrapped file.
//!
//! **Public API:** [`Panes`], [`solve`].

use ratatui::layout::{Constraint, Layout, Rect};

use crate::editor::window::WindowId;
use crate::editor::window::tree::{Axis, Node};

/// Where every window goes this frame, and the rules drawn between them.
#[derive(Debug, Default)]
pub struct Panes {
    /// One rectangle per open window, in screen order.
    pub windows: Vec<(WindowId, Rect)>,
    /// Separators, with the axis of the split that produced them: a horizontal
    /// split is divided by a vertical rule and the other way round.
    pub separators: Vec<(Rect, Axis)>,
}

/// Divide `area` between the windows in `node`.
#[must_use]
pub fn solve(node: &Node, area: Rect) -> Panes {
    let mut panes = Panes::default();
    partition(node, area, &mut panes);
    panes
}

fn partition(node: &Node, area: Rect, panes: &mut Panes) {
    let (axis, children) = match node {
        Node::Leaf(id) => {
            panes.windows.push((*id, area));
            return;
        }
        Node::Split { axis, children } => (*axis, children),
    };

    // Every child but the last is followed by a one-cell rule, so the gaps are
    // part of the constraint list rather than carved off the children after the
    // fact — that way the solver, not this code, absorbs the rounding.
    let mut constraints = Vec::with_capacity(children.len() * 2 - 1);
    for (index, branch) in children.iter().enumerate() {
        if index > 0 {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Fill(branch.weight));
    }

    let slots = match axis {
        Axis::Horizontal => Layout::horizontal(constraints).split(area),
        Axis::Vertical => Layout::vertical(constraints).split(area),
    };

    for (index, slot) in slots.iter().enumerate() {
        if index % 2 == 1 {
            panes.separators.push((*slot, axis));
        } else {
            partition(&children[index / 2].node, *slot, panes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::window::Window;
    use crate::editor::window::tree::Windows;

    fn area() -> Rect {
        Rect::new(0, 0, 81, 24)
    }

    #[test]
    fn a_single_window_takes_the_whole_area() {
        let windows = Windows::new(Window::new(0));
        let panes = solve(windows.root(), area());
        assert_eq!(panes.windows, vec![(0, area())]);
        assert!(panes.separators.is_empty());
    }

    #[test]
    fn a_vertical_split_divides_the_width_and_leaves_a_rule() {
        let mut windows = Windows::new(Window::new(0));
        let right = windows.split(Axis::Horizontal);
        let panes = solve(windows.root(), area());

        assert_eq!(panes.windows.len(), 2);
        assert_eq!(panes.separators.len(), 1);

        let (_, left_rect) = panes.windows[0];
        let (id, right_rect) = panes.windows[1];
        assert_eq!(id, right);
        // 81 columns: 40 each side with a one-column rule between them.
        assert_eq!(left_rect.width, 40);
        assert_eq!(right_rect.width, 40);
        assert_eq!(panes.separators[0].0.x, 40);
        assert_eq!(panes.separators[0].0.width, 1);
        assert_eq!(right_rect.x, 41);
    }

    #[test]
    fn a_horizontal_split_divides_the_height() {
        let mut windows = Windows::new(Window::new(0));
        windows.split(Axis::Vertical);
        let panes = solve(windows.root(), Rect::new(0, 0, 80, 25));

        assert_eq!(panes.windows.len(), 2);
        assert_eq!(panes.windows[0].1.height, 12);
        assert_eq!(panes.windows[1].1.height, 12);
        assert_eq!(panes.separators[0].0.height, 1);
    }

    #[test]
    fn three_columns_get_two_rules() {
        let mut windows = Windows::new(Window::new(0));
        windows.split(Axis::Horizontal);
        windows.split(Axis::Horizontal);
        let panes = solve(windows.root(), area());

        assert_eq!(panes.windows.len(), 3);
        assert_eq!(panes.separators.len(), 2);
        // Nothing overlaps and nothing is left over.
        let covered: u16 = panes
            .windows
            .iter()
            .map(|(_, rect)| rect.width)
            .sum::<u16>()
            + panes.separators.len() as u16;
        assert_eq!(covered, area().width);
    }

    #[test]
    fn weights_are_respected() {
        let mut windows = Windows::new(Window::new(0));
        windows.split(Axis::Horizontal);
        windows.resize(Axis::Horizontal, 1);
        let panes = solve(windows.root(), Rect::new(0, 0, 61, 24));

        // The focused half now carries twice the weight of the other.
        assert_eq!(panes.windows[0].1.width, 20);
        assert_eq!(panes.windows[1].1.width, 40);
    }

    #[test]
    fn nested_splits_stay_inside_their_parent() {
        let mut windows = Windows::new(Window::new(0));
        windows.split(Axis::Horizontal);
        windows.split(Axis::Vertical);
        let panes = solve(windows.root(), area());

        assert_eq!(panes.windows.len(), 3);
        for (_, rect) in &panes.windows {
            assert!(rect.right() <= area().right());
            assert!(rect.bottom() <= area().bottom());
        }
    }
}
