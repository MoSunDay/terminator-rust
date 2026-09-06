//! Byte-level ground truth for key encoding when the child pushes kitty
//! keyboard flags. The child (`printf flags; stty raw -echo; cat > file`)
//! records its raw stdin, so each case asserts the exact pty bytes.

use std::time::Duration;

use libghostty_vt::key::{Action as GAction, Event as GEvent, Key as GKey, Mods as GMods};
use vt_pane::task as vtask;
use vt_pane::SessionOpts;

/// Spawn a child that pushes `push` (a printf-style escape) and then dumps
/// its raw stdin to `bytes_path`; send one configured key; return the bytes.
fn capture_key<F>(bytes_path: &str, push: &str, cfg: F) -> Vec<u8>
where
    F: FnOnce(&mut GEvent),
{
    let opts = SessionOpts::command(
        20,
        5,
        vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("printf '{}'; stty raw -echo; cat > {bytes_path}", push),
        ],
    );
    let mut s = vtask::spawn_session(&opts).expect("spawn pty");
    std::thread::sleep(Duration::from_millis(400));
    for _ in 0..8 {
        let _ = vtask::pump(&mut s);
        std::thread::sleep(Duration::from_millis(50));
    }
    vtask::send_key(&mut s, cfg).expect("send_key");
    let _ = vtask::pump(&mut s);
    std::thread::sleep(Duration::from_millis(300));
    let _ = vtask::pump(&mut s);
    std::fs::read(bytes_path).unwrap_or_default()
}

fn press(key: GKey, mods: GMods) -> impl FnOnce(&mut GEvent) {
    move |ev: &mut GEvent| {
        ev.set_action(GAction::Press);
        ev.set_key(key);
        ev.set_mods(mods);
        ev.set_utf8::<&str>(None);
    }
}

/// Regression: a child that pushes kitty keyboard flags (crossterm apps
/// like opencoder push \e[>7u at startup) used to silence plain Ctrl+letter
/// presses entirely - the pinned encoder's kitty path bails on textless
/// events, killing ^C/^D/^Z delivery. send_key must fall back to the legacy
/// C0 byte for exactly those combos and leave other shapes untouched.
#[test]
fn ctrl_letter_falls_back_to_legacy_c0_under_kitty_flags() {
    const ESC: u8 = 27;

    // flags pushed (opencoder shape 7): legacy C0 fallback must kick in.
    assert_eq!(
        capture_key(
            "/tmp/probe-cd7.bin",
            "\\033[>7u",
            press(GKey::D, GMods::CTRL)
        ),
        vec![4],
        "Ctrl+D under kitty flags 7 must arrive as the C0 byte"
    );
    // a different letter, a different flag set: same fallback contract.
    assert_eq!(
        capture_key(
            "/tmp/probe-cc1.bin",
            "\\033[>1u",
            press(GKey::C, GMods::CTRL)
        ),
        vec![3],
        "Ctrl+C under kitty flags 1 must arrive as the C0 byte"
    );
    // no flags at all: plain legacy byte, unchanged.
    assert_eq!(
        capture_key("/tmp/probe-cd0.bin", "", press(GKey::D, GMods::CTRL)),
        vec![4]
    );

    // Non-C0 keys keep their shapes under kitty flags: Enter stays the
    // 0x0d byte and Escape stays its disambiguated CSI-u form.
    assert_eq!(
        capture_key(
            "/tmp/probe-en7.bin",
            "\\033[>7u",
            press(GKey::Enter, GMods::empty())
        ),
        vec![13]
    );
    assert_eq!(
        capture_key(
            "/tmp/probe-esc7.bin",
            "\\033[>7u",
            press(GKey::Escape, GMods::empty())
        ),
        vec![ESC, b'[', b'2', b'7', b'u']
    );
}
