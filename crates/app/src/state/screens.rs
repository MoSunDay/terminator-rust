//! Screen-space bookkeeping for cross-window tab drags.
//!
//! Every window's render pass publishes its tab-strip rect (plus the
//! viewport outer rect it derives from) in SCREEN points, so a chip
//! dragged out of one window can be hit-tested against every OTHER
//! window's strip while the pointer travels across window borders.
//! Transient, never persisted.

use std::collections::HashMap;

use layout_tree::PaneId;

/// Screen-space geometry published by each window pass for cross-window
/// tab drags.
#[derive(Debug, Clone, Default)]
pub struct WinScreens {
    /// Window outer rect in screen points (`ViewportInfo::outer_rect`,
    /// monitor space at ui-point scale; None on Wayland).
    pub outer: HashMap<u64, egui::Rect>,
    /// Tab-strip rect in screen points (local strip translated by the
    /// window origin).
    pub strip: HashMap<u64, egui::Rect>,
}

/// In-flight cross-window tab drag: the dragged tab's anchor pane, the
/// source window id, and the window whose strip the pointer currently
/// hovers (None = no valid target).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XTabDrag {
    pub anchor: PaneId,
    pub source: u64,
    pub over: Option<u64>,
}
