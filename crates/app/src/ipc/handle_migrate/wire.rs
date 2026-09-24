//! Layout tree <-> wire mappers: the shape of a tab on the wire (no
//! pane ids) and the rebuild of that shape with freshly allocated
//! ones. Leaf order is depth-first both ways, which is what ties an
//! fd and a snapshot block to the pane they belong to.

use ipc_proto::migrate::{MigrateNode, MigratePane};
use layout_tree::{Axis, Node, PaneId};
use remote::{PaneKind, RemoteTarget};

use crate::session_map;
use crate::state::PaneMeta;

use super::Leaf;

pub fn tab_title(st: &crate::state::AppState, wi: usize, ti: usize) -> String {
    st.windows
        .get(wi)
        .and_then(|w| w.tree.tabs.get(ti))
        .map(|t| t.title.clone())
        .unwrap_or_else(|| "tab".to_string())
}

pub fn focus_index(
    st: &crate::state::AppState,
    wi: usize,
    ti: usize,
    leaves: &[Leaf],
) -> Option<u64> {
    let focused = st.windows.get(wi)?.tree.tabs.get(ti)?.focused;
    leaves
        .iter()
        .position(|l| l.id == focused)
        .map(|i| i as u64)
}

/// Mirror the layout tree onto the wire. Leaf order matches `leaves` (both
/// are depth-first over the same shape), which is what ties an fd and a
/// snapshot block to the pane they belong to.
pub fn wire_node(
    st: &crate::state::AppState,
    wi: usize,
    ti: usize,
    leaves: &[Leaf],
    next: &mut usize,
) -> MigrateNode {
    let node = st
        .windows
        .get(wi)
        .and_then(|w| w.tree.tabs.get(ti))
        .map(|t| t.root.clone());
    wire_from(node.as_ref(), leaves, next)
}

fn wire_from(node: Option<&Node>, leaves: &[Leaf], next: &mut usize) -> MigrateNode {
    match node {
        Some(Node::Split {
            axis,
            ratio,
            first,
            second,
        }) => MigrateNode::Split {
            axis: axis_str(*axis).to_string(),
            ratio: *ratio,
            first: Box::new(wire_from(Some(first), leaves, next)),
            second: Box::new(wire_from(Some(second), leaves, next)),
        },
        _ => {
            let leaf = leaves.get(*next);
            *next += 1;
            MigrateNode::Pane {
                pane: MigratePane {
                    manual_title: leaf.and_then(|l| l.meta.manual_title.clone()),
                    kind: kind_str(leaf.map(|l| &l.meta.kind)),
                    degraded: leaf.is_some_and(|l| l.meta.degraded),
                    pid: leaf.map_or(0, |l| l.pid),
                    cols: leaf.map_or(session_map::START_COLS, |l| l.cols),
                    rows: leaf.map_or(session_map::START_ROWS, |l| l.rows),
                },
            }
        }
    }
}

/// Rebuild a layout node, allocating a fresh pane id per leaf and
/// recording `(id, wire pane)` in depth-first order.
pub fn build_node(
    wire: MigrateNode,
    next: &mut PaneId,
    plan: &mut Vec<(PaneId, MigratePane)>,
) -> Node {
    match wire {
        MigrateNode::Pane { pane } => {
            let id = *next;
            *next += 1;
            plan.push((id, pane));
            Node::Pane { id }
        }
        MigrateNode::Split {
            axis,
            ratio,
            first,
            second,
        } => Node::Split {
            axis: axis_of(&axis),
            ratio: ratio.clamp(layout_tree::MIN_RATIO, layout_tree::MAX_RATIO),
            first: Box::new(build_node(*first, next, plan)),
            second: Box::new(build_node(*second, next, plan)),
        },
    }
}

/// Pane meta for an adopted leaf. A remote pane keeps the sender's host;
/// an ssh target for that host is taken from the registry when one exists,
/// otherwise a stub is built - the PTY is adopted as-is either way, so the
/// target only matters if the pane is ever respawned.
pub fn meta_of(mp: &MigratePane, registry: &[RemoteTarget]) -> PaneMeta {
    PaneMeta {
        kind: kind_of(&mp.kind, registry),
        manual_title: mp.manual_title.clone(),
        degraded: mp.degraded,
    }
}

fn kind_str(kind: Option<&PaneKind>) -> String {
    match kind {
        Some(PaneKind::Remote(t)) => format!("remote:{}", t.host),
        _ => "local".to_string(),
    }
}

fn kind_of(kind: &str, registry: &[RemoteTarget]) -> PaneKind {
    let Some(host) = kind.strip_prefix("remote:") else {
        return PaneKind::Local;
    };
    match registry.iter().find(|t| t.host == host) {
        Some(target) => PaneKind::Remote(target.clone()),
        None => PaneKind::Remote(RemoteTarget {
            label: host.to_string(),
            host: host.to_string(),
            user: None,
            port: None,
            session_name: remote::suggest_session_name(host),
        }),
    }
}

fn axis_str(axis: Axis) -> &'static str {
    match axis {
        Axis::Horizontal => "h",
        Axis::Vertical => "v",
    }
}

fn axis_of(axis: &str) -> Axis {
    if axis == "v" {
        Axis::Vertical
    } else {
        Axis::Horizontal
    }
}
