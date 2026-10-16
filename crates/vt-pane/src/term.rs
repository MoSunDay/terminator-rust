//! Plain-data render snapshots and terminal glue helpers.
//!
//! The UI layer never touches libghostty types directly; it consumes
//! [`Frame`] values produced from a render-state snapshot.

use libghostty_vt::render::{
    CellIteration, CellIterator, Dirty, RenderState, RowIteration, RowIterator, Snapshot,
};
use libghostty_vt::style::RgbColor;
use libghostty_vt::Terminal;
use unicode_width::UnicodeWidthChar;

/// One grid cell, fully resolved to plain data.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CellData {
    /// Grapheme text (may be multi-codepoint; empty for blank cells).
    pub text: String,
    /// Cell-wide (double-width CJK etc.): wide cells are followed by
    /// a trailing empty cell; renderers should skip the tail.
    pub wide: bool,
    /// Explicit fg color if any (already palette-resolved by libghostty).
    pub fg: Option<Color>,
    /// Explicit bg color if any.
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub invisible: bool,
    pub inverse: bool,
    pub underline: bool,
    pub strikethrough: bool,
    /// Inside the terminal's active selection (render as highlighted).
    pub selected: bool,
}

/// Plain RGB color, independent of libghostty.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl From<RgbColor> for Color {
    fn from(c: RgbColor) -> Self {
        Color {
            r: c.r,
            g: c.g,
            b: c.b,
        }
    }
}

impl From<Color> for RgbColor {
    fn from(c: Color) -> Self {
        RgbColor {
            r: c.r,
            g: c.g,
            b: c.b,
        }
    }
}

/// Cursor position/visibility in the frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrameCursor {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
    /// The cursor has filled the last cell; wrapping still depends on DECAWM.
    pub pending_wrap: bool,
    /// DECAWM: wide input with too little space also wraps before writing.
    pub autowrap: bool,
}

/// A full render snapshot: everything needed to draw one frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    pub cols: u16,
    pub rows: u16,
    pub cursor: FrameCursor,
    /// Effective defaults, including OSC overrides, for cells without explicit colors.
    pub default_fg: Color,
    pub default_bg: Color,
    pub cursor_color: Option<Color>,
    pub cells: Vec<Vec<CellData>>,
    /// Some cell carries the blink attribute: the renderer must keep
    /// scheduling blink edges for this pane even without new output.
    pub has_blink: bool,
}

/// Reset a cached cell for reuse: keeps the text buffer's allocation
/// (ghostty writes the next grapheme cluster straight back into it) and
/// drops every derived field, i.e. the in-place twin of
/// `CellData::default()`.
///
/// In-place updates need this because a cell that lost its text/style
/// between two frames must not keep the stale flags of the previous one.
fn reset_cell(out: &mut CellData) {
    let text = std::mem::take(&mut out.text);
    *out = CellData {
        text,
        ..CellData::default()
    };
}

/// Fill one grid cell from the current cell iteration, in place.
///
/// `selected` is the cell's column inside the row-local selection range
/// (see [`rebuild_row`]): the engine answers that range once per row, so
/// no per-cell `is_selected()` FFI call is needed.
///
/// PERF: on the in-place path the only allocations left are (a) the text
/// buffer of a cell that needs more room than before (the cluster is
/// written into the existing `String`, which keeps its capacity) and (b)
/// nothing else - every other field is a plain copy. An all-background
/// cell costs 2 FFI calls (graphemes + bg) instead of 5.
fn cell_data_into(cell: &CellIteration<'_, '_>, out: &mut CellData, selected: bool) {
    // clear() first: an empty cell (graphemes_utf8 writes len 0 and keeps
    // the buffer) and the error path (the C API leaves the buffer
    // unmodified) both end up as "" without allocating.
    out.text.clear();
    let _ = cell.graphemes_utf8(&mut out.text);
    out.wide = out
        .text
        .chars()
        .next()
        .is_some_and(|c| c.width().unwrap_or(1) > 1);
    let styled = cell.has_styling().unwrap_or(false);
    // An empty, unstyled cell carries no ink, so its fg color cannot be
    // observed: the renderer paints ink only for non-empty text, and an
    // inverse-video cell's background falls back to `default_fg` whether
    // or not `fg` is None. Skip the FFI call; bg stays unconditional
    // (BCE background of erased cells).
    out.fg = if out.text.is_empty() && !styled {
        None
    } else {
        cell.fg_color().ok().flatten().map(Color::from)
    };
    out.bg = cell.bg_color().ok().flatten().map(Color::from);
    out.selected = selected;
    if styled {
        if let Ok(style) = cell.style() {
            out.bold = style.bold;
            out.italic = style.italic;
            out.faint = style.faint;
            out.blink = style.blink;
            out.invisible = style.invisible;
            out.inverse = style.inverse;
            out.underline = !matches!(style.underline, libghostty_vt::style::Underline::None);
            out.strikethrough = style.strikethrough;
        }
    }
}

