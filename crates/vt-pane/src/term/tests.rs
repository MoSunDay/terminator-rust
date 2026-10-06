use super::*;

fn frame(bytes: &[u8]) -> Frame {
    let mut term = Terminal::new(8, 3).expect("terminal");
    term.vt_write(bytes);
    snapshot_frame(
        &mut term,
        &mut RenderState::new().expect("render"),
        &mut RowIterator::new().expect("rows"),
        &mut CellIterator::new().expect("cells"),
    )
    .expect("snapshot")
}

#[test]
fn snapshot_distinguishes_pending_wrap_from_a_cursor_in_the_last_column() {
    for text in ["abcdefgh", "汉字汉字"] {
        let before = frame(text.as_bytes());
        assert!(before.cursor.pending_wrap);
        assert!(before.cursor.autowrap);
        let after = frame(format!("{text}你").as_bytes());
        assert_eq!((after.cursor.x, after.cursor.y), (2, 1));
        assert!(!after.cursor.pending_wrap);
    }
    let positioned = frame(b"\x1b[1;8H");
    assert_eq!(positioned.cursor.x, 7);
    assert!(!positioned.cursor.pending_wrap);
    let no_wrap = frame(b"\x1b[?7labcdefgh");
    assert!(!no_wrap.cursor.autowrap);
    let overwritten = frame(b"\x1b[?7labcdefghi");
    assert_eq!((overwritten.cursor.x, overwritten.cursor.y), (7, 0));
    assert_eq!(overwritten.cells[0][7].text, "i");
}
