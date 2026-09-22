# macOS as a first-class target

Date: 2026-09-21 · Status: verified (aarch64-apple-darwin cross-check from
Linux via zig cc green: `cargo check --workspace --all-targets --target
aarch64-apple-darwin`; Linux gates clean; CI `build-test-macos` job wired
in .github/workflows/ci.yml - same fmt/clippy/test gates on macos-15)

## What changed

The workspace now builds natively on macOS (aarch64-apple-darwin); every
glibc/Linux-only assumption was replaced by a cfg'd or portable path:

- vt-pane pty.rs: slave name via `ptsname_r` on Linux, `TIOCPTYGNAME`
  ioctl on macOS (const 0x80807463 - the libc crate does not export it);
  child locale `C.UTF-8` on Linux, `en_US.UTF-8` on macOS (no C.UTF-8
  there). task.rs reads `io::Error::last_os_error()` instead of
  glibc-only `__errno_location`.
- ctl: /proc discovery split into procfs_linux.rs / procfs_macos.rs;
  macOS uses libproc (proc_listallpids + proc_pidinfo SHORTBSDINFO/
  VNODEPATHINFO + proc_pidfdinfo PROC_PIDFDVNODEPATHINFO=2, custom
  const), same semantics incl. the multi-store refusal. libc is now a
  macOS-only dep of ctl.
- app: x11-dl is linux-gated in Cargo.toml; pointer_poll (XQueryPointer)
  is absent on macOS -> edge resize falls back to the event-driven path.
- build tooling: scripts/bin/zig falls back dist-packages -> PATH zig ->
  `python3 -m ziglang`; fetch-vendor.sh sed is BSD-safe (tmp+mv).
  Vendored ghostty artifacts are PER-OS - re-run scripts/fetch-vendor.sh
  on each platform, never copy third_party/ between them.

e2e scripts (Xvfb/xdotool/openbox) stay Linux-only. Cross-check recipe
from Linux is recorded in agents.md Build/e2e (PKG_CONFIG_ALLOW_CROSS=1
or libghostty-vt-sys's build.rs falls back to a vendored zig build and
dies).
