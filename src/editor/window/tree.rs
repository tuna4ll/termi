//! # Window tree
//!
//! **Purpose:** hold every open window and the arrangement they are shown in.
//!
//! **Responsibility:** the windows themselves live in a flat list so that an id
//! stays valid while its neighbours come and go; the *arrangement* is a tree of
//! nested splits over those ids. Keeping the two apart means closing a window
//! is a tree edit rather than a renumbering, and it lets the renderer walk the
//! tree without being able to disturb the windows hanging off it.
//!
//! Nothing here knows about the terminal. The tree describes proportions and
//! the renderer turns them into rectangles, which is also why directional focus
//! reads the area each window recorded on the last frame rather than computing
//! geometry itself.
//!
//! **Public API:** [`Windows`], [`Node`], [`Branch`], [`Axis`], [`Side`].

use super::Window;

/// Position of a window in the window list.
pub type WindowId = usize;

/// The direction a split lays its children out in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Children sit side by side, left to right — what `:vsplit` makes.
    Horizontal,
    /// Children sit one above another — what `:split` makes.
    Vertical,
}

/// Which way to look for a neighbouring window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Up,
    Down,
}

/// One child of a split, and the share of the space it takes.
#[derive(Debug, Clone)]
pub struct Branch {
    /// Share of the parent's length along its axis, relative to its siblings.
    pub weight: u16,
    /// The subtree.
    pub node: Node,
}

impl Branch {
    /// A branch taking an equal share.
    const fn new(node: Node) -> Self {
        Self { weight: 1, node }
    }
}

/// The arrangement of windows on screen.
#[derive(Debug, Clone)]
pub enum Node {
    /// A single window fills this space.
    Leaf(WindowId),
    /// The space is divided between two or more children.
    Split {
        /// Which way the children are laid out.
        axis: Axis,
        /// The children, in screen order. Never fewer than two: a split that
        /// loses all but one child collapses into that child.
        children: Vec<Branch>,
    },
}

/// Every open window, and how they are arranged.
#[derive(Debug)]
pub struct Windows {
    /// Open windows. A `None` is a closed window whose id must not be reused
    /// while the tree might still be renumbered around it.
    slots: Vec<Option<Window>>,
    /// How the live windows divide the screen.
    root: Node,
    /// The window the keyboard is aimed at. Always live.
    focus: WindowId,
}

impl Windows {
    /// Start with a single window filling the screen.
    #[must_use]
    pub fn new(window: Window) -> Self {
        Self {
            slots: vec![Some(window)],
            root: Node::Leaf(0),
            focus: 0,
        }
    }

    /// The arrangement, for the renderer to walk.
    #[must_use]
    pub const fn root(&self) -> &Node {
        &self.root
    }

    /// Id of the focused window.
    #[must_use]
    pub const fn focus(&self) -> WindowId {
        self.focus
    }

    /// The focused window.
    #[must_use]
    pub fn focused(&self) -> &Window {
        self.get(self.focus)
    }

    /// Mutable access to the focused window.
    pub fn focused_mut(&mut self) -> &mut Window {
        let focus = self.focus;
        self.get_mut(focus)
    }

    /// Aim the keyboard at `id`, ignoring a window that is not open.
    pub fn set_focus(&mut self, id: WindowId) {
        if self.slots.get(id).is_some_and(Option::is_some) {
            self.focus = id;
        }
    }

    /// The window `id` refers to.
    ///
    /// # Panics
    /// Panics if `id` is not an open window. Ids only come from this type, and
    /// a stale one is a bug rather than a condition to handle.
    #[must_use]
    pub fn get(&self, id: WindowId) -> &Window {
        self.slots[id]
            .as_ref()
            .expect("window id refers to an open window")
    }

    /// Mutable access to the window `id` refers to.
    ///
    /// # Panics
    /// Panics if `id` is not an open window.
    pub fn get_mut(&mut self, id: WindowId) -> &mut Window {
        self.slots[id]
            .as_mut()
            .expect("window id refers to an open window")
    }

