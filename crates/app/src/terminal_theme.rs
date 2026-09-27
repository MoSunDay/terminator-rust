//! Keep VT query answers and indexed colors aligned with the painted theme.
use anyhow::Result;
use libghostty_vt::{
    style::{Palette as VtPalette, RgbColor},
    Terminal,
};
use theme::{Palette, Rgb};

fn rgb(c: Rgb) -> RgbColor {
    RgbColor {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

pub fn apply(
    term: &mut Terminal<'static, 'static>,
    palette: &Palette,
    bg: Option<Rgb>,
) -> Result<()> {
    let fg = rgb(palette.foreground);
    let bg = rgb(bg.unwrap_or(palette.background));
    let cursor = rgb(palette.cursor);
    let colors = VtPalette(std::array::from_fn(|i| {
        rgb(theme::indexed_color(palette, i as u8))
    }));
    // Defaults preserve OSC overrides chosen by applications.
    if term.default_fg_color()? != Some(fg) {
        term.set_default_fg_color(Some(fg))?;
    }
    if term.default_bg_color()? != Some(bg) {
        term.set_default_bg_color(Some(bg))?;
    }
    if term.default_cursor_color()? != Some(cursor) {
        term.set_default_cursor_color(Some(cursor))?;
    }
    if term.default_color_palette()?.0 != colors.0 {
        term.set_default_color_palette(Some(colors))?;
    }
    Ok(())
}

pub fn sync(sessions: &mut crate::session_map::SessionMap, palette: &Palette, bg: Option<Rgb>) {
    for session in sessions.map.values_mut() {
        if let Err(error) = apply(&mut session.term, palette, bg) {
            log::warn!("terminal theme: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn theme_answers_osc_queries_and_preserves_application_overrides() {
        let mut term = Terminal::new(80, 24).expect("terminal");
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = output.clone();
        term.on_pty_write(move |_, bytes| sink.lock().unwrap().extend_from_slice(bytes))
            .unwrap();
        let palette = theme::builtin::dracula();
        let bg = Rgb {
            r: 16,
            g: 32,
            b: 48,
        };
        apply(&mut term, &palette, Some(bg)).unwrap();
        term.vt_write(b"\x1b]10;?\x07\x1b]11;?\x07\x1b]4;1;?\x07");
        let response = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(response.contains("10;rgb:f8f8/f8f8/f2f2"), "{response:?}");
        assert!(response.contains("11;rgb:1010/2020/3030"), "{response:?}");
        assert!(response.contains("4;1;rgb:ffff/5555/5555"), "{response:?}");
        term.vt_write(b"\x1b]11;#123456\x07\x1b]4;1;#abcdef\x07");
        let next = theme::builtin::kanagawa_wave();
        apply(&mut term, &next, None).unwrap();
        assert_eq!(term.default_bg_color().unwrap(), Some(rgb(next.background)));
        assert_eq!(
            term.bg_color().unwrap(),
            Some(RgbColor {
                r: 18,
                g: 52,
                b: 86
            })
        );
        assert_eq!(
            term.color_palette().unwrap().0[1],
            RgbColor {
                r: 171,
                g: 205,
                b: 239
            }
        );
    }
}
