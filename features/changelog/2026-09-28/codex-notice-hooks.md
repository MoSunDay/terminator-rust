Commit: 10c1d1515e5963d9441e8df0f9c0ab3aeecd667c

# Codex attention hooks on ssh_dev

`/usr/local/bin/terminator-ctl` is built from this checkout. Its `notice`
command emits OSC 9 to its controlling terminal (stdout if detached).
Codex command hooks detach their controlling tty and capture stdout, so they
must use `scripts/bin/codex-notice.py` instead of invoking `notice` directly.
The adapter emits only the canonical fixed notification bytes, never hook
stdin or conversation text. On Linux it locates the inherited tmux pane tty,
or the nearest same-UID ancestor tty. Within tmux it uses DCS passthrough;
managed tmux servers must have `allow-passthrough on`.

Configured root hooks: `Stop`, `PermissionRequest`, and `PreToolUse` matching
`request_user_input` / `request_user_input_async` (including namespace prefixes).
Existing reporting hooks are preserved. Trust is scoped to these reviewed
command definitions, using the hashes confirmed by the installed Codex runtime;
no global hook-trust bypass is enabled. `hooks/list` confirms all three are
trusted and enabled for /root and this repository.

New Codex sessions load these hooks. Existing runtimes keep their loaded hook
registry until a config reload; in `/hooks`, toggling a notice entry off/on
writes configuration with reloadUserConfig=true. Do not kill active sessions
to force a reload. The old desktop APP process predating OSC 9 still needs the
new APP instance for badges; installing the remote CLI cannot add a parser to
an already-running old desktop binary.

Validation: ctl + ipc-proto tests; `scripts/bin/e2e-notice.sh` verifies the blue
badge pixels, dedup, acknowledge, focused-tab suppression, and keeper traversal.
`scripts/bin/e2e-codex-notice.sh` adds detached-hook and tmux+keeper traversal
with silent hook stdout. Tests use private HOME/runtime and an isolated Xvfb.
