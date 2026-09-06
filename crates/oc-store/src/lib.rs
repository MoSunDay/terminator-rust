//! Direct SQLite access to OpenCoder per-workdir stores.
//!
//! terminator-rust must hand user input to the OpenCoder TUI (the sole
//! runner) without going through a socket: [`db::insert_input`] appends a
//! `steer`/`queue` row to `session_inputs` that the TUI claims at its turn
//! boundaries (steer) or idle boundaries (queue), and [`db::receipts`]
//! reads back the `steer_consumed`/`queue_consumed` events it emits.
//!
//! Two invariants keep this safe against foreign schema versions:
//! the store file is never created here (open only), and both open paths
//! enforce `user_version == 18` plus the exact `session_inputs` columns.
//!
//! Pure-functional style: plain structs + free functions, no OOP.
pub mod db;
pub mod ulid;
