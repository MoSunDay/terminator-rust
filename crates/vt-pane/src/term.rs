//! Plain-data render snapshots and terminal glue helpers.
//!
//! The UI layer never touches libghostty types directly; it consumes
//! [`Frame`] values produced from a render-state snapshot.

use libghostty_vt::render::{
    CellIteration, CellIterator, RenderState, RowIteration, RowIterator, Snapshot,
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
    pub inverse: bool,
    pub underline: bool,
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
}

/// A full render snapshot: everything needed to draw one frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    pub cols: u16,
    pub rows: u16,
    pub cursor: FrameCursor,
    /// Default colors when a cell has no explicit override.
    pub default_fg: Color,
    pub default_bg: Color,
    pub cursor_color: Option<Color>,
    pub cells: Vec<Vec<CellData>>,
}

/// Read one cell iteration into plain data.
fn cell_data(cell: &CellIteration<'_, '_>) -> CellData {
    let mut text = String::new();
    let _ = cell.graphemes_utf8(&mut text);
    let mut data = CellData {
        text,
        ..CellData::default()
    };
    data.fg = cell.fg_color().ok().flatten().map(Color::from);
    data.bg = cell.bg_color().ok().flatten().map(Color::from);
    data.wide = data
        .text
        .chars()
        .next()
        .is_some_and(|c| c.width().unwrap_or(1) > 1);
    if cell.has_styling().unwrap_or(false) {
        if let Ok(style) = cell.style() {
            data.bold = style.bold;
            data.italic = style.italic;
            data.inverse = style.inverse;
            data.underline = !matches!(style.underline, libghostty_vt::style::Underline::None);
        }
    }
    data
}

/// Iterate a snapshot into a plain `Frame`.
///
/// The iterator objects are reusable and owned by the caller's session;
/// this borrows them for the duration of the call.
pub fn snapshot_frame(
    terminal: &mut Terminal<'static, 'static>,
    render_state: &mut RenderState<'static>,
    row_it: &mut RowIterator<'static>,
    cell_it: &mut CellIterator<'static>,
) -> anyhow::Result<Frame> {
    let snapshot: Snapshot = render_state.update(terminal)?;
    let cols = snapshot.cols()?;
    let rows = snapshot.rows()?;
    let colors = snapshot.colors()?;
    let cursor_viewport = snapshot.cursor_viewport()?;
    let cursor_visible = snapshot.cursor_visible()?;

    let mut frame = Frame {
        cols,
        rows,
        cursor: FrameCursor {
            x: cursor_viewport.map_or(0, |c| c.x),
            y: cursor_viewport.map_or(0, |c| c.y),
            visible: cursor_visible && cursor_viewport.is_some(),
        },
        default_fg: colors.foreground.into(),
        default_bg: colors.background.into(),
        cursor_color: snapshot.cursor_color()?.map(Color::from),
        cells: Vec::with_capacity(rows as usize),
    };

    let mut rows_iter: RowIteration<'_, '_> = row_it.update(&snapshot)?;
    while let Some(row) = rows_iter.next() {
        let mut row_cells = Vec::with_capacity(cols as usize);
        let mut cells: CellIteration<'_, '_> = cell_it.update(row)?;
        while let Some(cell) = cells.next() {
            row_cells.push(cell_data(cell));
        }
        frame.cells.push(row_cells);
    }
    while frame.cells.len() < rows as usize {
        frame.cells.push(vec![CellData::default(); cols as usize]);
    }
    Ok(frame)
}
