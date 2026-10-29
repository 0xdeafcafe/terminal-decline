//! Unit tests for scrollback v2: the worked examples of §11.4 byte for
//! byte, decode of encode for random rows, pop, the limits and sealing.

use super::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn template(styles: &[Style], id: usize) -> Cell {
    Cell::template(id as u32, styles[id].shows_empty())
}

/// Write `text` from column `at` in style `id`, wide characters with a
/// spacer. Returns the column after it.
fn put(row: &mut Row, styles: &[Style], at: usize, id: usize, text: &str) -> usize {
    let t = template(styles, id);
    let mut col = at;
    for ch in text.chars() {
        if single_width(ch, 1) == 2 {
            row.cells[col] = t.with_char(ch).with_width(Width::Wide);
            row.cells[col + 1] = t.with_width(Width::Spacer);
            col += 2;
        } else {
            row.cells[col] = t.with_char(ch);
            col += 1;
        }
    }
    col
}

/// A cluster cell (wide if `wide`) at `col`.
fn put_cluster(row: &mut Row, styles: &[Style], col: usize, id: usize, text: &str, wide: bool) -> usize {
    let t = template(styles, id).with_char(text.chars().next().unwrap());
    if wide {
        row.cells[col] = t.with_width(Width::Wide);
        row.cells[col + 1] = template(styles, id).with_width(Width::Spacer);
    } else {
        row.cells[col] = t;
    }
    assert!(row.set_cluster(col, text));
    col + 1 + wide as usize
}

/// The open block's meta and text for its only line.
fn stored(sb: &Scrollback) -> (Vec<u8>, Vec<u8>) {
    (sb.open.meta.clone(), sb.open.text.clone())
}

/// Sealed bytes for one line as §11.4 counts them: meta + text + its
/// 2-byte offset.
fn line_bytes(meta: &[u8], text: &[u8]) -> usize {
    meta.len() + text.len() + 2
}

fn ls_styles() -> Vec<Style> {
    vec![
        Style::DEFAULT,
        Style {
            fg: Color::indexed(4),
            attrs: Attrs::BOLD,
            ..Style::DEFAULT
        },
        Style {
            fg: Color::indexed(2),
            ..Style::DEFAULT
        },
        Style {
            fg: Color::indexed(1),
            ..Style::DEFAULT
        },
    ]
}

#[test]
fn plain_line() {
    let mut sb = Scrollback::new(10);
    let mut row = Row::new(160, Cell::BLANK);
    put(&mut row, &[Style::DEFAULT], 0, 0, "00012345 the quick brown fox jumps over");
    sb.push(&row, &[Style::DEFAULT]);
    let (meta, text) = stored(&sb);
    assert_eq!(meta, [0]);
    assert_eq!(text, b"00012345 the quick brown fox jumps over\n");
    assert_eq!(line_bytes(&meta, &text), 43);
}

#[test]
fn ls_line() {
    let styles = ls_styles();
    let mut row = Row::new(160, Cell::BLANK);
    let mut c = put(&mut row, &styles, 0, 1, "dir12345");
    c = put(&mut row, &styles, c, 0, "  ");
    c = put(&mut row, &styles, c, 2, "build.sh");
    c = put(&mut row, &styles, c, 0, "  notes.txt  ");
    c = put(&mut row, &styles, c, 1, "src");
    c = put(&mut row, &styles, c, 0, "  ");
    put(&mut row, &styles, c, 3, "archive.tar");
    let mut sb = Scrollback::new(10);
    sb.push(&row, &styles);
    let (meta, text) = stored(&sb);
    assert_eq!(meta, [0x04, 7, 0x41, 0x10, 0x42, 0x68, 0x19, 0x10, 0x5B]);
    assert_eq!(text.len(), 48);
    assert_eq!(line_bytes(&meta, &text), 59);
    // The dictionary, once per block: bold blue, green, red.
    assert_eq!(sb.open.styles, [0x41, 4, 1, 0x01, 2, 0x01, 1]);
    assert_eq!(sb.open.style_starts, [0, 3, 5]);
}

#[test]
fn full_width_background_line() {
    let styles = vec![
        Style::DEFAULT,
        Style {
            bg: Color::indexed(22),
            ..Style::DEFAULT
        },
    ];
    // EL to the end in a background colour: every column shows.
    let mut row = Row::new(160, template(&styles, 1));
    put(&mut row, &styles, 0, 1, "+12345");
    let mut sb = Scrollback::new(10);
    sb.push(&row, &styles);
    let (meta, text) = stored(&sb);
    assert_eq!(meta, [0x24, 1, 0x81, 0x0A]); // 160 << 3 | 1
    assert_eq!(text, b"+12345\n");
    assert_eq!(line_bytes(&meta, &text), 13);
    assert_eq!(sb.open.styles, [0x04, 22]);
    // At its own width it reads back whole; wider, the background goes on.
    let (back, back_styles) = sb.get(0).unwrap();
    assert_eq!(back.cells.len(), 160);
    assert!(back.cells.iter().all(|c| back_styles[c.style_id() as usize] == styles[1]));
    let (mut wide, mut wide_styles) = (Row::default(), Vec::new());
    assert!(sb.get_at_width(0, 200, &mut wide, &mut wide_styles, &mut Vec::new()));
    assert_eq!(wide.cells.len(), 200);
    assert_eq!(wide_styles[wide.cells[199].style_id() as usize], styles[1]);
    assert!(sb.fill(0));
}

