# Attention notices: one transport — OSC 9 to the pane's own tty

`terminator-ctl notice` takes NO arguments and never talks to the app
control socket, the keeper, or any environment variable: it writes the
canonical OSC 9 desktop-notification bytes (`\x1b]9;terminator-rust
notice\x07`, from `ipc_proto::notice_osc()`) to `/dev/tty` — the
controlling terminal of whatever pane the hook runs in — falling back to
stdout only when no tty can be opened. Best-effort by contract: a
notification must never break a hook, so a write failure surfaces as a
short stderr note and the command still exits 0. Any argument is a usage
error (exit 2).

The bytes are ordinary terminal OUTPUT, so this works in EVERY pane
kind — local panes, plain or degraded ssh panes, and keeper sessions.
The pane's VT engine parses the OSC 9, `on_desktop_notification` marks
the pane, and its tab chip gets the blue attention dot while that tab is
not the focused window's active tab; viewing the tab acknowledges it
(per-frame). Through a `terminator-session attach` the bytes traverse
the keeper like any other PTY output — the replay history records them,
so a reattach re-fires the badge. That is a consequence of the single
transport, not a second route.

## Usage

```sh
# POSIX sh one-liner, opencoder-style onFinish hook — any pane, any host
terminator-ctl notice
```

Zero-binary fallback for machines without the terminator tools (the
exact canonical bytes):

```sh
printf '\033]9;terminator-rust notice\007'
```

No session lookup, no env routing, no options. The ipc `Request::Notice`
type and its app handler stay as the socket route for external/legacy
callers; `terminator-ctl notice` does not use them.

## Removed (2026-09-27)

- The `terminator-session notice` subcommand and the keeper broadcast
  channel — never rely on them again; OSC 9 output through the attach
  stream replaces both.
- `$TERMINATOR_SESSION` is still exported into keeper shells, purely as
  metadata for scripts that want to know they are remote. Nothing in the
  notice path reads it.

## Limitations

- The channel is the pane the hook runs in: a notice marks that pane
  (via a keeper attach, every pane attached to the named session, since
  they share one PTY stream) — there is no way to target a different
  pane from a hook.
- The badge is advisory: a failed write costs one stderr line and exit 0,
  nothing else.

## Verification

- `cargo build/test/clippy -p ctl`: the canonical payload is pinned
  byte-exact, the tty/stdout sink choice is a pure function, and any
  argument is rejected by the parser.
- e2e `scripts/bin/e2e-notice.sh`: plain-pane notice, dedup, click-ack,
  active-tab suppression + re-badge, the printf fallback, a keeper
  attach traversal, and the usage error on extra arguments.

## opencoder hookup

`~/.opencoder/hooks.json` (hot-reloaded per event, `sh -c`, 3s timeout):

```json
{
  "turn_done": ["terminator-ctl notice"],
  "question":  ["terminator-ctl notice"]
}
```

Turn end / agent question each fire one OSC 9 into the pane's tty; the tab
badges while backgrounded. Requires `terminator-ctl` on PATH.