    /// How many windows are open.
    #[must_use]
    pub fn count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }

    /// Every open window with its id, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (WindowId, &Window)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(id, slot)| slot.as_ref().map(|window| (id, window)))
    }

    /// Mutable access to every open window with its id.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (WindowId, &mut Window)> {
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(|(id, slot)| slot.as_mut().map(|window| (id, window)))
    }

    /// Window ids in screen order, left to right and top to bottom.
    #[must_use]
    pub fn ids(&self) -> Vec<WindowId> {
        let mut ids = Vec::new();
        collect(&self.root, &mut ids);
        ids
    }

    /// Split the focused window along `axis` and focus the new half.
    ///
    /// The new window starts where the old one is looking, which is what makes
    /// a split useful for comparing two parts of one file: vi does the same.
    pub fn split(&mut self, axis: Axis) -> WindowId {
        let clone = self.focused().clone();
        let fresh = self.insert_window(clone);
        let placed = insert_beside(&mut self.root, self.focus, fresh, axis);
        // The focused window is a leaf of the tree by construction. If that ever
        // stopped holding, the new window would exist without being drawn and
        // the keyboard would be aimed at something invisible, so undo the half
        // of the split that did happen rather than leave that behind.
        debug_assert!(placed, "the focused window should be a leaf of the tree");
        if !placed {
            self.slots[fresh] = None;
            return self.focus;
        }
        self.focus = fresh;
        fresh
    }

    /// Close `id`, returning `false` when it is the only window left.
    ///
    /// The focus moves to the window that took its place on screen.
    pub fn close(&mut self, id: WindowId) -> bool {
        if self.count() == 1 {
            return false;
        }
        let order = self.ids();
        let position = order.iter().position(|other| *other == id);

        if matches!(remove(&mut self.root, id), Removal::NotFound) {
            return false;
        }
        self.slots[id] = None;

        if self.focus == id {
            // Step onto whatever slid into the closed window's place, falling
            // back to the last window when it was the final one on screen.
            let remaining = self.ids();
            let next = position
                .map(|index| index.min(remaining.len() - 1))
                .unwrap_or(0);
            self.focus = remaining[next];
        }
        true
    }

    /// Close every window except the focused one.
    pub fn close_others(&mut self) {
        // Every slot rather than every leaf of the tree: the two agree, and
        // sweeping the list directly means they cannot drift apart here.
        for (id, slot) in self.slots.iter_mut().enumerate() {
            if id != self.focus {
                *slot = None;
            }
        }
        self.root = Node::Leaf(self.focus);
    }

    /// The window neighbouring the focused one on `side`, if there is one.
    ///
    /// Windows are compared by the area each recorded when it was last drawn:
    /// the nearest one that lies wholly beyond the focused window's edge wins,
    /// and ties go to whichever lines up best across the gap.
    #[must_use]
    pub fn neighbour(&self, side: Side) -> Option<WindowId> {
        let current = self.focused().area;
        let (cx, cy) = current.centre();

        self.iter()
            .filter(|(id, _)| *id != self.focus)
            .filter_map(|(id, window)| {
                let area = window.area;
                let (gap, offset) = match side {
                    Side::Left => (
                        current.x.checked_sub(area.right())?,
                        area.centre().1.abs_diff(cy),
                    ),
                    Side::Right => (
                        area.x.checked_sub(current.right())?,
                        area.centre().1.abs_diff(cy),
                    ),
                    Side::Up => (
                        current.y.checked_sub(area.bottom())?,
                        area.centre().0.abs_diff(cx),
                    ),
                    Side::Down => (
                        area.y.checked_sub(current.bottom())?,
                        area.centre().0.abs_diff(cx),
                    ),
                };
                Some((gap, offset, id))
            })
            .min()
            .map(|(_, _, id)| id)
    }

    /// The window covering the cell at `(x, y)`, if any.
    #[must_use]
    pub fn at(&self, x: u16, y: u16) -> Option<WindowId> {
        self.iter()
            .find(|(_, window)| window.area.contains(x, y))
            .map(|(id, _)| id)
    }

    /// Grow or shrink the focused window along `axis`.
    ///
    /// The change lands on the nearest enclosing split that runs along that
    /// axis, because that is the only one whose sizes the user can see moving.
    pub fn resize(&mut self, axis: Axis, delta: i16) {
        adjust(&mut self.root, self.focus, axis, delta);
    }

    /// Give every split an even share again.
    pub fn equalise(&mut self) {
        equalise(&mut self.root);
    }

    /// Put `window` in a free slot, or append one.
    fn insert_window(&mut self, window: Window) -> WindowId {
        match self.slots.iter().position(Option::is_none) {
            Some(id) => {
                self.slots[id] = Some(window);
                id
            }
            None => {
                self.slots.push(Some(window));
                self.slots.len() - 1
            }
        }
    }
}