#[test]
fn compiler_error_line() {
    let styles = vec![
        Style::DEFAULT,
        Style {
            fg: Color::indexed(1),
            attrs: Attrs::BOLD,
            ..Style::DEFAULT
        },
        Style {
            attrs: Attrs::BOLD,
            ..Style::DEFAULT
        },
    ];
    let mut row = Row::new(160, Cell::BLANK);
    let mut c = put(&mut row, &styles, 0, 1, "error[E12345]");
    c = put(&mut row, &styles, c, 2, ": mismatched types");
    put(&mut row, &styles, c, 0, " --> src/main.rs:12345:5");
    let mut sb = Scrollback::new(10);
    sb.push(&row, &styles);
    let (meta, text) = stored(&sb);
    assert_eq!(meta, [0x04, 2, 0x69, 0x92, 0x01]);
    assert_eq!(text.len(), 56);
    assert_eq!(line_bytes(&meta, &text), 63);
}

#[test]
fn cjk_line_needs_no_runs() {
    let mut row = Row::new(160, Cell::BLANK);
    put(&mut row, &[Style::DEFAULT], 0, 0, "12345 漢字かなカナ한글 混合テキスト 中文字符");
    let mut sb = Scrollback::new(10);
    sb.push(&row, &[Style::DEFAULT]);
    let (meta, text) = stored(&sb);
    assert_eq!(meta, [0]);
    assert_eq!(text.len(), 63);
    assert_eq!(line_bytes(&meta, &text), 66);
    let (back, back_styles) = sb.get(0).unwrap();
    same((&back, &back_styles), (&row, &[Style::DEFAULT]), "CJK");
    assert_eq!(back.cells.len(), 44);
}

#[test]
fn emoji_line_lists_its_clusters() {
    let d = [Style::DEFAULT];
    let mut row = Row::new(160, Cell::BLANK);
    let mut c = put(&mut row, &d, 0, 0, "12345 \u{2705} done ");
    c = put_cluster(&mut row, &d, c, 0, "👍🏽", true);
    c = put(&mut row, &d, c, 0, " ship ");
    c = put_cluster(&mut row, &d, c, 0, "🇬🇧", true);
    c = put(&mut row, &d, c, 0, " ");
    c = put_cluster(&mut row, &d, c, 0, "❤\u{fe0f}", true);
    c = put(&mut row, &d, c, 0, " ");
    c = put_cluster(&mut row, &d, c, 0, "👨\u{200d}👩\u{200d}👧", true);
    put(&mut row, &d, c, 0, " ok");
    let mut sb = Scrollback::new(10);
    sb.push(&row, &d);
    let (meta, text) = stored(&sb);
    // Each cluster's extra scalars, shifted, with its width in bit 0.
    assert_eq!(meta, [0x08, 4, 0x0D, 3, 6, 3, 1, 3, 1, 9]);
    assert_eq!(text.len(), 67);
    assert_eq!(line_bytes(&meta, &text), 79);
    let (back, _) = sb.get(0).unwrap();
    let mut s = String::new();
    for col in 0..back.cells.len() {
        if !back.cells[col].is_spacer() {
            // Typed spaces read back as empty cells.
            let n = s.len();
            back.text_at(col, &mut s);
            if s.len() == n {
                s.push(' ');
            }
        }
    }
    assert_eq!(s, "12345 \u{2705} done 👍🏽 ship 🇬🇧 ❤\u{fe0f} 👨\u{200d}👩\u{200d}👧 ok");
    same((&back, &d), (&row, &d), "emoji");
}

#[test]
fn wrapped_line_keeps_its_cells_and_length() {
    let mut row = Row::new(160, Cell::BLANK);
    put(&mut row, &[Style::DEFAULT], 0, 0, &"x".repeat(160));
    row.wrapped = true;
    let mut sb = Scrollback::new(10);
    sb.push(&row, &[Style::DEFAULT]);
    let (meta, text) = stored(&sb);
    assert_eq!(meta, [0x01, 0xA0, 0x01]);
    assert_eq!(text.len(), 160);
    assert_eq!(line_bytes(&meta, &text), 165);
    // Trailing blanks of a wrapped line are kept as spaces, so the joined
    // logical line is exact.
    let mut row = Row::new(10, Cell::BLANK);
    put(&mut row, &[Style::DEFAULT], 0, 0, "ab");
    row.wrapped = true;
    sb.push(&row, &[Style::DEFAULT]);
    assert_eq!(sb.text(1).unwrap(), b"ab        ");
    assert!(sb.wrapped(1));
}