/// Rebuild ONE cached grid row in place from the row iteration at its
/// current position, padding/trimming to `cols` cells.
///
/// `row.selection()` is a single FFI call for the whole row's selected
/// column range (`None` = nothing selected in this row), replacing one
/// `is_selected()` call per cell. Cells the engine does not report are
/// reset to blank, so a rebuilt row never keeps stale text.
///
/// Returns whether the row holds a blinking cell, letting the caller
/// maintain [`Frame::has_blink`] without a grid-wide scan.
fn rebuild_row(
    cell_it: &mut CellIterator<'static>,
    row: &RowIteration<'static, '_>,
    out: &mut Vec<CellData>,
    cols: u16,
) -> anyhow::Result<bool> {
    let selection = row.selection()?;
    let width = cols as usize;
    let mut blink = false;
    let mut x = 0usize;
    let mut cells: CellIteration<'_, '_> = cell_it.update(row)?;
    while let Some(cell) = cells.next() {
        if x >= out.len() {
            out.push(CellData::default());
        }
        let col = x as u16;
        let selected = selection.is_some_and(|s| col >= s.start_x && col <= s.end_x);
        reset_cell(&mut out[x]);
        cell_data_into(cell, &mut out[x], selected);
        blink |= out[x].blink;
        x += 1;
    }
    if out.len() < width {
        out.resize_with(width, CellData::default);
    }
    // Grid width is `cols`; anything the engine under-reported is blank
    // (the row may still hold the previous frame's text in those slots).
    out.truncate(width);
    for cell in out.iter_mut().skip(x) {
        reset_cell(cell);
    }
    Ok(blink)
}

