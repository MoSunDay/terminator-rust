# macOS IME candidate-panel stability during composition

Date: 2026-09-29

## Symptom

On macOS, mid-composition (pinyin preedit visible and still updating), the
candidate panel would probabilistically disappear. Our own preedit overlay
kept painting, and only deactivating/reactivating the input method brought
the panel back.

## Root-cause chain

1. `grid::cursor_rect` returns `None` when the VT cursor is hidden
   (DECTCEM - vim/less/TUI) or its coordinates fall outside the grid
   (transition frames during a live resize). Brief dead-session frames and
   anchor-pane changes (tab/pane switch) had the same effect: one frame of
   `PlatformOutput.ime = None`.
2. egui-winit maps `o.ime = None` to `set_ime_allowed(false)`, and a
   `should_interrupt_composition` flag to `set_ime_allowed(false);
   set_ime_allowed(true)` (egui-winit 0.36 lib.rs:1157). winit 0.30 macOS
   implements `set_ime_allowed(false)` by silently clearing the NSView-side
   marked text WITHOUT notifying the input-method engine (no `unmarkText` /
   `discardMarkedText`); egui-winit ignores the resulting `Ime::Disabled`.
   The engine keeps its composition session and keeps calling
   `setMarkedText` (preedit still updates), but engine and client marked
   text are now desynced: the candidate panel closes and never self-heals -
   only an input-source deactivate/activate (`activateServer`) rebuilds it.
   Exactly the observed symptom.
3. Our `platform_ime_output` set `should_interrupt_composition = true` on
   every anchor-pane change - so ANY tab/pane switch fired the same
   false/true pair. On X11 that pair destroys and recreates the XIC and
   never re-focuses it (the same mechanism as the 2026-09-25
   `request_focus` XIM bug): a latent sibling bug - after any tab switch
   the next keys bypassed the input method.
4. Secondary factor: PTY output moving the cursor during composition kept
   changing the anchor rect -> repeated `invalidateCharacterCoordinates`
   -> some input methods drift/dismiss the panel (one source of the
   "probabilistic" behavior).

## Fix (app-side, pure functions)

- `state.rs`: `WindowUi.ime_anchor: Option<egui::Rect>` - composition
  anchor latch (transient UI state, never persisted).
- `input/ime.rs`:
  - `pane_anchor(composing, live, latched)` - while composing the anchor
    FREEZES on the first live cursor cell (`latched.or(live)`), so a
    transiently missing live rect or a moving cursor cannot change it;
    without a composition the live rect is used and the latch clears.
  - `effective_target(composing, live_pane, live_rect, last_pane, latch)` -
    while composing, a frame with no live anchor falls back to
    `(ime_last_pane, ime_anchor)`, so a transiently missing live anchor
    never flips `o.ime` to None mid-composition. One None path remains
    on purpose: if the fallback pane's own session died (focused shell
    exited mid-composition) IME still turns off - correct, since
    `vtask::write` on a closed session is a silent no-op.
  - `platform_ime_output` no longer takes a pane-change flag:
    `should_interrupt_composition` is always false. winit has no real
    interrupt API; the emulated pair only causes harm on both platforms.
    A composition follows its latched anchor; the commit lands on whatever
    pane is focused when it arrives (keyboard.rs) - correct terminal
    behavior.
- `render/screen.rs`: the focused pane computes `live = cursor_rect(...)`,
  derives `(anchor, latch) = ime::pane_anchor(...)` (write-back), paints
  the preedit at the SAME latched anchor (composition no longer drifts
  with output), and records `ime_pane` even when the live rect is missing.

Invariant achieved: while composing with a live pane, `o.ime` stays
`Some` across live-anchor-missing frames, its rect is bit-stable, and no
interrupt is ever requested - both macOS triggers
(marked-text erasure, panel invalidation) are gone, and the X11
tab-switch XIC kill is fixed as a side effect.

## Follow-up (same day): first-frame latch seeding

Review of the latch found one residual None path: the IDLE branch of
`pane_anchor` returned `(live, None)`, clearing the latch every frame
without a composition. A composition whose very FIRST frame already
lacked a live cursor - the VT cursor turned hidden between the keypress
and the first Preedit frame, e.g. a vim scroll or TUI mode switch
racing the first CJK key - therefore found no latch, produced
`o.ime = None` on the first composition frame, and hit the same macOS
marked-text erasure path the latch was built to close.

Fix: idle frames now RE-SEED the latch with the last live cursor rect
(`pane_anchor` idle branch returns `(live, live)`), so the race frame
starts the composition anchored at the last live cell instead of
flipping to None. Semantics while composing are unchanged (frozen,
bit-stable, `latched.or(live)`).

Deliberately NOT covered (narrowed claim): a cursor that has been
hidden since BEFORE the composition began (less from the start) sees
`o.ime = None` while idle, egui-winit keeps the platform IME off, and
the first CJK key arrives as raw latin - no composition starts. That
is the pre-existing hidden-cursor limitation ("composition cannot
begin"), NOT the candidate-panel death: the panel only dies when a
None flip interrupts an in-flight composition, which the latch now
prevents on every reachable path.

## Verification

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`,
  `cargo test --workspace` (new unit tests: latch behavior, composition
  fallback, never-interrupt).
- Cross-check `cargo check --workspace --all-targets --target
  aarch64-apple-darwin` (zig cc).
- Linux regression: `scripts/bin/e2e-ime.sh` (M1-M7: anchoring, rename
  editors, commit round-trip unaffected).
- macOS manual checklist (needs a real IME): compose while a pane streams
  output (`while sleep 0.2; do date; done`), compose during a live window
  resize, compose in vim right after the cursor turned hidden (scroll /
  ctrl-o jump then a CJK key) - the panel must stay up, anchored at the
  last live cell; in less (cursor hidden from the start) a CJK key must
  arrive as raw latin with NO composition and NO panel death (cannot
  begin, not death); after a tab switch, CJK input must still compose
  (not raw latin).