#[test]
fn edge_spacer_and_width_exceptions() {
    let d = [Style::DEFAULT];
    let mut row = Row::new(6, Cell::BLANK);
    // `±` is ambiguous: written two wide with the setting at 2, but the
    // block is at 1; a ZWJ family written one wide (mode 2027 off).
    let c = put(&mut row, &d, 0, 0, "a");
    row.cells[c] = Cell::BLANK.with_char('±').with_width(Width::Wide);
    row.cells[c + 1] = Cell::BLANK.with_width(Width::Spacer);
    put_cluster(&mut row, &d, c + 2, 0, "👨\u{200d}👩", false);
    row.cells[4] = Cell::BLANK.with_char('b');
    row.cells[5] = Cell::BLANK.with_width(Width::EdgeSpacer);
    row.wrapped = true;
    let mut sb = Scrollback::new(10);
    sb.push(&row, &d);
    let (meta, _) = stored(&sb);
    // WRAPPED | EDGE | CLUSTERS | WIDTHS, tlen, one cluster, two exceptions.
    assert_eq!(meta[0], 0x01 | 0x02 | 0x08 | 0x10);
    let (back, back_styles) = sb.get(0).unwrap();
    assert_eq!(back.cells.len(), 6);
    same((&back, &back_styles), (&row, &d), "edge");
    assert_eq!(back.cells[5].width(), Width::EdgeSpacer);
}

#[test]
fn placeholders_are_left_for_a_later_flag() {
    // Reserved now so the format doesn't change when they arrive.
    assert_eq!(flag::EXTRA, 0x80);
}

#[test]
fn prompt_line_with_marks() {
    let d = [Style::DEFAULT];
    let mut row = Row::new(80, Cell::BLANK);
    put(&mut row, &d, 0, 0, "$ ls");
    row.set_mark(Mark {
        kind: MarkKind::Prompt,
        col: 0,
        status: None,
    });
    row.set_mark(Mark {
        kind: MarkKind::Input,
        col: 2,
        status: None,
    });
    row.set_mark(Mark {
        kind: MarkKind::End,
        col: 0,
        status: Some(300),
    });
    let mut sb = Scrollback::new(10);
    sb.push(&row, &d);
    sb.push(&Row::new(80, Cell::BLANK.with_char('x')), &d);
    let (meta, _) = stored(&sb);
    // Plain text with marks: flags, then count, and per mark kind (bit 7:
    // a status follows), column, status.
    assert_eq!(meta[..10], [0x40, 3, 0x01, 0, 0x02, 2, 0x84, 0, 0xAC, 0x02]);
    let (back, _) = sb.get(0).unwrap();
    assert_eq!(back.marks().collect::<Vec<_>>(), row.marks().collect::<Vec<_>>());
    assert_eq!(sb.marks(0), row.marks().collect::<Vec<_>>());
    assert!(sb.has_marks(0) && !sb.has_marks(1));
    sb.check_all().unwrap();
}

#[test]
fn marked_lines_skip_blocks_without_marks() {
    let d = [Style::DEFAULT];
    let mut sb = Scrollback::new(100_000);
    let plain = Row::new(100, Cell::BLANK.with_char('p'));
    let mut marked = plain.clone();
    marked.set_mark(Mark {
        kind: MarkKind::Prompt,
        col: 0,
        status: None,
    });
    let at = [5usize, 2000, 2001, 9000];
    for i in 0..10_000 {
        sb.push(if at.contains(&i) { &marked } else { &plain }, &d);
    }
    let blocks: Vec<bool> = sb.sealed().map(|(b, _)| b.has_marks()).collect();
    assert!(blocks.iter().filter(|&&m| !m).count() >= 5, "{blocks:?}");
    let mut down = Vec::new();
    let mut i = 0;
    while let Some(j) = sb.marked(i, false) {
        down.push(j);
        i = j + 1;
    }
    assert_eq!(down, at);
    let mut up = Vec::new();
    let mut i = sb.len();
    while let Some(j) = sb.marked(i, true) {
        up.push(j);
        i = j;
    }
    up.reverse();
    assert_eq!(up, at);
}

#[test]
fn links_go_into_the_blocks_own_table() {
    let a = Arc::new(LinkValue {
        uri: "https://a".into(),
        id: None,
    });
    let b = Arc::new(LinkValue {
        uri: "https://b".into(),
        id: Some("x".into()),
    });
    // The screen's link table, and styles naming it (ids 2 and 5).
    let mut links: Vec<Option<Arc<LinkValue>>> = vec![None; 6];
    links[2] = Some(a.clone());
    links[5] = Some(b.clone());
    let styles = vec![
        Style::DEFAULT,
        Style { link: 2, ..Style::DEFAULT },
        Style {
            link: 5,
            fg: Color::indexed(3),
            ..Style::DEFAULT
        },
        Style {
            link: 2,
            attrs: Attrs::BOLD,
            ..Style::DEFAULT
        },
    ];
    let mut row = Row::new(20, Cell::BLANK);
    let mut c = put(&mut row, &styles, 0, 1, "aa");
    c = put(&mut row, &styles, c, 2, "bb");
    c = put(&mut row, &styles, c, 3, "AA");
    put(&mut row, &styles, c, 0, "plain");
    let mut sb = Scrollback::new(10);
    sb.push_keyed(&row, &styles, &links, [1, 0]);
    assert_eq!(sb.open.links, [a.clone(), b.clone()], "each link once, by value");
    let (mut back, mut back_styles, mut back_links) = (Row::default(), Vec::new(), Vec::new());
    assert!(sb.get_at_width(0, 0, &mut back, &mut back_styles, &mut back_links));
    let link = |col: usize| back_links[back_styles[back.cells[col].style_id() as usize].link as usize].clone();
    assert_eq!([link(0), link(2), link(4), link(6)], [Some(a.clone()), Some(b), Some(a), None]);
    assert_eq!(back_styles[back.cells[4].style_id() as usize].attrs, Attrs::BOLD);
    sb.check_all().unwrap();
}

