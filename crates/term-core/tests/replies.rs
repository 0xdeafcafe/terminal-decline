//! Replies programs use to find out what the terminal supports, and the
//! colours they set and ask about: XTGETTCAP, DECRQSS, OSC 4/5/10–12 and
//! their resets, DECID, DECXCPR, DECREQTPARM and the title stack. Expected
//! values follow xterm (ctlseqs) and esctest2.

use term_core::{DynamicColor, Term};

fn new(cols: usize, lines: usize) -> Term {
    Term::new(cols, lines, 100, term_core::ImagePool::process())
}

/// Send `input`, return the replies as a string and clear them.
fn ask(t: &mut Term, input: &str) -> String {
    t.advance(input.as_bytes());
    String::from_utf8(t.state.take_replies()).unwrap()
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02X}")).collect()
}

fn unhex(s: &str) -> String {
    let bytes: Vec<u8> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect();
    String::from_utf8(bytes).unwrap()
}

fn text(t: &Term, line: usize) -> String {
    let row = t.state.visible_row(line);
    let s: String = row.cells().filter(|c| !c.is_spacer()).map(|c| c.ch()).collect();
    s.trim_end().to_string()
}

// ------------------------------------------------------------- XTGETTCAP

/// What XTGETTCAP says for `name`: None if unknown, Some("") for a flag.
fn tcap(t: &mut Term, name: &str) -> Option<String> {
    let r = ask(t, &format!("\x1bP+q{}\x1b\\", hex(name)));
    if let Some(rest) = r.strip_prefix(&format!("\x1bP1+r{}", hex(name))) {
        let body = rest.strip_suffix("\x1b\\").unwrap();
        return Some(body.strip_prefix('=').map(unhex).unwrap_or_default());
    }
    assert_eq!(r, format!("\x1bP0+r{}\x1b\\", hex(name)));
    None
}

#[test]
fn xtgettcap_answers_what_neovim_asks() {
    let mut t = new(10, 2);
    assert_eq!(tcap(&mut t, "Tc").as_deref(), Some(""));
    assert_eq!(tcap(&mut t, "RGB").as_deref(), Some(""));
    assert_eq!(tcap(&mut t, "Su").as_deref(), Some(""));
    assert_eq!(tcap(&mut t, "Smulx").as_deref(), Some("\x1b[4:%p1%dm"));
    let setulc = tcap(&mut t, "Setulc").unwrap();
    assert!(setulc.starts_with("\x1b[58:2::"), "{setulc:?}");
    assert_eq!(tcap(&mut t, "setrgbf").as_deref(), Some("\x1b[38:2::%p1%d:%p2%d:%p3%dm"));
    assert_eq!(tcap(&mut t, "Co").as_deref(), Some("256"));
    assert_eq!(tcap(&mut t, "colors").as_deref(), Some("256"));
    assert_eq!(tcap(&mut t, "TN").as_deref(), Some("xterm-256color"));
    assert_eq!(tcap(&mut t, "Ss").as_deref(), Some("\x1b[%p1%d q"));
    assert_eq!(tcap(&mut t, "nonsense"), None);
}

#[test]
fn xtgettcap_answers_each_name() {
    let mut t = new(10, 2);
    let r = ask(&mut t, &format!("\x1bP+q{};{};{}\x1b\\", hex("Tc"), hex("bogus"), hex("Co")));
    assert_eq!(
        r,
        format!(
            "\x1bP1+r{}\x1b\\\x1bP0+r{}\x1b\\\x1bP1+r{}={}\x1b\\",
            hex("Tc"),
            hex("bogus"),
            hex("Co"),
            hex("256")
        )
    );
    // Lower-case hex and bad hex.
    assert_eq!(ask(&mut t, "\x1bP+q5463\x1b\\"), "\x1bP1+r5463\x1b\\");
    assert_eq!(ask(&mut t, "\x1bP+q546\x1b\\"), "\x1bP0+r546\x1b\\");
}

