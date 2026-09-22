# Config persistence moved to ~/.terminator-rust

## Problem
All persisted files (app `state.json`, remote `sessions.json`, ctl
`oc-links.json`, and the ipc.sock fallback when `XDG_RUNTIME_DIR` is
unset) lived under the XDG base-dir chain
`$XDG_CONFIG_HOME|$HOME/.config`/terminator-rust, and that resolution
was hand-duplicated in four crates (app/persist.rs, app/ipc/server.rs,
remote/registry.rs, ctl/sidecar.rs + ctl/uds.rs client probe).

## Fix
- New `paths` crate (pure functions): `config_dir()` =
  `$HOME/.terminator-rust` (relative `.terminator-rust` when HOME is
  unset), and `migrate_legacy(name)` which copies a legacy
  `~/.config/terminator-rust/<name>` forward on first use - an
  existing new file wins, failures are logged best-effort, so
  upgrades from the old layout keep their state.
- state.json / sessions.json / oc-links.json resolve via
  `migrate_legacy` (auto-migrated); the ipc.sock FALLBACK dir is
  `config_dir()` (no migration - runtime artifact). ctl's client-side
  default socket probes stay in sync with the server fallback.
- All e2e scripts now preset/assert `$HOME/.terminator-rust/state.json`
  (they run with a scratch HOME already; XDG_CONFIG_HOME exports
  dropped). XDG_RUNTIME_DIR remains the primary control-socket
  location, unchanged.
- Legacy files are COPIED, not deleted: downgrading to an older
  binary keeps working off the old location.

## Verified
- cargo build/test/clippy/fmt green across the workspace
  (paths crate: 5 unit tests incl. existing-new-file-preserved and
  nested-parent creation).
- Live migration check: a legacy `~/.config/terminator-rust/
  sessions.json` is copied to `~/.terminator-rust/` once, idempotent
  on rerun.
- scripts/bin/e2e-empty-restore.sh PASS end-to-end against the new
  path (preset restore + quit persistence + empty relaunch loop).
- Live deploy to 192.168.31.196 via scripts/bin/deploy-remote.sh:
  legacy state.json (theme kanagawa-wave) auto-copied to
  ~/.terminator-rust on first launch, legacy file preserved; a
  Ctrl+Shift+E split saved ONLY to the new path (old-path mtime
  unchanged), ctl list/capture smoke green.