#[test]
fn dictionary_entries() {
    let cases = [
        (
            Style {
                fg: Color::indexed(4),
                attrs: Attrs::BOLD,
                ..Style::DEFAULT
            },
            vec![0x41, 4, 1],
        ),
        (
            Style {
                fg: Color::indexed(2),
                ..Style::DEFAULT
            },
            vec![0x01, 2],
        ),
        (
            Style {
                bg: Color::indexed(22),
                ..Style::DEFAULT
            },
            vec![0x04, 22],
        ),
        (
            Style {
                fg: Color::rgb(1, 2, 3),
                ..Style::DEFAULT
            },
            vec![0x02, 1, 2, 3],
        ),
        (
            Style {
                fg: Color::rgb(1, 2, 3),
                bg: Color::indexed(9),
                underline_color: Color::rgb(4, 5, 6),
                attrs: Attrs::ITALIC | Attrs::BLINK,
                underline: Underline::Curly,
                image_id_high: 7,
                ..Style::DEFAULT
            },
            vec![0x02 | 0x04 | 0x20 | 0x40 | 0x80, 1, 2, 3, 9, 4, 5, 6, 0x84, 0x04, 0x05, 3, 7],
        ),
    ];
    for (style, bytes) in cases {
        let mut out = Vec::new();
        put_style(&mut out, &style);
        assert_eq!(out, bytes, "{style:?}");
        assert_eq!(get_style(&out), style);
    }
}

#[test]
fn varint_edge_values() {
    let cases: &[(u32, usize)] = &[
        (0, 1),
        (1, 1),
        (127, 1),
        (128, 2),
        (16_383, 2),
        (16_384, 3),
        ((1 << 21) - 1, 3),
        (1 << 21, 4),
        ((1 << 28) - 1, 4),
        (1 << 28, 5),
        (u32::MAX, 5),
    ];
    for &(v, len) in cases {
        let mut out = vec![0xAA];
        put_varint(&mut out, v);
        out.push(0x55);
        assert_eq!(out.len(), len + 2, "{v} takes {len} bytes");
        let mut pos = 1;
        assert_eq!(get_varint(&out, &mut pos), v);
        assert_eq!(pos, len + 1, "{v} read to its end");
    }
    let mut out = Vec::new();
    for &(v, _) in cases {
        put_varint(&mut out, v);
    }
    let mut pos = 0;
    for &(v, _) in cases {
        assert_eq!(get_varint(&out, &mut pos), v);
    }
    assert_eq!(pos, out.len());
}

#[test]
fn varint_truncated_input_stops_at_the_end() {
    let mut pos = 0;
    assert_eq!(get_varint(&[0x80, 0x81], &mut pos), 0x80);
    assert_eq!(pos, 2);
    let mut pos = 5;
    assert_eq!(get_varint(&[1], &mut pos), 0);
    // Too long: read to its end without overflowing.
    let mut pos = 0;
    get_varint(&[0xFF; 9], &mut pos);
    assert_eq!(pos, 9);
}

#[test]
fn find_end_finds_the_first_end_byte() {
    for len in 0..40 {
        for at in 0..=len {
            let mut text = vec![b'x'; len];
            if at < len {
                text[at] = END;
                // A later one, and bytes just around 0x0A, don't confuse it.
                if at + 3 < len {
                    text[at + 3] = END;
                }
                if at > 0 {
                    text[at - 1] = 0x0B;
                }
            }
            let want = (at < len).then_some(at);
            assert_eq!(find_end(&text), want, "{len} {at}");
        }
    }
    assert_eq!(find_end(&[0x8A, 0x0B, 0x09, 0x1A, 0x0A]), Some(4));
}

/// What a row reads back as, cell by cell: content (clusters whole,
/// spaces as empty), width, style by value; trailing blank cells dropped
/// (never half of a wide character).
fn normalise(row: &Row, styles: &[Style]) -> (Vec<(String, Width, Style)>, bool) {
    normalise_with(row, styles)
}

#[track_caller]
fn same(a: (&Row, &[Style]), b: (&Row, &[Style]), what: &str) {
    let (x, y) = (normalise(a.0, a.1), normalise(b.0, b.1));
    if x != y {
        let i = x.0.iter().zip(&y.0).position(|(p, q)| p != q).unwrap_or(x.0.len().min(y.0.len()));
        panic!(
            "{what}: differ at cell {i} (lens {} {}, wrapped {} {}): {:?} vs {:?}\nrow {:?}",
            x.0.len(),
            y.0.len(),
            x.1,
            y.1,
            x.0.get(i),
            y.0.get(i),
            b.0
        );
    }
}

