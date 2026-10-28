//! Mouse reporting to the program (X10 and SGR encodings, xterm button and
//! modifier codes) and the word/line spans for double- and triple-click
//! selection.

use term_core::MouseMode;

/// Button numbers as xterm reports them.
pub const LEFT: u8 = 0;
pub const MIDDLE: u8 = 1;
pub const RIGHT: u8 = 2;
/// "No button" in motion reports.
pub const NONE: u8 = 3;
/// Added to the button code for motion reports.
pub const MOTION: u8 = 32;
pub const WHEEL_UP: u8 = 64;
pub const WHEEL_DOWN: u8 = 65;
/// At most this many wheel reports per scroll event.
pub const MAX_WHEEL_REPORTS: usize = 10;
/// The largest column or line (0-based) X10 encoding can carry.
pub const X10_MAX: usize = 222;

/// xterm's modifier bits in a button code: Shift 4, Meta (Option) 8,
/// Control 16.
pub fn modifier_bits(shift: bool, option: bool, ctrl: bool) -> u8 {
    (shift as u8) * 4 + (option as u8) * 8 + (ctrl as u8) * 16
}

/// Whether events go to the program: it asked for them and Shift isn't held
/// (Shift forces local selection).
pub fn reports(mode: MouseMode, shift: bool) -> bool {
    !shift && mode != MouseMode::Off
}

/// One mouse report for a 0-based `line` and `col`. `None` when X10 can't
/// carry the position.
pub fn encode(code: u8, line: usize, col: usize, press: bool, sgr: bool) -> Option<Vec<u8>> {
    if sgr {
        return Some(format!("\x1b[<{code};{};{}{}", col + 1, line + 1, if press { 'M' } else { 'm' }).into_bytes());
    }
    if col > X10_MAX || line > X10_MAX {
        return None;
    }
    // X10 has no release per button: release is button 3, keeping modifiers.
    let b = if press { code } else { 3 | (code & !3) };
    Some(vec![0x1b, b'[', b'M', 32 + b, 33 + col as u8, 33 + line as u8])
}

/// Whether a mouse move is reported: always in any-motion mode (1003), only
/// with a button held in drag mode (1002), never otherwise.
pub fn reports_motion(mode: MouseMode, held: Option<u8>) -> bool {
    match mode {
        MouseMode::Motion => true,
        MouseMode::Drag => held.is_some(),
        _ => false,
    }
}

/// Button code for a motion report: the held button (or the one the event
/// is for, or "none"), plus 32, plus modifiers.
pub fn motion_code(held: Option<u8>, button: Option<u8>, mods: u8) -> u8 {
    MOTION + held.or(button).unwrap_or(NONE) + mods
}

/// Button code for a wheel report; positive `lines` scroll up.
pub fn wheel_code(lines: isize, mods: u8) -> u8 {
    let base = if lines > 0 { WHEEL_UP } else { WHEEL_DOWN };
    base + mods
}

/// Characters that end a word for double-click selection (besides spaces).
const WORD_BREAKS: &str = "()[]{}<>'\"`|,;";

pub fn is_word_char(c: char) -> bool {
    !c.is_whitespace() && !WORD_BREAKS.contains(c)
}

/// The columns (inclusive) of the word under `col` in `row`, or `None` when
/// `col` isn't on a word (the selection is then just that cell).
pub fn word_at(row: &[char], col: usize) -> Option<(usize, usize)> {
    let is_word = |c: usize| row.get(c).copied().is_some_and(is_word_char);
    if !is_word(col) {
        return None;
    }
    let mut a = col;
    while a > 0 && is_word(a - 1) {
        a -= 1;
    }
    let mut b = col;
    while b + 1 < row.len() && is_word(b + 1) {
        b += 1;
    }
    Some((a, b))
}

/// The columns (inclusive) a triple-click selects: the whole line.
pub fn line_span(cols: usize) -> (usize, usize) {
    (0, cols.saturating_sub(1))
}