#[test]
fn abandoned_or_other_dcs_strings_get_no_reply() {
    let mut t = new(10, 2);
    assert_eq!(ask(&mut t, "\x1bP+q5463\x18"), "", "cancelled with CAN");
    assert_eq!(ask(&mut t, "\x1bPq#0;2;0;0;0\x1b\\"), "", "sixel");
    assert_eq!(ask(&mut t, &format!("\x1bP+q{}\x1b\\", "54".repeat(2000))), "", "too long");
    assert_eq!(ask(&mut t, "\x1bP$qm\x1b\\"), "\x1bP1$r0m\x1b\\", "and the next one works");
}

// ---------------------------------------------------------------- DECRQSS

#[test]
fn decrqss_sgr() {
    let mut t = new(10, 2);
    assert_eq!(ask(&mut t, "\x1bP$qm\x1b\\"), "\x1bP1$r0m\x1b\\");
    assert_eq!(ask(&mut t, "\x1b[1m\x1bP$qm\x1b\\"), "\x1bP1$r0;1m\x1b\\", "esctest DECRQSS_SGR");
    assert_eq!(
        ask(&mut t, "\x1b[0;3;4:3;31;48;2;1;2;3;58:5:9m\x1bP$qm\x1b\\"),
        "\x1bP1$r0;3;4:3;31;48;2;1;2;3;58;5;9m\x1b\\"
    );
    // Neovim checks true colour this way: what it set comes back.
    assert_eq!(
        ask(&mut t, "\x1b[0m\x1b[48;2;10;20;30m\x1bP$qm\x1b\\"),
        "\x1bP1$r0;48;2;10;20;30m\x1b\\"
    );
}

#[test]
fn decrqss_cursor_style_and_margins() {
    let mut t = new(10, 10);
    assert_eq!(
        ask(&mut t, "\x1b[4 q\x1bP$q q\x1b\\"),
        "\x1bP1$r4 q\x1b\\",
        "esctest DECRQSS_DECSCUSR"
    );
    assert_eq!(ask(&mut t, "\x1b[5 q\x1bP$q q\x1b\\"), "\x1bP1$r5 q\x1b\\");
    assert_eq!(ask(&mut t, "\x1b[0 q\x1bP$q q\x1b\\"), "\x1bP1$r1 q\x1b\\");
    assert_eq!(ask(&mut t, "\x1bP$qr\x1b\\"), "\x1bP1$r1;10r\x1b\\");
    assert_eq!(
        ask(&mut t, "\x1b[5;6r\x1bP$qr\x1b\\"),
        "\x1bP1$r5;6r\x1b\\",
        "esctest DECRQSS_DECSTBM"
    );
    assert_eq!(ask(&mut t, "\x1bP$q\"q\x1b\\"), "\x1bP0$r\x1b\\", "not one we know");
}

// ----------------------------------------------------------------- colours

#[test]
fn osc4_sets_queries_and_resets_the_palette() {
    let mut t = new(10, 2);
    let original = ask(&mut t, "\x1b]4;1;?\x1b\\");
    assert_eq!(original, "\x1b]4;1;rgb:cccc/6666/6666\x1b\\");
    t.state.take_colors_changed();
    for (spec, want) in [
        ("rgb:f0f0/f0f0/f0f0", "f0f0/f0f0/f0f0"),
        ("#fff", "f0f0/f0f0/f0f0"),
        ("#808080", "8080/8080/8080"),
        ("#800800800", "8080/8080/8080"),
        ("#aaaabbbbcccc", "aaaa/bbbb/cccc"),
    ] {
        let r = ask(&mut t, &format!("\x1b]4;1;{spec}\x1b\\\x1b]4;1;?\x1b\\"));
        assert_eq!(r, format!("\x1b]4;1;rgb:{want}\x1b\\"), "{spec}");
    }
    assert!(t.state.take_colors_changed());
    assert_eq!(t.state.palette()[1], 0xaabbcc);
    // Several in one; a query ended with BEL is answered with ST.
    let r = ask(&mut t, "\x1b]4;0;#102030;200;#405060\x07\x1b]4;0;?;200;?\x07");
    assert_eq!(r, "\x1b]4;0;rgb:1010/2020/3030\x1b\\\x1b]4;200;rgb:4040/5050/6060\x1b\\");
    // One reset, then all.
    ask(&mut t, "\x1b]104;1\x1b\\");
    assert_eq!(ask(&mut t, "\x1b]4;1;?\x1b\\"), original);
    assert_eq!(t.state.palette()[0], 0x102030);
    ask(&mut t, "\x1b]104\x1b\\");
    assert_eq!(t.state.palette()[0], 0x1d1f21);
    assert_eq!(t.state.palette()[200], 0xff00d7);
}