fn normalise_with(row: &Row, styles: &[Style]) -> (Vec<(String, Width, Style)>, bool) {
    let blank = |c: &Cell| {
        let empty = !c.is_cluster() && matches!(c.codepoint(), 0 | 0x20);
        empty && !c.shows_empty()
    };
    let mut len = row.cells.iter().rposition(|c| !blank(c)).map_or(0, |i| i + 1);
    if len > 0 && len < row.cells.len() && row.cells[len - 1].is_wide() {
        len += 1;
    }
    let cells = (0..len)
        .map(|i| {
            let c = row.cells[i];
            let mut s = String::new();
            row.text_at(i, &mut s);
            if s == " " {
                s.clear();
            }
            (s, c.width(), styles[c.style_id() as usize])
        })
        .collect();
    (cells, row.wrapped)
}

fn colour(r: &mut Rng) -> Color {
    match r.below(3) {
        0 => Color::DEFAULT,
        1 => Color::indexed(r.below(256) as u8),
        _ => Color::rgb(r.below(256) as u8, r.below(256) as u8, r.below(256) as u8),
    }
}

fn random_styles(r: &mut Rng, n: usize) -> Vec<Style> {
    let mut styles = vec![Style::DEFAULT];
    for _ in 0..n {
        let underline = [
            Underline::None,
            Underline::None,
            Underline::Single,
            Underline::Double,
            Underline::Curly,
            Underline::Dotted,
            Underline::Dashed,
        ][r.below(7) as usize];
        styles.push(Style {
            fg: colour(r),
            bg: if r.below(2) == 0 { Color::DEFAULT } else { colour(r) },
            underline_color: if r.below(3) == 0 { colour(r) } else { Color::DEFAULT },
            attrs: Attrs::from_bits(r.next() as u16 & 0x2FF),
            underline,
            // Links aren't kept yet (PR 9); the image byte is.
            link: 0,
            image_id_high: if r.below(8) == 0 { r.below(256) as u8 } else { 0 },
        });
    }
    styles
}

const CLUSTERS: [&str; 6] = ["e\u{301}", "👍🏽", "🇬🇧", "❤\u{fe0f}", "👨\u{200d}👩\u{200d}👧", "1\u{fe0f}\u{20e3}"];

/// A random row of every kind of cell the encoder treats specially, at
/// any width written with either ambiguous setting or mode 2027 on or off.
fn random_row(r: &mut Rng, styles: &[Style]) -> Row {
    let cols = 1 + r.below(120) as usize;
    let mut row = Row::new(cols, Cell::BLANK);
    let mut col = 0;
    let fill = r.below(4) == 0;
    while col < cols {
        let id = r.below(styles.len() as u64) as usize;
        let t = template(styles, id);
        let room = cols - col;
        match r.below(14) {
            0..=3 => row.cells[col] = Cell::BLANK.with_char((0x20 + r.below(95) as u8) as char),
            4 => row.cells[col] = t.with_char((0x21 + r.below(94) as u8) as char),
            5 => row.cells[col] = t,
            6 => {}
            7 => row.cells[col] = t.with_char(['é', 'Ω', '─', '±', '\u{10FFFD}', '\u{7F}'][r.below(6) as usize]),
            8 if room >= 2 => {
                row.cells[col] = t.with_char(['漢', '±', '①', 'ä'][r.below(4) as usize]).with_width(Width::Wide);
                row.cells[col + 1] = t.with_width(Width::Spacer);
                col += 1;
            }
            9 | 10 => {
                let s = CLUSTERS[r.below(CLUSTERS.len() as u64) as usize];
                let wide = r.below(3) != 0 && room >= 2;
                put_cluster(&mut row, styles, col, id, s, wide);
                col += wide as usize;
            }
            11 if room >= 2 => {
                row.cells[col] = t.with_width(Width::Wide);
                row.cells[col + 1] = t.with_width(Width::Spacer);
                col += 1;
            }
            _ => row.cells[col] = Cell::BLANK.with_char(' '),
        }
        col += 1;
        if r.below(30) == 0 {
            // Trailing blanks of every kind.
            for c in &mut row.cells[col..] {
                *c = if r.below(2) == 0 { Cell::BLANK } else { t.style_part() };
            }
            break;
        }
    }
    row.wrapped = r.below(3) == 0;
    if row.wrapped && r.below(3) == 0 && cols >= 2 && !row.cells[cols - 2].is_wide() {
        row.free_clusters(cols - 1..cols);
        row.cells[cols - 1] = template(styles, r.below(styles.len() as u64) as usize).with_width(Width::EdgeSpacer);
    }
    if fill && !row.wrapped {
        let id = (1..styles.len()).find(|&i| styles[i].shows_empty());
        if let Some(id) = id
            && !row.cells[cols - 1].is_spacer()
        {
            row.free_clusters(cols - 1..cols);
            row.cells[cols - 1] = template(styles, id);
            if cols >= 2 && row.cells[cols - 2].is_wide() {
                row.free_clusters(cols - 2..cols - 1);
                row.cells[cols - 2] = template(styles, id);
            }
        }
    }
    row
}

