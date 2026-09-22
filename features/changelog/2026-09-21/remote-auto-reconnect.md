# Remote auto-reconnect (network drop recovery)

Date: 2026-09-21 · Status: verified (remote/app unit tests + ignored e2e:
net_drop_e2e server_reset/outage_window, app reconnect_net, zellij_e2e
reattach_recovers_session_state; full workspace fmt/clippy/tests clean)

## The problem

An ssh connection drop (network outage, server RST) used to end the pane
like any exited shell - corpse-close or an "exit 255" label - even though
the zellij session keeps running on the host.

## The model

- remote/session.rs: EXIT_SSH_FAIL = 255 + `is_disconnect(code)` (255 or
  negative poll/read failures) -> `PaneStatus::Disconnected`; the
  distinction "connection dropped" vs "session ended" now exists on the
  wire types. ssh argv gained ConnectTimeout=10 so a dead network fails
  in bounded time instead of hanging.
- app actions/reconnect.rs (new): per-frame `pump` - fresh disconnects
  schedule a retry after backoff(1s, 2s, 4s, cap 5s); a due retry just
  terminates the dead session and lets ensure_sessions respawn the SAME
  plan (`zellij attach --create` reattaches, on-screen state intact).
  A session that lived >= 30s (STABLE_AFTER) before dying was healthy ->
  ladder restarts at the bottom; manual respawn resets it too.
  `reconnect_n` deliberately survives terminate so rapid fail-loops grow
  the delay.
- actions close_exited: remote disconnect exits are EXEMPT from the
  corpse-close - the pane stays with its last frame plus a
  "reconnecting" badge (pane_header) instead of "exit 255".
- screen.rs frame loop order: pump_all -> close_exited ->
  reconnect::pump -> ensure_sessions (driven once per frame).

## e2e (real network interruption)

HARD RULE (agents.md): the test process itself runs inside an sshd
session - never stop/restart sshd; interrupt via kill -9 of the
per-connection sshd child or an iptables REJECT window on `-i lo
--dport 22` (comment-tagged, idempotent delete, never assert while
inserted). REJECT tcp-reset fails ssh in ~0.1s; both shapes (server RST
exit 255, outage window) reattach with marker state intact.
