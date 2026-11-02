# M4: chrome steady-state cost

## Problem
The chrome runs on EVERY frame (idle repaint, output burst, hover), and its
per-frame fixed costs had accumulated:
- the tab strip laid out each chip, then made a second per-chip pass just to
  scan a title for a notice badge;
- tab anchoring / attention collected and sorted every pane id per tab
  (`pane_ids()` allocates a `Vec`) when only the minimum or ANY match was
  needed;
- the pane-header X was an egui `Button`: a full widget (state machine,
  focus bookkeeping, text layout) rebuilt per pane per frame.

## Change Summary
1. **`ui/tabs/chips.rs` (new, 352 lines)**: `ChipInfo` + `chips::measure`
   produce every chip's label width, badge and hit zones in ONE pass over
   the tabs (`measure_is_one_pass_over_the_tabs` test). `chip_label`,
   `ghost_chip`, `dwell_step`, `clamp_scroll`, `ensure_visible`, `wheel_px`
   moved here; `ui/tabs.rs` is 740 -> 505 lines with call paths unchanged.
2. **`layout-tree/src/tree.rs`**: allocation-free recursive helpers
   `min_pane_id` and `any_pane_id` (short-circuits on the first match).
   `state::tab_anchor` and `attention::tab_has_notice` use them instead of
   collecting + sorting all pane ids.
3. **`ui/pane_header.rs` (443 lines)**: the close X is now
   `ui.interact(rect, Id::new("pane_close").with(pane), Sense::CLICK)` plus
   painter drawing (`vis.weak_bg_fill`, `Align2::CENTER_CENTER`) - no egui
   Button, and still non-focusable, so it cannot take the 0.36 Tab focus.

## Verification
- `cargo test --workspace` green (app 251, layout-tree 40, vt-pane 63, ...);
  new `min_pane_id`/`any_pane_id` unit tests match `sorted_pane_ids`.
- `cargo fmt --check` clean; clippy only the pre-existing
  `colors.rs` `type_complexity` baseline warning.
- Live Xvfb pixel probes of the self-drawn X match the old button colors;
  `e2e-idle-cpu.sh` passed with S1 idle 0 frames.