#[test]
fn rows_read_back_as_written() {
    let mut r = Rng(0x5EED_1234_ABCD_0001);
    for round in 0..300 {
        let n = 1 + r.below(30) as usize;
        let styles = random_styles(&mut r, n);
        let mut sb = Scrollback::new(1000);
        sb.set_ambiguous(1 + r.below(2) as u8);
        let rows: Vec<Row> = (0..60).map(|_| random_row(&mut r, &styles)).collect();
        for (i, row) in rows.iter().enumerate() {
            // Sometimes keep the slot cache from line to line (one table).
            let key = if round % 2 == 0 { [1, 1] } else { [0, 0] };
            sb.push_keyed(row, &styles, &[], key);
            if i % 17 == 16 {
                sb.set_ambiguous(1 + r.below(2) as u8);
            }
        }
        sb.check_all().unwrap();
        for (i, row) in rows.iter().enumerate() {
            let (back, back_styles) = sb.get(i).unwrap();
            same((&back, &back_styles), (row, &styles), &format!("round {round} row {i}"));
            assert_eq!(sb.wrapped(i), row.wrapped);
        }
    }
}

#[test]
fn rows_round_trip_through_sealed_blocks() {
    let mut r = Rng(42);
    let styles = random_styles(&mut r, 40);
    let mut sb = Scrollback::new(100_000);
    let rows: Vec<Row> = (0..5000).map(|_| random_row(&mut r, &styles)).collect();
    for row in &rows {
        sb.push_keyed(row, &styles, &[], [3, 0]);
    }
    assert!(sb.stats().sealed_blocks >= 3, "{:?}", sb.stats());
    sb.check_all().unwrap();
    for (i, row) in rows.iter().enumerate() {
        let (back, back_styles) = sb.get(i).unwrap();
        same((&back, &back_styles), (row, &styles), &format!("row {i}"));
    }
}

#[test]
fn the_slot_cache_follows_the_table() {
    let red = Style {
        fg: Color::indexed(1),
        ..Style::DEFAULT
    };
    let green = Style {
        fg: Color::indexed(2),
        ..Style::DEFAULT
    };
    let mut row = Row::new(3, Cell::BLANK);
    row.cells[0] = Cell::template(1, false).with_char('x');
    let mut sb = Scrollback::new(10);
    sb.push_keyed(&row, &[Style::DEFAULT, red], &[], [5, 0]);
    // A sweep freed id 1 and a new style took it: a new key.
    sb.push_keyed(&row, &[Style::DEFAULT, green], &[], [5, 1]);
    // Another table (the primary screen's rows while the alternate one is
    // shown) with the same id for another style.
    sb.push_keyed(&row, &[Style::DEFAULT, red], &[], [6, 0]);
    let fg = |i| {
        let (row, styles) = sb.get(i).unwrap();
        styles[row.cells[0].style_id() as usize].fg
    };
    assert_eq!([fg(0), fg(1), fg(2)], [red.fg, green.fg, red.fg]);
}

#[test]
fn blank_rows_and_empty_rows() {
    let mut sb = Scrollback::new(10);
    sb.push(&Row::new(80, Cell::BLANK), &[Style::DEFAULT]);
    sb.push(&Row::default(), &[Style::DEFAULT]);
    assert_eq!(sb.len(), 2);
    assert_eq!(sb.get(0).unwrap().0.cells.len(), 0);
    assert_eq!(sb.get(1).unwrap().0.cells.len(), 0);
    assert_eq!(sb.text(0), Some(&b""[..]));
    sb.check_all().unwrap();
}

#[test]
fn rows_wider_than_a_block_allows_are_cut() {
    // Wide rows that fit are kept whole.
    let styles = random_styles(&mut Rng(9), 5);
    let mut row = Row::new(6000, Cell::BLANK);
    for (i, c) in row.cells.iter_mut().enumerate() {
        *c = template(&styles, i % 6).with_char('x');
    }
    let mut sb = Scrollback::new(10);
    sb.push(&row, &styles);
    assert_eq!(sb.get(0).unwrap().0.cells.len(), 6000);
    // A 20,000-cell gradient doesn't fit even in an empty block: it is cut
    // to 4096 cells.
    let styles = random_styles(&mut Rng(10), 20_000);
    let mut row = Row::new(20_000, Cell::BLANK);
    for (i, c) in row.cells.iter_mut().enumerate() {
        *c = template(&styles, 1 + i).with_char('漢');
    }
    sb.push(&row, &styles);
    sb.push(&row, &styles);
    sb.check_all().unwrap();
    assert_eq!(sb.get(2).unwrap().0.cells.len(), MAX_CELLS);
}