/// Collect leaf ids in screen order.
fn collect(node: &Node, into: &mut Vec<WindowId>) {
    match node {
        Node::Leaf(id) => into.push(*id),
        Node::Split { children, .. } => {
            for branch in children {
                collect(&branch.node, into);
            }
        }
    }
}

/// Put `fresh` next to `target`, splitting along `axis`.
///
/// When the split enclosing `target` already runs along that axis, the new
/// window joins it as a sibling rather than nesting a second split inside it —
/// three `:vsplit`s should give three columns, not a column beside a column
/// beside a column.
fn insert_beside(node: &mut Node, target: WindowId, fresh: WindowId, axis: Axis) -> bool {
    match node {
        Node::Leaf(id) if *id == target => {
            *node = Node::Split {
                axis,
                children: vec![
                    Branch::new(Node::Leaf(target)),
                    Branch::new(Node::Leaf(fresh)),
                ],
            };
            true
        }
        Node::Leaf(_) => false,
        Node::Split {
            axis: existing,
            children,
        } => {
            if *existing == axis
                && let Some(index) = children
                    .iter()
                    .position(|branch| matches!(branch.node, Node::Leaf(id) if id == target))
            {
                let weight = children[index].weight;
                children.insert(
                    index + 1,
                    Branch {
                        weight,
                        node: Node::Leaf(fresh),
                    },
                );
                return true;
            }
            children
                .iter_mut()
                .any(|branch| insert_beside(&mut branch.node, target, fresh, axis))
        }
    }
}

/// What removing a window did to the subtree it was in.
enum Removal {
    /// The window was not in this subtree.
    NotFound,
    /// The window was removed and the subtree is still needed.
    Removed,
    /// The subtree is now empty and its parent should drop it.
    Collapse,
}

/// Take `target` out of the tree, collapsing splits left with one child.
fn remove(node: &mut Node, target: WindowId) -> Removal {
    match node {
        Node::Leaf(id) if *id == target => Removal::Collapse,
        Node::Leaf(_) => Removal::NotFound,
        Node::Split { children, .. } => {
            let mut hit = None;
            for (index, branch) in children.iter_mut().enumerate() {
                match remove(&mut branch.node, target) {
                    Removal::NotFound => continue,
                    outcome => {
                        hit = Some((index, matches!(outcome, Removal::Collapse)));
                        break;
                    }
                }
            }
            let Some((index, emptied)) = hit else {
                return Removal::NotFound;
            };
            if emptied {
                children.remove(index);
            }
            match children.len() {
                0 => Removal::Collapse,
                1 => {
                    // A split with one child is just that child.
                    let only = children.pop().expect("length checked");
                    *node = only.node;
                    Removal::Removed
                }
                _ => Removal::Removed,
            }
        }
    }
}

/// Move `delta` onto the focused window's share of the nearest split along
/// `axis`. Returns whether `target` is in this subtree and still unhandled.
fn adjust(node: &mut Node, target: WindowId, axis: Axis, delta: i16) -> bool {
    match node {
        Node::Leaf(id) => *id == target,
        Node::Split {
            axis: existing,
            children,
        } => {
            for branch in children.iter_mut() {
                if !adjust(&mut branch.node, target, axis, delta) {
                    continue;
                }
                if *existing != axis {
                    // Wrong axis: let an enclosing split handle it instead.
                    return true;
                }
                let weight = i32::from(branch.weight) + i32::from(delta);
                branch.weight = u16::try_from(weight.clamp(1, i32::from(MAX_WEIGHT))).unwrap_or(1);
                return false;
            }
            false
        }
    }
}

