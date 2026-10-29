//! kitty Unicode placeholders (cells §7): U+10EEEE cells with the image id
//! in the foreground colour, the placement id in the underline colour and
//! diacritics for the row, the column and the id's high byte; inferred
//! step by step from the cell to the left; kept through history and
//! reflow.

use term_core::image::placeholder::{PLACEHOLDER, diacritic};
use term_core::image::{Image, ImageKey, Placement, placeholder_image_id};
use term_core::{Content, ImagePool, RowFlags, Snapshot, Term};

fn term(cols: usize, lines: usize) -> Term {
    Term::new(cols, lines, 1000, ImagePool::new(32 << 20))
}

/// U+10EEEE with diacritics for these values.
fn ph(values: &[u16]) -> String {
    let mut s = String::from(PLACEHOLDER);
    s.extend(values.iter().map(|&v| diacritic(v).unwrap()));
    s
}

/// Image id `id` (low 24 bits) as the foreground, placement `p` as the
/// underline colour.
fn pen(id: u32, p: u32) -> String {
    format!(
        "\x1b[38;2;{};{};{}m\x1b[58;2;{};{};{}m",
        id >> 16 & 255,
        id >> 8 & 255,
        id & 255,
        p >> 16 & 255,
        p >> 8 & 255,
        p & 255
    )
}

/// The pieces in view row `line`, None for cells that aren't placeholders.
fn pieces(t: &Term, line: usize) -> Vec<Option<(u16, u16)>> {
    let row = t.state.visible_row(line);
    row.cells()
        .map(|c| match c.content() {
            Content::Placeholder { row, col } => Some((row, col)),
            _ => None,
        })
        .collect()
}

fn id_at(t: &Term, line: usize, col: usize) -> u32 {
    placeholder_image_id(&t.state.visible_row(line).cell(col).style())
}

#[test]
fn missing_diacritics_are_inferred_from_the_left() {
    let mut t = term(6, 3);
    let s = format!("{}{}{}{}\r\n{}{}", pen(42, 1), ph(&[0, 0]), ph(&[]), ph(&[]), ph(&[1]), ph(&[]));
    t.advance(s.as_bytes());
    assert_eq!(pieces(&t, 0)[..4], [Some((0, 0)), Some((0, 1)), Some((0, 2)), None]);
    assert_eq!(pieces(&t, 1)[..3], [Some((1, 0)), Some((1, 1)), None]);
    assert_eq!(id_at(&t, 0, 2), 42);
    assert_eq!(
        t.state.visible_row(0).cell(1).style().underline_color,
        term_core::Color::rgb(0, 0, 1)
    );
    // A placeholder cell copies as a space, trimmed at the line end.
    assert_eq!(t.state.visible_row(0).text().trim_end(), "");
}

#[test]
fn a_row_diacritic_keeps_the_column_only_on_the_same_row() {
    let mut t = term(8, 2);
    let s = format!("{}{}{}{}", pen(5, 0), ph(&[2, 4]), ph(&[2]), ph(&[3]));
    t.advance(s.as_bytes());
    assert_eq!(pieces(&t, 0)[..3], [Some((2, 4)), Some((2, 5)), Some((3, 0))]);
}

#[test]
fn the_third_diacritic_is_the_high_byte_and_is_inherited() {
    let mut t = term(8, 2);
    let s = format!("{}{}{}{}{}", pen(0x123456, 0), ph(&[0, 0, 7]), ph(&[]), ph(&[0]), ph(&[0, 3]));
    t.advance(s.as_bytes());
    assert_eq!(pieces(&t, 0)[..4], [Some((0, 0)), Some((0, 1)), Some((0, 2)), Some((0, 3))]);
    assert_eq!(id_at(&t, 0, 0), 0x0712_3456);
    assert_eq!(id_at(&t, 0, 1), 0x0712_3456, "base only: the left cell's high byte");
    assert_eq!(id_at(&t, 0, 2), 0x0712_3456, "same row: kept");
    assert_eq!(id_at(&t, 0, 3), 0x0712_3456, "the column right after: kept");
    let s = format!("{}{}", ph(&[0, 9]), ph(&[1]));
    t.advance(s.as_bytes());
    assert_eq!(id_at(&t, 0, 4), 0x12_3456, "a column jump drops it");
}

#[test]
fn another_image_or_placement_on_the_left_starts_again() {
    let mut t = term(8, 2);
    let s = format!("{}{}{}{}{}{}", pen(1, 0), ph(&[4, 4]), pen(2, 0), ph(&[]), pen(2, 9), ph(&[]));
    t.advance(s.as_bytes());
    assert_eq!(pieces(&t, 0)[..3], [Some((4, 4)), Some((0, 0)), Some((0, 0))]);
}

#[test]
fn other_combining_marks_are_dropped_and_extra_diacritics_ignored() {
    let mut t = term(8, 2);
    let s = format!("{}{}\u{301}{}", pen(1, 0), ph(&[1, 2, 3, 4, 5]), ph(&[]));
    t.advance(s.as_bytes());
    assert_eq!(pieces(&t, 0)[..2], [Some((1, 2)), Some((1, 3))]);
    assert_eq!(id_at(&t, 0, 0), 3 << 24 | 1);
}

#[test]
fn diacritics_after_a_cursor_move_go_nowhere() {
    let mut t = term(8, 2);
    let s = format!("{}{}\x1b[C{}", pen(1, 0), ph(&[]), diacritic(5).unwrap());
    t.advance(s.as_bytes());
    assert_eq!(pieces(&t, 0)[..2], [Some((0, 0)), None]);
}

