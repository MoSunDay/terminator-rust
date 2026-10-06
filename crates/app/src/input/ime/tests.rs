use super::*;

fn rect() -> Rect {
    Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(8.0, 16.0))
}

fn rect_at(y: f32) -> Rect {
    Rect::from_min_size(egui::pos2(10.0, y), egui::vec2(8.0, 16.0))
}

#[test]
fn dead_pane_or_missing_cursor_disables_ime() {
    assert!(platform_ime_output(false, Some(rect()), true).is_none());
    assert!(platform_ime_output(true, None, true).is_none());
}

#[test]
fn live_pane_anchors_terminal_ime() {
    let out =
        platform_ime_output(true, Some(rect()), true).expect("live pane with cursor enables IME");
    assert_eq!(out.purpose, egui::IMEPurpose::Terminal);
    assert_eq!(out.cursor_rect, rect());
    assert!(!out.should_interrupt_composition);
}

#[test]
fn xim_spot_anchor_sits_at_the_caret_baseline() {
    // X11: rect.min = bottom-left of the caret cell (the XIM spot
    // convention shared with xterm/GTK), size carried through.
    let anchored = anchor_rect(rect(), true);
    assert_eq!(anchored.min, egui::pos2(10.0, 36.0));
    assert_eq!(anchored.size(), rect().size());
    // Full-rect backends (macOS, Wayland) keep the true caret cell.
    assert_eq!(anchor_rect(rect(), false), rect());
    let out = platform_ime_output(true, Some(rect()), true).expect("spot anchor keeps IME alive");
    assert_eq!(out.rect.min, egui::pos2(10.0, 36.0));
    assert_eq!(out.cursor_rect, rect());
}

#[test]
fn ime_output_never_interrupts_composition() {
    // Even a pane switch mid-composition must not ask the platform
    // to interrupt: egui-winit implements the flag as a
    // set_ime_allowed(false)/(true) pair, which erases macOS marked
    // text and rebuilds an unfocused XIC on X11.
    let out = platform_ime_output(true, Some(rect()), false).expect("anchored");
    assert!(!out.should_interrupt_composition);
}

#[test]
fn composition_tracks_cursor_updates_and_remembers_the_latest_position() {
    // Composing with no latch yet: the live rect latches.
    let (anchor, latch) = pane_anchor(true, Some(rect_at(20.0)), None);
    assert_eq!(anchor, Some(rect_at(20.0)));
    assert_eq!(latch, Some(rect_at(20.0)));
    // A delayed echo or resize moves both the overlay and native popup.
    let (anchor, latch) = pane_anchor(true, Some(rect_at(40.0)), latch);
    assert_eq!(anchor, Some(rect_at(40.0)));
    assert_eq!(latch, anchor);
    // A transiently missing cursor keeps the newest position, not the
    // first composition cell. No None flip or interrupt is introduced.
    let (anchor, _) = pane_anchor(true, None, latch);
    assert_eq!(anchor, Some(rect_at(40.0)));
    for xim in [false, true] {
        let out = platform_ime_output(true, anchor, xim).expect("IME stays enabled");
        assert!(!out.should_interrupt_composition);
    }
}

#[test]
fn idle_frames_reseed_the_latch_with_the_live_rect() {
    // Idle frame with a live cursor: passes through AND seeds the
    // latch with that rect (composition-start insurance).
    let (anchor, latch) = pane_anchor(false, Some(rect_at(40.0)), Some(rect_at(20.0)));
    assert_eq!(anchor, Some(rect_at(40.0)));
    assert_eq!(latch, Some(rect_at(40.0)), "idle frames reseed, not clear");
    // Idle frame with a hidden cursor: no anchor and no seed -
    // the platform IME is off, a composition cannot begin there.
    let (anchor, latch) = pane_anchor(false, None, Some(rect_at(20.0)));
    assert_eq!(anchor, None);
    assert_eq!(latch, None);
}