#[test]
fn pop_gives_rows_back_and_leaves_shared_blocks_alone() {
    let mut r = Rng(77);
    let styles = random_styles(&mut r, 10);
    let mut sb = Scrollback::new(10_000);
    let rows: Vec<Row> = (0..3000).map(|_| random_row(&mut r, &styles)).collect();
    for row in &rows {
        sb.push(row, &styles);
    }
    let blocks: Vec<(Arc<BlockData>, usize)> = sb.sealed().map(|(d, n)| (d.clone(), n)).collect();
    let snapshot: Vec<BlockData> = blocks.iter().map(|(d, _)| (**d).clone()).collect();
    let total = sb.total();
    assert_eq!(sb.take_popped_from(), None);
    // Pop past the open block into the sealed ones.
    let popped = sb.stats().open_lines + 700;
    for k in 0..popped {
        let (row, back_styles) = sb.pop().unwrap();
        let want = &rows[rows.len() - 1 - k];
        same((&row, &back_styles), (want, &styles), &format!("pop {k}"));
        sb.check().unwrap();
    }
    assert_eq!(sb.total(), total - popped as u64);
    assert_eq!(sb.take_popped_from(), Some(total - popped as u64));
    assert_eq!(sb.take_popped_from(), None, "taken");
    for ((d, _), before) in blocks.iter().zip(&snapshot) {
        assert_eq!(**d, *before, "shared data unchanged");
    }
    sb.check_all().unwrap();
    // Pushing again starts a new block and reads as it should.
    for row in &rows[..50] {
        sb.push(row, &styles);
    }
    sb.check_all().unwrap();
    for (i, row) in rows[..50].iter().enumerate() {
        let (back, back_styles) = sb.get(sb.len() - 50 + i).unwrap();
        assert_eq!(normalise(&back, &back_styles), normalise(row, &styles));
    }
    // Popping everything empties it; the mark is the lowest popped.
    sb.push(&rows[0], &styles);
    sb.pop().unwrap();
    sb.pop().unwrap();
    assert_eq!(sb.take_popped_from(), Some(sb.total()));
    while sb.pop().is_some() {}
    assert!(sb.is_empty());
    assert_eq!(sb.sealed().count(), 0);
    sb.check_all().unwrap();
    assert_eq!(sb.take_popped_from(), Some(0));
}

#[test]
fn pop_brings_back_lines_hidden_by_the_limit() {
    let mut sb = Scrollback::new(100);
    let row = Row::new(10, Cell::BLANK.with_char('x'));
    for _ in 0..150 {
        sb.push(&row, &[Style::DEFAULT]);
    }
    assert_eq!(sb.len(), 100);
    sb.pop().unwrap();
    // 149 lines stored (none freed: they're all in the open block), 100 shown.
    assert_eq!(sb.len(), 100);
    assert_eq!(sb.total(), 149);
    sb.check().unwrap();
}

#[test]
fn freeing_keeps_at_most_one_spare_block_and_clear_frees_all() {
    let mut sb = Scrollback::new(500);
    let row = Row::new(200, Cell::BLANK.with_char('x'));
    for _ in 0..20_000 {
        sb.push(&row, &[Style::DEFAULT]);
        // 500 lines of ~200 bytes span two blocks, plus the open one.
        assert!(sb.blocks.len() <= 3, "{} blocks", sb.blocks.len());
    }
    assert_eq!(sb.len(), 500);
    sb.check_all().unwrap();
    sb.clear();
    assert!(sb.is_empty());
    assert!(sb.memory_bytes() < 1024, "{}", sb.memory_bytes());
    assert!(sb.get(0).is_none());
    sb.push(&row, &[Style::DEFAULT]);
    assert_eq!(sb.len(), 1);
    assert_eq!(sb.get(0).unwrap().0.cells.len(), 200);
}

#[test]
fn limit_and_block_freeing() {
    let mut sb = Scrollback::new(1000);
    let mut row = Row::new(80, Cell::BLANK);
    for i in 0..100_000u32 {
        let s = i.to_string();
        for (j, c) in s.chars().enumerate() {
            row.cells[j] = Cell::BLANK.with_char(c);
        }
        sb.push(&row, &[Style::DEFAULT]);
    }
    assert_eq!(sb.len(), 1000);
    let first: String = sb.get(0).unwrap().0.cells.iter().map(|c| c.ch()).collect();
    assert_eq!(first, "99000");
    assert!(sb.blocks.len() <= 2, "old blocks freed");
}

#[test]
fn changing_the_limit() {
    let mut sb = Scrollback::new(10_000);
    let row = Row::new(100, Cell::BLANK.with_char('y'));
    for _ in 0..5000 {
        sb.push(&row, &[Style::DEFAULT]);
    }
    let bytes = sb.memory_bytes();
    sb.set_limit(100);
    assert_eq!(sb.len(), 100);
    assert!(sb.memory_bytes() < bytes / 4, "blocks freed at once");
    sb.check().unwrap();
    // Raising it brings back lines still stored, and keeps more after.
    let stored = (sb.total() - sb.oldest()) as usize;
    sb.set_limit(20_000);
    assert_eq!(sb.len(), stored);
    for _ in 0..1000 {
        sb.push(&row, &[Style::DEFAULT]);
    }
    assert_eq!(sb.len(), stored + 1000);
    sb.set_limit(0);
    assert!(sb.is_empty());
    sb.push(&row, &[Style::DEFAULT]);
    assert!(sb.is_empty());
    assert_eq!(sb.total(), 6001, "lines are still counted");
    sb.check().unwrap();
    sb.set_limit(5);
    sb.push(&row, &[Style::DEFAULT]);
    assert_eq!(sb.len(), 1);
}