#[test]
fn inference_works_with_mode_2027_too() {
    let mut t = term(8, 2);
    let s = format!("\x1b[?2027h{}{}{}", pen(1, 0), ph(&[1, 1]), ph(&[]));
    t.advance(s.as_bytes());
    assert_eq!(pieces(&t, 0)[..2], [Some((1, 1)), Some((1, 2))]);
}

#[test]
fn a_column_guessed_past_511_is_a_blank() {
    let mut t = term(220, 2);
    let mut s = format!("{}{}", pen(1, 0), ph(&[0, 296]));
    for _ in 0..216 {
        s.push(PLACEHOLDER);
    }
    t.advance(s.as_bytes());
    let p = pieces(&t, 0);
    assert_eq!(p[0], Some((0, 296)));
    assert_eq!(p[215], Some((0, 511)));
    assert_eq!(p[216], None);
    assert!(matches!(t.state.visible_row(0).cell(216).content(), Content::Empty));
}

#[test]
fn overwriting_a_placeholder_removes_that_piece() {
    let mut t = term(8, 2);
    let s = format!("{}{}{}{}\rx", pen(1, 0), ph(&[0, 0]), ph(&[]), ph(&[]));
    t.advance(s.as_bytes());
    assert_eq!(pieces(&t, 0)[..3], [None, Some((0, 1)), Some((0, 2))]);
    t.advance(b"\x1b[K");
    assert_eq!(pieces(&t, 0)[..3], [None, None, None]);
}

#[test]
fn with_images_off_the_character_is_text() {
    let mut t = Term::new(8, 2, 0, ImagePool::process());
    if t.state.images_enabled() {
        return; // TD_IMAGES=1 in the environment
    }
    t.advance(ph(&[0, 0]).as_bytes());
    assert!(matches!(t.state.visible_row(0).cell(0).content(), Content::Cluster(_)));
}

#[test]
fn placeholders_survive_history_and_pull_back() {
    let mut t = term(6, 2);
    let s = format!("{}{}{}{}\x1b[m\r\nab\r\ncd", pen(0x10203, 7), ph(&[3, 0, 9]), ph(&[]), ph(&[]));
    t.advance(s.as_bytes());
    assert_eq!(t.state.scrollback_len(), 1);
    t.state.scroll_view(1);
    // History rows end at their last cell.
    assert_eq!(pieces(&t, 0), [Some((3, 0)), Some((3, 1)), Some((3, 2))]);
    assert_eq!(id_at(&t, 0, 2), 0x0901_0203);
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    assert!(snap.rows[0].flags.contains(RowFlags::PLACEHOLDERS) && snap.rows[0].flags.contains(RowFlags::HISTORY));
    assert!(!snap.rows[1].flags.contains(RowFlags::PLACEHOLDERS));
    t.state.scroll_view_to_bottom();
    // Taller: the line comes back onto the screen, pieces and all.
    t.state.resize(6, 3);
    assert_eq!(t.state.scrollback_len(), 0);
    assert_eq!(pieces(&t, 0)[..4], [Some((3, 0)), Some((3, 1)), Some((3, 2)), None]);
    assert_eq!(id_at(&t, 0, 1), 0x0901_0203);
    t.state.snapshot(&mut snap, |_| false);
    assert!(snap.rows[0].flags.contains(RowFlags::PLACEHOLDERS));
    assert!(!snap.rows[0].flags.contains(RowFlags::HISTORY));
}

#[test]
fn reflow_wraps_image_rows_in_pieces_losslessly() {
    let mut t = term(8, 4);
    let mut s = pen(77, 0);
    s += &ph(&[2, 0]);
    for _ in 0..5 {
        s.push(PLACEHOLDER);
    }
    s += "\x1b[m\r\nnext";
    t.advance(s.as_bytes());
    let want: Vec<_> = (0..6).map(|c| Some((2, c))).collect();
    assert_eq!(pieces(&t, 0)[..6], want[..]);
    t.state.resize(4, 4);
    assert_eq!(pieces(&t, 0), want[..4]);
    assert_eq!(pieces(&t, 1)[..2], want[4..]);
    assert_eq!(id_at(&t, 1, 1), 77);
    t.state.resize(8, 4);
    assert_eq!(pieces(&t, 0)[..6], want[..]);
}

fn placement(image: ImageKey, line: u64, col: u16) -> Placement {
    Placement {
        image,
        id: 1,
        line,
        col,
        cols: 2,
        rows: 1,
        offset: (0, 0),
        src: [0, 0, 1, 1],
        z: 0,
    }
}

#[test]
fn reflow_moves_placements_with_their_cell_and_drops_those_in_dropped_rows() {
    let mut t = term(10, 6);
    // A logical line of 20 cells over rows 0–1, the cursor on row 2.
    t.advance(b"0123456789abcdefghij\r\n");
    let k = ImageKey::Kitty(1);
    let g = t.state.graphics_mut_for_test();
    g.insert(Image::new(k, 1, 1));
    // On 'f' (row 1, column 5), and on blank row 4 below the cursor.
    g.place(placement(k, 1, 5));
    let below = ImageKey::Kitty(2);
    g.insert(Image::new(below, 1, 1));
    g.place(placement(below, 4, 0));
    // Five columns: 'f' is row 3, column 0; rows go into history and the
    // blank rows below the cursor are dropped.
    t.state.resize(5, 3);
    let g = t.state.graphics();
    let lines: Vec<(u64, u16)> = g.placements().map(|(_, p)| (p.line, p.col)).collect();
    assert_eq!(lines, vec![(3, 0)]);
    assert!(g.image(below).is_some(), "the image stays; only its placement went");
    t.state.check_invariants().unwrap();
    t.state.resize(10, 6);
    let lines: Vec<(u64, u16)> = t.state.graphics().placements().map(|(_, p)| (p.line, p.col)).collect();
    assert_eq!(lines, vec![(1, 5)]);
}
