//! Cross-instance tab migration: the two halves of the hand-over.
//!
//! A tab travels as one `TabOffer` request: a JSON header line (the split
//! tree, with pane ids left out - the receiver allocates fresh ones),
//! followed on the SAME sendmsg by one SCM_RIGHTS descriptor per leaf (in
//! depth-first order) and one length-prefixed vt snapshot block per leaf.
//!
//! Sender ([`migrate_out`]): duplicate every master fd FIRST (the dup is
//! what the receiver adopts, so it must not depend on the local session
//! surviving), then stop the readers and encode the state, then offer.
//! Only a positive ack touches local state; a failure re-adopts the panes
//! from the dups we still hold, so a refused offer is a no-op.
//!
//! Receiver ([`tab_offer`]): adopt every PTY into a fresh session and only
//! then plant the tab into the user-focused window. Failures drop the
//! already-adopted sessions WITHOUT signalling their children - their PTYs
//! are still in transit back to the sender's rollback.
//!
//! The sender never signals the migrated children: the child keeps running
//! in place and both processes hold the same pty description, so the pane
//! is literally the same shell after the hand-over.

// Split by responsibility: `out` is the sender half (offer, ack,
// commit/rollback), `in` is the receiver half, `wire` maps the
// layout tree to the wire shape and back.

mod r#in;
mod out;
mod wire;

// The module surface pre-split, kept stable for callers.
pub use out::migrate_out;
pub use r#in::tab_offer;
// The migration tests drive the sender's reader join directly.
#[cfg(test)]
pub(crate) use out::wait_reader;

use std::time::Duration;

use layout_tree::PaneId;

use crate::state::PaneMeta;

/// Connect/ack budget for the peer instance (its handler runs inline on
/// that instance's UI thread, so a healthy peer answers in milliseconds).
const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// Cap on the ack line (mirrors the server's own header cap).
const MAX_LINE: usize = 1 << 20;
/// How long to wait for a stopped reader thread to close its master fd
/// (its poll timeout caps the real latency).
const READER_JOIN: Duration = Duration::from_millis(400);

/// One pane leaf of a migrating tab, with everything the receiver needs to
/// rebuild it (the wire carries no ids, so the order IS the identity).
struct Leaf {
    id: PaneId,
    meta: PaneMeta,
    pid: i32,
    cols: u16,
    rows: u16,
}

#[cfg(test)]
#[path = "../migrate_tests.rs"]
mod tests;
