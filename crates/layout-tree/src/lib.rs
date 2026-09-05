//! Pure data model and functions for a terminator-style pane layout.
//!
//! Each [`Tab`] holds a binary tree of panes ([`Node`]) that can be
//! recursively split with draggable dividers. This crate owns only the data
//! model and pure operations — geometry ([`rect`]), tree structure and
//! splits ([`tree`]), tab/pane lifecycle ([`tabs`]), layout output
//! ([`layout`]) and focus movement ([`focus`]). There is no UI code here.
//!
//! Everything is plain data plus free functions: no traits, no methods, no
//! interior mutability. The public API is re-exported flat at the crate root,
//! so e.g. `layout_tree::split_pane` and `layout_tree::Axis` work directly.

pub mod focus;
pub mod layout;
pub mod rect;
pub mod tabs;
pub mod tree;

pub use focus::{cycle_focus, move_focus, neighbor_for_cycle, FocusDir};
pub use layout::{content_rect, layout_tab, pane_at, DEFAULT_DIVIDER_W};
pub use rect::{
    near_divider, rect_area, rect_center, rect_contains, shrink, split_rect, split_rect_gapped,
    Axis, Rect,
};
pub use tabs::{alloc_pane_id, close_pane, close_tab, new_tab, new_tree, next_pane_id};
pub use tree::{
    contains_pane, find_node, find_node_mut, pane_count, pane_ids, parent_axis, parent_ratio,
    set_parent_ratio, set_ratio_at_level, sorted_pane_ids, split_pane, LayoutTree, Node, PaneId,
    Tab, MAX_RATIO, MIN_RATIO,
};
