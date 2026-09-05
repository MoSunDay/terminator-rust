//! Moving focus between the panes of a tab.

use crate::layout::{layout_tab, DEFAULT_DIVIDER_W};
use crate::rect::{rect_center, Rect};
use crate::tree::{sorted_pane_ids, LayoutTree, PaneId, Tab};

/// Direction of a focus move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDir {
    /// Move to the pane above.
    Up,
    /// Move to the pane below.
    Down,
    /// Move to the pane on the left.
    Left,
    /// Move to the pane on the right.
    Right,
}

/// Nominal area for focus geometry: only relative pane positions matter.
const NOMINAL_AREA: Rect = Rect { x: 0.0, y: 0.0, w: 1024.0, h: 640.0 };

/// Gap between the 1-D ranges `a` and `b` (`0.0` when they overlap).
fn range_gap(a: (f32, f32), b: (f32, f32)) -> f32 {
    (b.0 - a.1).max(a.0 - b.1).max(0.0)
}

/// Distance from `from` to `to` when moving in `dir`: the edge-to-edge gap
/// along the axis of movement plus the perpendicular gap between the rects.
/// `None` when `to` is not strictly beyond `from` in that direction
/// (compared by rect centers).
fn dir_distance(dir: FocusDir, from: Rect, to: Rect) -> Option<f32> {
    let (fx, fy) = rect_center(from);
    let (tx, ty) = rect_center(to);
    let x_gap = |a: Rect, b: Rect| range_gap((a.x, a.x + a.w), (b.x, b.x + b.w));
    let y_gap = |a: Rect, b: Rect| range_gap((a.y, a.y + a.h), (b.y, b.y + b.h));
    match dir {
        FocusDir::Up if ty < fy => Some(y_gap(from, to) + x_gap(from, to)),
        FocusDir::Down if ty > fy => Some(y_gap(from, to) + x_gap(from, to)),
        FocusDir::Left if tx < fx => Some(x_gap(from, to) + y_gap(from, to)),
        FocusDir::Right if tx > fx => Some(x_gap(from, to) + y_gap(from, to)),
        _ => None,
    }
}

/// Moves focus of tab `tab` one pane in `dir`.
///
/// The target is the nearest pane strictly beyond the focused pane in that
/// direction, measured edge to edge (perpendicular separation counts as
/// distance, so a pane straight ahead beats one that is merely closer in
/// the movement axis; ties go to the pane first in tree order). Returns the
/// newly focused id, or `None` — leaving focus untouched — when no pane lies
/// in that direction.
pub fn move_focus(tree: &mut LayoutTree, tab: usize, dir: FocusDir) -> Option<PaneId> {
    let t = tree.tabs.get_mut(tab)?;
    let rects = layout_tab(t, NOMINAL_AREA, 0.0, DEFAULT_DIVIDER_W);
    let from = rects
        .iter()
        .find(|(id, _)| *id == t.focused)
        .map(|(_, r)| *r)?;
    let mut best: Option<(f32, PaneId)> = None;
    for (id, r) in &rects {
        if *id == t.focused {
            continue;
        }
        if let Some(dist) = dir_distance(dir, from, *r) {
            if best.map_or(true, |(d, _)| dist < d) {
                best = Some((dist, *id));
            }
        }
    }
    let (_, id) = best?;
    t.focused = id;
    Some(id)
}

/// Id of the next (`forward == true`) or previous pane in ascending pane id
/// order, wrapping around. A single-pane tab yields its own focused id.
pub fn neighbor_for_cycle(tab: &Tab, forward: bool) -> PaneId {
    let ids = sorted_pane_ids(&tab.root);
    if ids.len() < 2 {
        return tab.focused;
    }
    let idx = ids.iter().position(|&id| id == tab.focused).unwrap_or(0);
    if forward {
        ids[(idx + 1) % ids.len()]
    } else {
        ids[(idx + ids.len() - 1) % ids.len()]
    }
}