#[test]
fn special_colours_by_osc5_and_after_the_palette() {
    let mut t = new(10, 2);
    let r = ask(&mut t, "\x1b]5;0;#f0f0f0;1;#f00000\x1b\\\x1b]5;0;?;1;?\x1b\\");
    assert_eq!(r, "\x1b]5;0;rgb:f0f0/f0f0/f0f0\x1b\\\x1b]5;1;rgb:f0f0/0000/0000\x1b\\");
    assert_eq!(ask(&mut t, "\x1b]4;256;?\x1b\\"), "\x1b]4;256;rgb:f0f0/f0f0/f0f0\x1b\\");
    ask(&mut t, "\x1b]105;0\x1b\\");
    assert_eq!(
        ask(&mut t, "\x1b]4;256;?\x1b\\"),
        "\x1b]4;256;rgb:d8d8/dede/e9e9\x1b\\",
        "the foreground"
    );
    assert_eq!(ask(&mut t, "\x1b]4;261;?\x1b\\"), "", "past the special colours");
}

#[test]
fn osc10_to_12_set_query_and_reset() {
    let mut t = new(10, 2);
    t.state.set_default_colors([0x111111, 0x222222, 0x333333]);
    let r = ask(&mut t, "\x1b]10;#f0f0f0;#f00000\x1b\\\x1b]10;?;?;?\x1b\\");
    assert_eq!(
        r, "\x1b]10;rgb:f0f0/f0f0/f0f0\x1b\\\x1b]11;rgb:f0f0/0000/0000\x1b\\\x1b]12;rgb:3333/3333/3333\x1b\\",
        "each parameter is the next colour"
    );
    assert_eq!(t.state.dynamic_color(DynamicColor::Background), 0xf00000);
    ask(&mut t, "\x1b]12;rgb:ff/00/ff\x07");
    assert_eq!(t.state.dynamic_color(DynamicColor::Cursor), 0xff00ff);
    // The theme changes: what the program set stays.
    t.state.set_default_colors([0x444444, 0x555555, 0x666666]);
    assert_eq!(t.state.dynamic_color(DynamicColor::Foreground), 0xf0f0f0);
    assert_eq!(t.state.default_colors(), [0x444444, 0x555555, 0x666666]);
    ask(&mut t, "\x1b]110\x1b\\\x1b]111\x1b\\");
    assert_eq!(ask(&mut t, "\x1b]10;?\x1b\\"), "\x1b]10;rgb:4444/4444/4444\x1b\\");
    assert_eq!(ask(&mut t, "\x1b]11;?\x07"), "\x1b]11;rgb:5555/5555/5555\x1b\\");
    assert_eq!(t.state.dynamic_color(DynamicColor::Cursor), 0xff00ff);
    ask(&mut t, "\x1b]112\x1b\\");
    assert_eq!(t.state.dynamic_color(DynamicColor::Cursor), 0x666666);
}

#[test]
fn bad_colours_are_ignored() {
    let mut t = new(10, 2);
    t.advance(b"\x1b]4;1;red\x1b\\\x1b]4;1;#12\x1b\\\x1b]10;CIEXYZ:1/1/1\x1b\\\x1b]4;x;#fff\x1b\\");
    assert!(!t.state.take_colors_changed());
    assert_eq!(t.state.palette()[1], 0xcc6666);
}

#[test]
fn full_reset_drops_program_colours_but_keeps_the_theme() {
    let mut t = new(10, 2);
    let mut theme = [0u32; 256];
    theme[3] = 0x010203;
    t.state.set_default_palette(&theme);
    t.state.set_default_colors([1, 2, 3]);
    t.advance(b"\x1b]4;3;#ffffff\x1b\\\x1b]11;#ffffff\x1b\\");
    assert_eq!(t.state.palette()[3], 0xffffff);
    t.state.take_colors_changed();
    t.advance(b"\x1bc");
    assert!(t.state.take_colors_changed());
    assert_eq!(t.state.palette()[3], 0x010203);
    assert_eq!(t.state.dynamic_color(DynamicColor::Background), 2);
}

