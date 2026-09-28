//! Resolve explicit OSC 8 links or visible HTTP(S) text at a viewport cell.
use libghostty_vt::{
    terminal::{Point, PointCoordinate},
    Terminal,
};

const MAX_URL: usize = 8192;

fn web_url(text: &str) -> Option<String> {
    let rest = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))?;
    if rest.is_empty()
        || text.len() > MAX_URL
        || text.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    Some(text.to_owned())
}

fn point(x: u16, y: u32) -> Point {
    Point::Viewport(PointCoordinate { x, y })
}

/// Hyperlink resolution only runs for a deliberate modifier-click, never PTY output.
pub fn at(term: &Terminal<'_, '_>, x: u16, y: u32) -> Option<String> {
    let cell = term.grid_ref(point(x, y)).ok()?;
    let mut uri = [0; MAX_URL];
    if let Ok(n) = cell.hyperlink_uri(&mut uri) {
        if n > 0 {
            return web_url(std::str::from_utf8(&uri[..n]).ok()?);
        }
    }
    let cols = term.cols().ok()?;
    let rows = u32::from(term.rows().ok()?);
    if x >= cols || y >= rows {
        return None;
    }
    let mut first = y;
    while first > 0 && (y - first) * u32::from(cols) < MAX_URL as u32 {
        if !term
            .grid_ref(point(0, first))
            .ok()?
            .row()
            .ok()?
            .is_wrap_continuation()
            .ok()?
        {
            break;
        }
        first -= 1;
    }
    let mut line = String::new();
    let mut clicked = None;
    for row in first..rows {
        for col in 0..cols {
            if col == x && row == y {
                clicked = Some(line.len());
            }
            let cell = term.grid_ref(point(col, row)).ok()?;
            let mut chars = ['\0'; 32];
            let n = cell.graphemes(&mut chars).ok()?;
            if n == 0 {
                line.push(' ');
            } else {
                line.extend(&chars[..n]);
            }
        }
        if line.len() > MAX_URL * 2 {
            return None;
        }
        if !term
            .grid_ref(point(0, row))
            .ok()?
            .row()
            .ok()?
            .is_wrapped()
            .ok()?
        {
            break;
        }
    }
    text_at(&line, clicked?)
}

fn text_at(line: &str, clicked: usize) -> Option<String> {
    for (start, _) in line.match_indices("http") {
        let rest = &line[start..];
        if !(rest.starts_with("https://") || rest.starts_with("http://")) {
            continue;
        }
        let end = rest
            .find(|c: char| {
                c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`' | '[' | ']')
            })
            .unwrap_or(rest.len());
        let mut candidate = rest[..end].trim_end_matches(['.', ',', ';', '!']);
        while candidate.ends_with(')')
            && candidate.matches(')').count() > candidate.matches('(').count()
        {
            candidate = &candidate[..candidate.len() - 1];
        }
        if clicked >= start && clicked < start + candidate.len() {
            return web_url(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_query_and_hyphenated_code() {
        let s = "Opened https://accounts.feishu.cn/oauth/v1/device/verify?flow_id=abc&user_code=PAFP-N8YK in your browser.";
        assert_eq!(
            text_at(s, 30).as_deref(),
            Some(
                "https://accounts.feishu.cn/oauth/v1/device/verify?flow_id=abc&user_code=PAFP-N8YK"
            )
        );
        assert!(text_at(s, 0).is_none());
    }
    #[test]
    fn osc8_label_resolves_actual_target_and_blocks_other_schemes() {
        let mut term = Terminal::new(40, 4).unwrap();
        term.vt_write(b"\x1b]8;;https://example.com/?a=1&b=2\x1b\\label\x1b]8;;\x1b\\");
        assert_eq!(
            at(&term, 2, 0).as_deref(),
            Some("https://example.com/?a=1&b=2")
        );
        term.vt_write(b"\r\x1b]8;;file:///tmp/example\x1b\\label\x1b]8;;\x1b\\");
        assert!(at(&term, 2, 0).is_none());
    }
    #[test]
    fn plain_link_across_terminal_wrap() {
        let mut term = Terminal::new(20, 6).unwrap();
        term.vt_write(b"URL https://example.com/verify?flow_id=abc&user_code=PAFP-N8YK end");
        assert_eq!(
            at(&term, 8, 1).as_deref(),
            Some("https://example.com/verify?flow_id=abc&user_code=PAFP-N8YK")
        );
    }
}