/// Ceiling on a window's share, so one window cannot squeeze the rest to
/// nothing however long a key is held down.
const MAX_WEIGHT: u16 = 200;

/// Reset every branch to an equal share.
fn equalise(node: &mut Node) {
    if let Node::Split { children, .. } = node {
        for branch in children {
            branch.weight = 1;
            equalise(&mut branch.node);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::window::Area;

    fn windows() -> Windows {
        Windows::new(Window::new(0))
    }

    /// Lay windows out in a row so the geometry-based helpers have something to
    /// read, without pulling the renderer into the test.
    fn place(windows: &mut Windows, areas: &[(WindowId, Area)]) {
        for (id, area) in areas {
            windows.get_mut(*id).area = *area;
        }
    }

    #[test]
    fn a_fresh_tree_is_one_focused_window() {
        let windows = windows();
        assert_eq!(windows.count(), 1);
        assert_eq!(windows.ids(), vec![0]);
        assert_eq!(windows.focus(), 0);
    }

    #[test]
    fn splitting_adds_a_window_and_focuses_it() {
        let mut windows = windows();
        let fresh = windows.split(Axis::Horizontal);
        assert_eq!(windows.count(), 2);
        assert_eq!(windows.focus(), fresh);
        assert_eq!(windows.ids(), vec![0, fresh]);
    }

    #[test]
    fn a_split_inherits_where_the_old_window_was_looking() {
        let mut windows = windows();
        windows.focused_mut().view.top_line = 40;
        let fresh = windows.split(Axis::Vertical);
        assert_eq!(windows.get(fresh).view.top_line, 40);
    }

    #[test]
    fn splitting_along_the_same_axis_stays_flat() {
        let mut windows = windows();
        windows.split(Axis::Horizontal);
        windows.split(Axis::Horizontal);

        let Node::Split { children, axis } = windows.root() else {
            panic!("root should be a split");
        };
        assert_eq!(*axis, Axis::Horizontal);
        assert_eq!(children.len(), 3);
        assert!(children.iter().all(|b| matches!(b.node, Node::Leaf(_))));
    }

    #[test]
    fn splitting_along_the_other_axis_nests() {
        let mut windows = windows();
        windows.split(Axis::Horizontal);
        windows.split(Axis::Vertical);

        let Node::Split { children, axis } = windows.root() else {
            panic!("root should be a split");
        };
        assert_eq!(*axis, Axis::Horizontal);
        assert_eq!(children.len(), 2);
        assert!(matches!(children[1].node, Node::Split { .. }));
    }

    #[test]
    fn closing_the_last_split_collapses_the_tree() {
        let mut windows = windows();
        let fresh = windows.split(Axis::Horizontal);
        assert!(windows.close(fresh));
        assert_eq!(windows.count(), 1);
        assert!(matches!(windows.root(), Node::Leaf(0)));
        assert_eq!(windows.focus(), 0);
    }

    #[test]
    fn the_only_window_cannot_be_closed() {
        let mut windows = windows();
        assert!(!windows.close(0));
        assert_eq!(windows.count(), 1);
    }

    #[test]
    fn closing_the_focused_window_moves_the_focus_on() {
        let mut windows = windows();
        windows.split(Axis::Horizontal);
        let third = windows.split(Axis::Horizontal);
        windows.set_focus(0);
        windows.close(0);
        // Window 0 is gone, so the focus lands on whatever took its place.
        assert!(windows.ids().contains(&windows.focus()));
        assert_eq!(windows.count(), 2);
        assert!(windows.ids().contains(&third));
    }

    #[test]
    fn a_closed_slot_is_reused_by_the_next_split() {
        let mut windows = windows();
        let second = windows.split(Axis::Horizontal);
        windows.close(second);
        let third = windows.split(Axis::Horizontal);
        assert_eq!(third, second);
    }

    #[test]
    fn only_leaves_the_focused_window_alone_on_screen() {
        let mut windows = windows();
        windows.split(Axis::Horizontal);
        windows.split(Axis::Vertical);
        let kept = windows.focus();

        windows.close_others();
        assert_eq!(windows.count(), 1);
        assert_eq!(windows.ids(), vec![kept]);
        assert!(matches!(windows.root(), Node::Leaf(id) if *id == kept));
    }

    #[test]
    fn neighbours_are_found_across_the_nearest_edge() {
        let mut windows = windows();
        let right = windows.split(Axis::Horizontal);
        place(
            &mut windows,
            &[
                (0, Area::new(0, 0, 40, 20)),
                (right, Area::new(40, 0, 40, 20)),
            ],
        );

        windows.set_focus(0);
        assert_eq!(windows.neighbour(Side::Right), Some(right));
        assert_eq!(windows.neighbour(Side::Left), None);
        assert_eq!(windows.neighbour(Side::Up), None);

        windows.set_focus(right);
        assert_eq!(windows.neighbour(Side::Left), Some(0));
        assert_eq!(windows.neighbour(Side::Right), None);
    }

    #[test]
    fn the_nearest_of_several_neighbours_wins() {
        let mut windows = windows();
        let middle = windows.split(Axis::Horizontal);
        let far = windows.split(Axis::Horizontal);
        place(
            &mut windows,
            &[
                (0, Area::new(0, 0, 20, 20)),
                (middle, Area::new(20, 0, 20, 20)),
                (far, Area::new(40, 0, 20, 20)),
            ],
        );

        windows.set_focus(0);
        assert_eq!(windows.neighbour(Side::Right), Some(middle));
    }

    #[test]
    fn a_click_lands_in_the_window_under_it() {
        let mut windows = windows();
        let right = windows.split(Axis::Horizontal);
        place(
            &mut windows,
            &[
                (0, Area::new(0, 0, 40, 20)),
                (right, Area::new(40, 0, 40, 20)),
            ],
        );

        assert_eq!(windows.at(10, 5), Some(0));
        assert_eq!(windows.at(50, 5), Some(right));
        assert_eq!(windows.at(200, 5), None);
    }

    #[test]
    fn resizing_moves_the_focused_windows_share() {
        let mut windows = windows();
        let right = windows.split(Axis::Horizontal);
        windows.resize(Axis::Horizontal, 2);

        let Node::Split { children, .. } = windows.root() else {
            panic!("root should be a split");
        };
        let grown = children
            .iter()
            .find(|branch| matches!(branch.node, Node::Leaf(id) if id == right))
            .expect("the split still holds the focused window");
        assert_eq!(grown.weight, 3);
    }

    #[test]
    fn resizing_along_the_wrong_axis_does_nothing() {
        let mut windows = windows();
        windows.split(Axis::Horizontal);
        windows.resize(Axis::Vertical, 5);

        let Node::Split { children, .. } = windows.root() else {
            panic!("root should be a split");
        };
        assert!(children.iter().all(|branch| branch.weight == 1));
    }

    #[test]
    fn equalising_puts_every_share_back() {
        let mut windows = windows();
        windows.split(Axis::Horizontal);
        windows.resize(Axis::Horizontal, 6);
        windows.equalise();

        let Node::Split { children, .. } = windows.root() else {
            panic!("root should be a split");
        };
        assert!(children.iter().all(|branch| branch.weight == 1));
    }

    #[test]
    fn a_window_never_shrinks_past_its_floor() {
        let mut windows = windows();
        windows.split(Axis::Horizontal);
        windows.resize(Axis::Horizontal, -100);

        let Node::Split { children, .. } = windows.root() else {
            panic!("root should be a split");
        };
        assert!(children.iter().all(|branch| branch.weight >= 1));
    }
}
