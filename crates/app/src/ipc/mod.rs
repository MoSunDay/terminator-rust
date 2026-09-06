//! Unix-domain-socket control plane: external clients (`terminator-ctl`)
//! talk one-line JSON over `TERMINATOR_SOCK` to drive running panes.
//!
//! - `server`: listener thread + UI-thread inbox (`Ipc`, `start`, `drain`)
//! - `handle`: request -> response translation against live app state

pub mod handle;
pub mod server;