/// Applies [`neighbor_for_cycle`] and stores the result as the tab's focus.
/// Returns the newly focused id, or `None` when `tab` is out of range.
pub fn cycle_focus(tree: &mut LayoutTree, tab: usize, forward: bool) -> Option<PaneId> {
    let t = tree.tabs.get_mut(tab)?;
    let next = neighbor_for_cycle(t, forward);
    t.focused = next;
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rect::Axis;
    use crate::tabs::{new_tab, new_tree};
    use crate::tree::split_pane;

    /// Panes 1 (top-left), 3 (top-right) and 2 (bottom); focus starts on 3.
    fn three_panes() -> LayoutTree {
        let mut tree = new_tree("main");
        assert_eq!(split_pane(&mut tree, 0, 1, Axis::Horizontal), Some(2));
        assert_eq!(split_pane(&mut tree, 0, 1, Axis::Vertical), Some(3));
        tree
    }

    #[test]
    fn moves_in_each_direction() {
        let mut tree = three_panes(); // focused: 3 (top-right)
        assert_eq!(move_focus(&mut tree, 0, FocusDir::Left), Some(1));
        assert_eq!(tree.tabs[0].focused, 1);
        // Straight ahead beats diagonal: Right from 1 goes to 3, not 2.
        assert_eq!(move_focus(&mut tree, 0, FocusDir::Right), Some(3));
        assert_eq!(move_focus(&mut tree, 0, FocusDir::Down), Some(2));
        // From the bottom pane, Up prefers the pane directly above.
        assert_eq!(move_focus(&mut tree, 0, FocusDir::Up), Some(1));
        assert_eq!(move_focus(&mut tree, 0, FocusDir::Right), Some(3));
        assert_eq!(move_focus(&mut tree, 0, FocusDir::Down), Some(2));
    }

    #[test]
    fn dead_ends_are_noops() {
        let mut tree = three_panes();
        assert_eq!(move_focus(&mut tree, 7, FocusDir::Up), None); // bad tab
        tree.tabs[0].focused = 1; // top-left: nothing left or above
        assert_eq!(move_focus(&mut tree, 0, FocusDir::Left), None);
        assert_eq!(move_focus(&mut tree, 0, FocusDir::Up), None);
        assert_eq!(tree.tabs[0].focused, 1);
        tree.tabs[0].focused = 2; // bottom: nothing below
        assert_eq!(move_focus(&mut tree, 0, FocusDir::Down), None);
    }

    #[test]
    fn cycle_neighbors_wrap_around() {
        let tree = three_panes(); // pane ids 1, 2, 3
        let tab = &tree.tabs[0];
        let with_focus = |focused: PaneId| Tab {
            title: tab.title.clone(),
            root: tab.root.clone(),
            focused,
        };
        assert_eq!(neighbor_for_cycle(&with_focus(1), true), 2);
        assert_eq!(neighbor_for_cycle(&with_focus(2), true), 3);
        assert_eq!(neighbor_for_cycle(&with_focus(3), true), 1); // wraps forward
        assert_eq!(neighbor_for_cycle(&with_focus(1), false), 3); // wraps back
        assert_eq!(neighbor_for_cycle(&with_focus(2), false), 1);
    }

    #[test]
    fn cycle_focus_applies_and_handles_single_pane() {
        let mut tree = new_tree("solo"); // pane 1
        assert_eq!(cycle_focus(&mut tree, 0, true), Some(1));
        assert_eq!(cycle_focus(&mut tree, 0, false), Some(1));
        assert_eq!(tree.tabs[0].focused, 1);
        new_tab(&mut tree, "duo"); // pane 2 in tab 1
        split_pane(&mut tree, 1, 2, Axis::Vertical); // pane 3, focused
        assert_eq!(cycle_focus(&mut tree, 1, true), Some(2)); // wraps to first
        assert_eq!(tree.tabs[1].focused, 2);
        assert_eq!(cycle_focus(&mut tree, 1, false), Some(3));
        assert_eq!(cycle_focus(&mut tree, 9, false), None); // bad tab
    }
}
