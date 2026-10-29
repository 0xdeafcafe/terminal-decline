//! Mouse reporting to the program (X10 and SGR encodings, xterm button and
//! modifier codes). Word and line selection live in term-core
//! (`Terminal::select_word`, `select_line`).

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

#[cfg(test)]
mod tests {
    use super::*;

    fn x10(b: u8, col: u8, line: u8) -> Vec<u8> {
        vec![0x1b, b'[', b'M', b, col, line]
    }

    #[test]
    fn reports_in_sgr_and_x10() {
        let shift_ctrl = modifier_bits(true, false, true);
        // (what, code, line, col, press, SGR bytes, X10 bytes)
        type Row = (&'static str, u8, usize, usize, bool, &'static str, Option<Vec<u8>>);
        let rows: Vec<Row> = vec![
            ("left press at 1,1", LEFT, 0, 0, true, "\x1b[<0;1;1M", Some(x10(32, 33, 33))),
            ("left release at 1,1", LEFT, 0, 0, false, "\x1b[<0;1;1m", Some(x10(35, 33, 33))),
            ("middle press", MIDDLE, 4, 9, true, "\x1b[<1;10;5M", Some(x10(33, 42, 37))),
            ("right press", RIGHT, 2, 4, true, "\x1b[<2;5;3M", Some(x10(34, 37, 35))),
            ("right release", RIGHT, 2, 4, false, "\x1b[<2;5;3m", Some(x10(35, 37, 35))),
            (
                "shift+ctrl+left press",
                LEFT + shift_ctrl,
                0,
                0,
                true,
                "\x1b[<20;1;1M",
                Some(x10(52, 33, 33)),
            ),
            (
                "shift+ctrl+left release keeps mods",
                LEFT + shift_ctrl,
                0,
                0,
                false,
                "\x1b[<20;1;1m",
                Some(x10(55, 33, 33)),
            ),
            (
                "drag with left",
                motion_code(Some(LEFT), Some(LEFT), 0),
                1,
                1,
                true,
                "\x1b[<32;2;2M",
                Some(x10(64, 34, 34)),
            ),
            (
                "motion, no button",
                motion_code(None, None, 0),
                1,
                1,
                true,
                "\x1b[<35;2;2M",
                Some(x10(67, 34, 34)),
            ),
            ("wheel up", wheel_code(1, 0), 0, 0, true, "\x1b[<64;1;1M", Some(x10(96, 33, 33))),
            ("wheel down", wheel_code(-1, 0), 0, 0, true, "\x1b[<65;1;1M", Some(x10(97, 33, 33))),
            (
                "option+wheel up",
                wheel_code(3, modifier_bits(false, true, false)),
                0,
                0,
                true,
                "\x1b[<72;1;1M",
                Some(x10(104, 33, 33)),
            ),
            // 223 is the last column and line X10 can carry.
            ("at 223,223", LEFT, 222, 222, true, "\x1b[<0;223;223M", Some(x10(32, 255, 255))),
            ("column 224", LEFT, 0, 223, true, "\x1b[<0;224;1M", None),
            ("line 224", LEFT, 223, 0, true, "\x1b[<0;1;224M", None),
            ("far away", LEFT, 999, 4000, true, "\x1b[<0;4001;1000M", None),
        ];
        for (what, code, line, col, press, sgr, x) in rows {
            assert_eq!(
                encode(code, line, col, press, true).map(|b| String::from_utf8(b).unwrap()),
                Some(sgr.to_string()),
                "SGR {what}"
            );
            assert_eq!(encode(code, line, col, press, false), x, "X10 {what}");
        }
    }

    #[test]
    fn modifiers() {
        for (shift, option, ctrl, want) in [
            (false, false, false, 0),
            (true, false, false, 4),
            (false, true, false, 8),
            (false, false, true, 16),
            (true, true, true, 28),
        ] {
            assert_eq!(modifier_bits(shift, option, ctrl), want);
        }
    }

    #[test]
    fn which_events_are_reported() {
        use MouseMode::*;
        for (mode, shift, want) in [
            (Off, false, false),
            (Click, false, true),
            (Drag, false, true),
            (Motion, false, true),
            (Click, true, false),
            (Motion, true, false),
        ] {
            assert_eq!(reports(mode, shift), want, "{mode:?} shift={shift}");
        }
        for (mode, held, want) in [
            (Off, None, false),
            (Off, Some(LEFT), false),
            (Click, None, false),
            (Click, Some(LEFT), false),
            (Drag, None, false),
            (Drag, Some(RIGHT), true),
            (Motion, None, true),
            (Motion, Some(LEFT), true),
        ] {
            assert_eq!(reports_motion(mode, held), want, "{mode:?} held={held:?}");
        }
    }

    #[test]
    fn motion_and_wheel_codes() {
        for (held, button, mods, want) in [
            (Some(LEFT), None, 0, 32),
            (None, Some(LEFT), 0, 32),
            (Some(RIGHT), Some(LEFT), 0, 34),
            (None, None, 0, 35),
            (Some(MIDDLE), None, 8, 41),
        ] {
            assert_eq!(motion_code(held, button, mods), want);
        }
        for (lines, mods, want) in [(1, 0, 64), (10, 0, 64), (-1, 0, 65), (-4, 16, 81), (2, 4, 68)] {
            assert_eq!(wheel_code(lines, mods), want);
        }
    }
}