/// Iterate a snapshot into a plain `Frame`, updating `frame` IN PLACE.
///
/// `force_full` rebuilds every grid row (used when the caller cannot
/// trust the engine's dirty tracking: selection, viewport scroll, theme
/// sync, resize). Otherwise the engine's two dirty layers decide the
/// work: a `Full` snapshot rebuilds everything, `Partial` rebuilds only
/// the rows whose per-row flag is set, `Clean` skips the grid entirely
/// (the scalars below are always re-read). Both layers are cleared here -
/// they are independent, and an uncleared flag would only ever cause
/// EXTRA work, never a stale cell.
pub(crate) fn snapshot_frame_into(
    terminal: &mut Terminal<'static, 'static>,
    render_state: &mut RenderState<'static>,
    row_it: &mut RowIterator<'static>,
    cell_it: &mut CellIterator<'static>,
    frame: &mut Frame,
    force_full: bool,
) -> anyhow::Result<()> {
    let pending_wrap = terminal.is_cursor_pending_wrap()?;
    let autowrap = terminal.mode(libghostty_vt::terminal::Mode::WRAPAROUND)?;
    let snapshot: Snapshot = render_state.update(terminal)?;
    let cols = snapshot.cols()?;
    let rows = snapshot.rows()?;
    let colors = snapshot.colors()?;
    let cursor_viewport = snapshot.cursor_viewport()?;
    let cursor_visible = snapshot.cursor_visible()?;
    let cursor_color = snapshot.cursor_color()?.map(Color::from);

    // A different grid geometry cannot be patched row-wise: the cached
    // cells describe another shape (and `frame.cells` is empty before the
    // first snapshot).
    let reshape =
        frame.cells.len() != rows as usize || frame.cells.iter().any(|r| r.len() != cols as usize);
    let full = force_full || reshape;
    // `Full` means the engine cannot point at individual rows (scroll,
    // clear, erase-to-end): every row must be rebuilt. `Partial` marks the
    // rows exactly; `Clean` touches none (the scalars above stay fresh).
    let dirty = if full { Dirty::Full } else { snapshot.dirty()? };
    let every_row = full || matches!(dirty, Dirty::Full);
    let some_rows = matches!(dirty, Dirty::Partial);

    frame.cols = cols;
    frame.rows = rows;
    frame.cursor = FrameCursor {
        x: cursor_viewport.map_or(0, |c| c.x),
        y: cursor_viewport.map_or(0, |c| c.y),
        visible: cursor_visible && cursor_viewport.is_some(),
        pending_wrap,
        autowrap,
    };
    frame.default_fg = colors.foreground.into();
    frame.default_bg = colors.background.into();
    frame.cursor_color = cursor_color;

    if reshape {
        // Reuse the row/string capacity that is still valid.
        frame.cells.truncate(rows as usize);
        frame
            .cells
            .resize_with(rows as usize, || vec![CellData::default(); cols as usize]);
    }

    let mut rebuilt = false;
    let mut rebuilt_blink = false;
    let mut y = 0usize;
    let mut rows_iter: RowIteration<'_, '_> = row_it.update(&snapshot)?;
    while let Some(row) = rows_iter.next() {
        let Some(row_cells) = frame.cells.get_mut(y) else {
            break; // defensive: never index past the cached grid
        };
        if row_cells.len() != cols as usize {
            row_cells.resize_with(cols as usize, CellData::default);
        }
        let row_dirty = every_row || (some_rows && row.dirty()?);
        if row_dirty {
            rebuilt_blink |= rebuild_row(cell_it, row, row_cells, cols)?;
            // Clear the per-row layer for every row we touched (the global
            // flag below does NOT clear it - the layers are independent).
            row.set_dirty(false)?;
            rebuilt = true;
        }
        y += 1;
    }
    // Defensive: rows the engine did not report are blank, not stale.
    for row in frame.cells.iter_mut().skip(y) {
        for cell in row.iter_mut() {
            reset_cell(cell);
        }
    }
    if every_row {
        // Every row was rebuilt (or blanked): the accumulated flag is exact.
        frame.has_blink = rebuilt_blink;
    } else if rebuilt_blink {
        frame.has_blink = true;
    } else if rebuilt && frame.has_blink {
        // A row that blinked in the cached frame may have just been
        // repainted without blink; skipped rows keep their flags, so only
        // a scan can tell. It runs while a blink is actually on screen.
        frame.has_blink = frame.cells.iter().flatten().any(|c| c.blink);
    }
    snapshot.set_dirty(Dirty::Clean)?;
    Ok(())
}

/// Build a FRESH frame (one-shot callers, tests): full rebuild into a new
/// `Frame`. The session path uses [`snapshot_frame_into`] on a cached
/// frame so clean rows are skipped entirely.
pub fn snapshot_frame(
    terminal: &mut Terminal<'static, 'static>,
    render_state: &mut RenderState<'static>,
    row_it: &mut RowIterator<'static>,
    cell_it: &mut CellIterator<'static>,
) -> anyhow::Result<Frame> {
    let mut frame = Frame::default();
    snapshot_frame_into(terminal, render_state, row_it, cell_it, &mut frame, true)?;
    Ok(frame)
}

#[cfg(test)]
#[path = "term/tests.rs"]
mod tests;
