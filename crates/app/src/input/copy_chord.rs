//! Forward a TUI-owned copy chord without ever falling back to SIGINT or text.
use libghostty_vt::{
    key::{Action, Encoder, Event, Key, KittyKeyFlags, Mods},
    Terminal,
};

fn encode(term: &Terminal<'_, '_>, mods: Mods) -> anyhow::Result<Vec<u8>> {
    let mut encoder = Encoder::new()?;
    encoder.set_options_from_terminal(term);
    // tmux advertises extended keys but uses modifyOtherKeys on the outer
    // terminal, not kitty flags. An explicit copy action must still carry
    // all modifiers: tmux and crossterm accept CSI-u without a kitty push.
    encoder.set_kitty_flags(KittyKeyFlags::DISAMBIGUATE);
    let mut event = Event::new()?;
    event.set_action(Action::Press);
    event.set_key(Key::C);
    event.set_mods(mods);
    event.set_unshifted_codepoint('c');
    event.set_utf8(Some("c"));
    let mut bytes = Vec::new();
    encoder.encode_to_vec(&event, &mut bytes)?;
    // Legacy encoders may collapse Ctrl+Shift+C to ^C, or Super+C to 'c'.
    // Only an extended keyboard sequence can safely carry this copy action.
    if !bytes.starts_with(b"\x1b[") {
        bytes.clear();
    }
    Ok(bytes)
}

pub fn forward(session: &mut vt_pane::Session, mods: Mods) -> anyhow::Result<()> {
    // The managed tmux announces this key through an OSC capability marker.
    // Its native decoder otherwise maps Super to Alt and drops Ctrl+Shift.
    let bytes = if session
        .copy_transport
        .load(std::sync::atomic::Ordering::Acquire)
        != 0
    {
        b"\x1b[9001~".to_vec()
    } else {
        encode(&session.term, mods)?
    };
    log::debug!(
        "copy chord: {mods:?}, flags={:?}, encoded={bytes:?}",
        session.term.kitty_keyboard_flags()
    );
    if !bytes.is_empty() {
        vt_pane::task::write(session, &bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copy_never_becomes_interrupt_or_plain_text() {
        let mut term = Terminal::new(20, 4).unwrap();
        assert_eq!(encode(&term, Mods::SUPER).unwrap(), b"\x1b[99;9u");
        assert_eq!(
            encode(&term, Mods::CTRL | Mods::SHIFT).unwrap(),
            b"\x1b[99;6u"
        );
        term.vt_write(b"\x1b[>1u");
        assert_eq!(encode(&term, Mods::SUPER).unwrap(), b"\x1b[99;9u");
        assert_eq!(
            encode(&term, Mods::CTRL | Mods::SHIFT).unwrap(),
            b"\x1b[99;6u"
        );
    }
}
