//! Unix-domain-socket control plane: external clients (`terminator-ctl`)
//! talk one-line JSON over `TERMINATOR_SOCK` to drive running panes.
//!
//! - `server`: listener thread + UI-thread inbox (`Ipc`, `start`, `drain`)
//! - `handle`: request -> response translation against live app state
//! - `fd`: SCM_RIGHTS descriptor passing (cross-instance tab migration)
//! - `handle_migrate`: cross-instance tab migration (both halves)

pub mod fd;
pub mod handle;
pub mod handle_migrate;
pub mod server;
