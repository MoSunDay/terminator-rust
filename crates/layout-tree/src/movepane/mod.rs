//! Drag-and-drop pane moves: drop-zone geometry and the tree surgery behind
//! the Ctrl+drag pane-move gesture.

mod crosstab;
mod sametab;
mod surgery;
mod zone;

#[cfg(test)]
mod tests;

pub use crosstab::move_pane_across_tabs;
pub use sametab::move_pane_to_pane;
pub use zone::{zone_for, zone_rect, DropZone};