// ------------------------------------------------------------ reports

#[test]
fn decid_answers_as_primary_da() {
    let mut t = new(10, 2);
    assert_eq!(ask(&mut t, "\x1bZ"), "\x1b[?62;22c");
}

#[test]
fn decxcpr() {
    let mut t = new(10, 10);
    assert_eq!(ask(&mut t, "\x1b[5;6H\x1b[?6n"), "\x1b[?5;6R");
    assert_eq!(ask(&mut t, "\x1b[3;8r\x1b[?6h\x1b[2;4H\x1b[?6n"), "\x1b[?2;4R", "origin mode");
}

#[test]
fn decreqtparm() {
    let mut t = new(10, 2);
    assert_eq!(ask(&mut t, "\x1b[x"), "\x1b[2;1;1;128;128;1;0x");
    assert_eq!(ask(&mut t, "\x1b[1x"), "\x1b[3;1;1;128;128;1;0x");
    assert_eq!(ask(&mut t, "\x1b[2x"), "");
}

// ---------------------------------------------------------- title stack

#[test]
fn title_push_and_pop() {
    let mut t = new(10, 2);
    t.advance(b"\x1b]2;shell\x07\x1b[22;0t\x1b]2;vim\x07");
    assert_eq!(t.state.take_title().as_deref(), Some("vim"));
    t.advance(b"\x1b[23;0t");
    assert_eq!(t.state.take_title().as_deref(), Some("shell"));
    t.advance(b"\x1b[23;0t");
    assert_eq!(t.state.take_title(), None, "nothing left to pop");
    // The icon label alone (1) isn't kept.
    t.advance(b"\x1b[22;1t\x1b]2;x\x07\x1b[23;1t");
    assert_eq!(t.state.take_title().as_deref(), Some("x"));
    // At most 10 kept: the oldest go.
    for i in 0..12 {
        t.advance(format!("\x1b]2;t{i}\x07\x1b[22t").as_bytes());
    }
    for i in (2..12).rev() {
        t.advance(b"\x1b[23t");
        assert_eq!(t.state.take_title(), Some(format!("t{i}")));
    }
    t.advance(b"\x1b[23t");
    assert_eq!(t.state.take_title(), None);
}

// --------------------------------------------- E.19 conformance fixes

#[test]
fn dec_graphics_control_pictures() {
    let mut t = new(12, 1);
    t.advance(b"\x1b(0bcdehi_a\x1b(B_");
    assert_eq!(text(&t, 0), "␉␌␍␊␤␋ ▒_");
}

#[test]
fn british_charset() {
    let mut t = new(10, 1);
    t.advance(b"\x1b)A#\x0e#\x0f#\x1b(A#\x1b(B#");
    assert_eq!(text(&t, 0), "#£#£#");
}

#[test]
fn rep_after_rep_repeats_nothing() {
    let mut t = new(20, 1);
    t.advance(b"+\x1b[1b\x1b[10b");
    assert_eq!(text(&t, 0), "++", "vttest iso6429-rep");
}

#[test]
fn decaln_resets_margins_and_homes_the_cursor() {
    let mut t = new(10, 10);
    t.advance(b"\x1b[4;5r\x1b[5;5H\x1b#8");
    assert_eq!((t.state.cursor().row(), t.state.cursor().col()), (0, 0));
    assert_eq!(text(&t, 9), "EEEEEEEEEE");
    // The margins are gone: CUU from row 4 goes past row 4's top margin.
    t.advance(b"\x1b[4;2H\x1b[A\x1b[?6n");
    assert_eq!(String::from_utf8(t.state.take_replies()).unwrap(), "\x1b[?3;2R");
    t.advance(b"\x1bP$qr\x1b\\");
    assert_eq!(String::from_utf8(t.state.take_replies()).unwrap(), "\x1bP1$r1;10r\x1b\\");
}