#[test]
fn the_backstop_frees_whole_blocks() {
    let mut sb = Scrollback::new(1_000_000);
    sb.set_byte_limit(1 << 20);
    let mut r = Rng(5);
    let styles = random_styles(&mut r, 4000);
    let mut row = Row::new(160, Cell::BLANK);
    for i in 0..20_000 {
        // A gradient: a new style in every cell.
        for (c, cell) in row.cells.iter_mut().enumerate() {
            *cell = template(&styles, 1 + (i * 160 + c) % 4000).with_char('█');
        }
        sb.push(&row, &styles);
        assert!(sb.sealed_bytes <= 1 << 20);
        assert!(sb.memory_bytes() <= (1 << 20) + (1 << 20), "{}", sb.memory_bytes());
    }
    assert!(sb.len() < 20_000);
    assert!(sb.stats().sealed_bytes <= 1 << 20);
    sb.check_all().unwrap();
    // The lines left read back.
    let (back, back_styles) = sb.get(0).unwrap();
    assert_eq!(back.cells.len(), 160);
    assert!(back_styles.len() > 100);
}

#[test]
fn blocks_seal_before_any_part_passes_64_kb() {
    let mut r = Rng(11);
    // Plain text fills the text part first; a gradient the dictionary.
    let styles = random_styles(&mut r, 20_000);
    let mut sb = Scrollback::new(1_000_000);
    let mut row = Row::new(300, Cell::BLANK);
    for i in 0..3000 {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            *cell = if i % 3 == 0 {
                template(&styles, 1 + (i * 300 + c) % 20_000).with_char('a')
            } else {
                Cell::BLANK.with_char('b')
            };
        }
        sb.push(&row, &styles);
        let o = &sb.open;
        assert!(o.meta.capacity() <= PART + 1 && o.text.capacity() <= PART + 1 && o.styles.capacity() <= PART + 1);
    }
    sb.check_all().unwrap();
    for (d, _) in sb.sealed() {
        assert!(d.meta.len() <= PART && d.text.len() <= PART && d.styles.len() <= PART);
    }
}

#[test]
fn ambiguous_setting_is_kept_per_block() {
    let d = [Style::DEFAULT];
    let mut sb = Scrollback::new(100);
    let mut narrow = Row::new(10, Cell::BLANK);
    put(&mut narrow, &d, 0, 0, "±x");
    let mut wide = Row::new(10, Cell::BLANK);
    wide.cells[0] = Cell::BLANK.with_char('±').with_width(Width::Wide);
    wide.cells[1] = Cell::BLANK.with_width(Width::Spacer);
    wide.cells[2] = Cell::BLANK.with_char('x');
    sb.push(&narrow, &d);
    sb.set_ambiguous(2);
    assert_eq!(sb.stats().sealed_blocks, 1, "the setting sealed the block");
    sb.set_ambiguous(2);
    sb.push(&wide, &d);
    // A row written before the change but pushed after: an exception.
    sb.push(&narrow, &d);
    sb.set_ambiguous(1);
    sb.set_ambiguous(2);
    assert_eq!(sb.stats().sealed_blocks, 2, "an empty block just takes the setting");
    let flags = |sb: &Scrollback, i| sb.locate(i).map(|(v, n)| v.flags(n)).unwrap();
    assert_eq!(flags(&sb, 1) & flag::WIDTHS, 0, "± is two wide at 2");
    assert_ne!(flags(&sb, 2) & flag::WIDTHS, 0);
    for (i, row) in [&narrow, &wide, &narrow].into_iter().enumerate() {
        let (back, back_styles) = sb.get(i).unwrap();
        assert_eq!(normalise(&back, &back_styles), normalise(row, &d), "row {i}");
    }
    let (blocks, ambiguous): (Vec<_>, Vec<_>) = sb.sealed().map(|(d, n)| (n, d.ambiguous())).unzip();
    assert_eq!((blocks, ambiguous), (vec![1, 2], vec![1, 2]));
}

#[test]
fn checkpoints_find_every_line() {
    let mut sb = Scrollback::new(1000);
    let mut row = Row::new(40, Cell::BLANK);
    for i in 0..500usize {
        row.reset(40, Cell::BLANK);
        put(&mut row, &[Style::DEFAULT], 0, 0, &"z".repeat(i % 37));
        row.wrapped = i % 5 == 0;
        sb.push(&row, &[Style::DEFAULT]);
    }
    sb.check_all().unwrap();
    for i in 0..500usize {
        let text = sb.text(i).unwrap();
        let want = if i % 5 == 0 { 40 } else { i % 37 };
        assert_eq!(text.len(), want, "line {i}");
    }
}

#[test]
fn plain_lines_are_small() {
    let mut sb = Scrollback::new(100_000);
    let mut row = Row::new(200, Cell::BLANK);
    put(&mut row, &[Style::DEFAULT], 0, 0, "the quick brown fox jumps over the lazy dog");
    for _ in 0..10_000 {
        sb.push(&row, &[Style::DEFAULT]);
    }
    let s = sb.stats();
    let sealed = s.sealed_bytes / s.sealed_lines;
    assert!(sealed <= 47, "{sealed} sealed bytes per line");
    let per_line = sb.memory_bytes() / 10_000;
    assert!(per_line <= 52, "{per_line} bytes per line");
    assert_eq!(sb.get(9_999).unwrap().0.cells.len(), 43);
}
