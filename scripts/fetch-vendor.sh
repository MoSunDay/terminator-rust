#!/usr/bin/env bash
# Build the pinned libghostty-vt static library once and harvest it into
# third_party/ for pkg-config consumption. Normal cargo builds never need
# zig again afterwards.
#
# Usage: scripts/fetch-vendor.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ZIG="$ROOT/scripts/bin/zig"
OUT="$ROOT/third_party/libghostty-vt"
WORK="$(mktemp -d /tmp/gvt-vendor.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

command -v "$ZIG" >/dev/null 2>&1 || {
  echo "zig not found. Install with: pip install -i https://pypi.org/simple ziglang==0.16.0" >&2
  exit 1
}
export PATH="$(dirname "$ZIG"):$PATH"

mkdir -p "$WORK/src" "$OUT"
cat > "$WORK/Cargo.toml" <<'TOML'
[package]
name = "gvt-vendor"
version = "0.0.0"
edition = "2021"

[dependencies]
libghostty-vt-sys = { git = "https://github.com/Uzaaft/libghostty-rs", rev = "8272abe" }

[workspace]
TOML
cat > "$WORK/src/main.rs" <<'RS'
fn main() {
    // Touch one symbol so the static archive is actually linked in.
    let p: usize = libghostty_vt_sys::ghostty_terminal_free as *const () as usize;
    println!("linked: {p:?}");
}
RS

# Vendored build (default feature): fetches ghostty at the pinned commit and
# builds libghostty-vt with zig.
(cd "$WORK" && cargo build --release)

# Harvest the installed tree out of cargo's OUT_DIR.
INSTALL="$(find "$WORK/target" -type d -name ghostty-install | head -1)"
test -n "$INSTALL"
rm -rf "$OUT/lib" "$OUT/include" "$OUT/share"
cp -r "$INSTALL/lib" "$INSTALL/include" "$INSTALL/share" "$OUT/"

# Rewrite pkg-config prefixes to the harvested location.
for pc in "$OUT"/share/pkgconfig/*.pc; do
  sed -i "s|^prefix=.*|prefix=$OUT|" "$pc"
done

echo "harvested to $OUT:"
ls -la "$OUT/lib" "$OUT/share/pkgconfig"