#[test]
fn composition_starting_on_a_hidden_cursor_uses_the_last_live_seed() {
    // The residual 2026-09-29 race: cursor live on the frame
    // before the keypress, hidden by the first Preedit frame. The
    // idle seed must anchor the composition so `o.ime` never
    // flips to None mid-composition (the macOS marked-text
    // erasure path).
    let (_, seed) = pane_anchor(false, Some(rect_at(40.0)), None);
    assert_eq!(seed, Some(rect_at(40.0)));
    let (anchor, latch) = pane_anchor(true, None, seed);
    assert_eq!(anchor, Some(rect_at(40.0)));
    assert_eq!(latch, Some(rect_at(40.0)));
    // Composing with neither latch nor live (cursor hidden since
    // before the composition began): nothing to anchor on.
    let (anchor, latch) = pane_anchor(true, None, None);
    assert_eq!(anchor, None);
    assert_eq!(latch, None);
}

#[test]
fn composition_falls_back_to_remembered_pane_and_latch() {
    let pane = 7u64;
    // Live anchor missing while composing: remembered pane + latch.
    assert_eq!(
        effective_target(true, None, None, Some(pane), Some(rect())),
        (Some(pane), Some(rect()))
    );
    // Live anchor present: passes through unchanged (latch already
    // applied by screen.rs via pane_anchor).
    assert_eq!(
        effective_target(true, Some(pane), Some(rect()), Some(8), None),
        (Some(pane), Some(rect()))
    );
    // Not composing: live values pass through, even when None.
    assert_eq!(
        effective_target(false, None, None, Some(pane), Some(rect())),
        (None, None)
    );
}

#[test]
fn new_composition_uses_current_cursor_instead_of_previous_frame_seed() {
    let (_, seed) = pane_anchor(false, Some(rect()), None);
    // A PTY echo, cursor move, or resize arrives with the first Preedit.
    let current = rect().translate(egui::vec2(32.0, 16.0));
    let (anchor, latch) = pane_anchor(true, Some(current), seed);
    assert_eq!(anchor, Some(current));
    // Subsequent real cursor moves must also be reflected.
    assert_eq!(pane_anchor(true, Some(rect()), latch).0, Some(rect()));
}

#[test]
fn consecutive_compositions_follow_both_immediate_and_delayed_echo() {
    use crate::state::{fresh_state, ui_state};
    use egui::{Event, ImeEvent, RawInput};

    let preedit = |text: &str| {
        Event::Ime(ImeEvent::Preedit {
            text: text.into(),
            active_range_chars: None,
        })
    };
    // Backends may send an empty preedit, a commit, or both before
    // starting the next word, all within a single egui event batch.
    for ending in [
        vec![preedit("")],
        vec![Event::Ime(ImeEvent::Commit("汉字".into()))],
        vec![preedit(""), Event::Ime(ImeEvent::Commit("汉字".into()))],
    ] {
        let mut st = fresh_state();
        st.windows[0].ui.ime = Some("hanzi".into());
        st.windows[0].ui.ime_anchor = Some(rect());
        let ctx = Context::default();
        ctx.begin_pass(RawInput {
            events: ending.into_iter().chain([preedit("ni")]).collect(),
            ..Default::default()
        });
        crate::input::keyboard::handle(
            &ctx,
            &mut st,
            &mut crate::session_map::session_map(),
            &mut ui_state(),
            &mut false,
        );
        ctx.end_pass().textures_delta.clear();
        let ui = &st.windows[0].ui;
        assert_eq!(ui.ime.as_deref(), Some("ni"));
        let next_cell = rect().translate(egui::vec2(32.0, 0.0));
        for first_cursor in [rect(), next_cell] {
            let (_, pending) = pane_anchor(true, Some(first_cursor), ui.ime_anchor);
            let (after_echo, _) = pane_anchor(true, Some(next_cell), pending);
            assert_eq!(after_echo, Some(next_cell));
        }
        // If the cursor disappears at this boundary, keep IME enabled.
        let (fallback, _) = pane_anchor(true, None, ui.ime_anchor);
        let out = platform_ime_output(true, fallback, false).expect("fallback stays enabled");
        assert!(!out.should_interrupt_composition);
    }
}
