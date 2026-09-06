//! Terminal pane sessions: PTY plumbing + libghostty-vt terminal state.
//!
//! One [`Session`] owns a child process on a PTY and the corresponding
//! [`libghostty_vt::Terminal`]. Bytes read from the PTY are fed to the
//! terminal via a reader thread and an mpsc channel; the UI drains the
//! channel each frame, then snapshots the render state into plain data
//! (`Frame`) that carries no libghostty types or lifetimes.

pub mod effects;
pub mod mouse;
pub mod pty;
pub mod task;
pub mod term;
#[cfg(test)]
mod tests;
pub mod viewport;

pub use pty::{pty_resize, pty_wait, pty_write, PtyHandle};
pub use task::{PtyEvent, Session, SessionOpts};
pub use term::{CellData, Frame, FrameCursor};
