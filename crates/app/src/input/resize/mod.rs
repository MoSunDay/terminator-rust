//! Borderless-window edge resizing: invisible pointer strips at the
//! viewport border drive an APP-OWNED resize. The previous design sent
//! `ViewportCommand::BeginResize` (EWMH _NET_WM_MOVERESIZE), handing
//! gesture termination to the WM: the WM's pointer grab also swallows
//! the ButtonRelease, so egui keeps believing the button is held (the
//! next gesture then cannot latch) and a WM that misses the release
//! keeps resizing after mouseup. Applying `InnerSize`/`OuterPosition`
//! each dragged frame keeps the release - and with it termination - on
//! our side of the fence: the window stops following the pointer the
//! moment the button goes up.
//!
//! Split by responsibility: `hit` answers "which border direction is
//! this pointer in", `gesture` holds the in-flight anchor and derives
//! the resized rect, `strips` registers the border widgets and drives
//! the live gesture.

mod gesture;
mod hit;
mod strips;

// The module surface pre-split, kept stable for callers: window passes
// register the strips, the renderer hit-tests for suppression.
pub use gesture::Gesture;
pub use hit::dir_at;
pub use strips::strips;

#[cfg(test)]
mod tests;
