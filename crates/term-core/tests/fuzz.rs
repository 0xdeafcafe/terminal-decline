//! Robustness: pseudo-random byte streams heavy on escape sequences, fed at
//! various sizes with resizes in between. Must never panic, and the terminal
//! invariants (cursor in bounds, row widths, scrollback cap) must hold.

use term_core::Term;

/// xorshift64* — deterministic, no dependencies.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn push_random_bytes(out: &mut Vec<u8>, rng: &mut Rng, n: usize) {
    const CSI_FINALS: &[u8] = b"@ABCDEFGHIJKLMPSTXZ`bcdefghlnqrsum~";
    const ESC_FINALS: &[u8] = b"78DEMc=>()#012B";
    for _ in 0..n {
        match rng.below(100) {
            0..=39 => {
                // printable ASCII, biased to short runs
                let c = b' ' + (rng.below(95)) as u8;
                out.push(c);
            }
            40..=49 => {
                // wide / non-ASCII characters
                const WIDE: &[&str] = &[
                    "漢",
                    "字",
                    "あ",
                    "🙂",
                    "é",
                    "界",
                    "가",
                    "\u{301}",
                    "\u{FE0F}",
                    "\u{FE0E}",
                    "\u{200D}",
                    "\u{20E3}",
                    "\u{1F1EC}",
                    "\u{1F3FD}",
                    "❤",
                    "⌚",
                    "§",
                    "\u{94D}",
                    "\u{915}",
                    "\u{600}",
                    "\u{1161}",
                    "\x1b[?2027l",
                    "\x1b[?2027h",
                ];
                out.extend_from_slice(WIDE[rng.below(WIDE.len())].as_bytes());
            }
            50..=57 => {
                // C0 controls
                const CTL: &[u8] = b"\x07\x08\x09\x0a\x0b\x0c\x0d";
                out.push(CTL[rng.below(CTL.len())]);
            }
            58..=77 => {
                // CSI with random params, maybe private/intermediate
                out.extend_from_slice(b"\x1b[");
                if rng.below(3) == 0 {
                    out.push(*b"?!> ".get(rng.below(4)).unwrap());
                }
                for _ in 0..rng.below(4) {
                    let p = rng.below(2000);
                    out.extend_from_slice(p.to_string().as_bytes());
                    match rng.below(4) {
                        0 => out.push(b';'),
                        1 => out.push(b':'),
                        _ => {}
                    }
                }
                out.push(CSI_FINALS[rng.below(CSI_FINALS.len())]);
            }
            78..=82 => {
                // ESC sequence
                out.push(0x1b);
                out.push(ESC_FINALS[rng.below(ESC_FINALS.len())]);
            }
            83..=87 => {
                // OSC string, BEL or ST terminated
                out.extend_from_slice(b"\x1b]");
                out.extend_from_slice(b"012"); // title-ish codes
                out.push(b';');
                for _ in 0..rng.below(12) {
                    out.push(b' ' + rng.below(95) as u8);
                }
                if rng.below(2) == 0 {
                    out.push(0x07);
                } else {
                    out.extend_from_slice(b"\x1b\\");
                }
            }
            88..=92 => {
                // raw bytes, possibly invalid UTF-8
                out.push(rng.next() as u8);
            }
            _ => {
                // plausible program output: words and newlines
                const WORDS: &[&str] = &["hello ", "world", "\r\n", "ls -la\r\n", "~ $ ", "ok\r\n"];
                out.extend_from_slice(WORDS[rng.below(WORDS.len())].as_bytes());
            }
        }
    }
}

fn check_invariants(t: &Term, cols: usize, lines: usize, limit: usize) {
    assert!(t.state.cursor().row() < lines, "cursor row {} >= {lines}", t.state.cursor().row());
    assert!(t.state.cursor().col() < cols, "cursor col {} >= {cols}", t.state.cursor().col());
    assert!(
        !t.state.cursor().pending_wrap() || t.state.cursor().col() == cols - 1,
        "pending wrap away from the last column"
    );
    assert!(t.state.scrollback_len() <= limit, "scrollback over the limit");
    let sb = t.state.scrollback_len();
    let off = t.state.display_offset();
    for l in 0..lines {
        let row = t.state.visible_row(l); // must not panic
        if sb - off + l >= sb {
            assert_eq!(row.len(), cols, "screen row {l} has wrong width");
        }
    }
}

fn run_stream(seed: u64, sizes: &[(usize, usize, usize)], bytes_per_size: usize) {
    for &(cols, lines, limit) in sizes {
        let mut rng = Rng(seed ^ (cols as u64) << 32 ^ lines as u64);
        let mut t = Term::new(cols, lines, limit, term_core::ImagePool::process());
        // Half the sizes sweep their style tables constantly (§5.2).
        if cols % 2 == 0 {
            t.state.set_sweep_at(Some(8));
        }
        let mut fed = 0;
        while fed < bytes_per_size {
            let mut chunk = Vec::new();
            let n = 2 * 1024 + rng.below(6 * 1024);
            push_random_bytes(&mut chunk, &mut rng, n);
            fed += chunk.len();
            t.advance(&chunk);
            match rng.below(10) {
                0 => {
                    let c = 2 + rng.below(120);
                    let l = 1 + rng.below(50);
                    t.state.resize(c, l);
                    check_invariants(&t, c.max(2), l.max(1), limit);
                    t.state.resize(cols, lines);
                }
                1 => {
                    t.state.scroll_view(rng.below(50) as isize);
                    t.state.scroll_view(-(rng.below(50) as isize));
                }
                2 => {
                    let a = t.state.point_at(rng.below(lines.max(1)), rng.below(cols + 2));
                    let b = t.state.point_at(rng.below(lines.max(1)), rng.below(cols + 2));
                    t.state.set_selection(Some(term_core::Selection::new(a, b)));
                    let _ = t.state.selection_text();
                    t.state.set_selection(None);
                }
                _ => {}
            }
            check_invariants(&t, cols.max(2), lines.max(1), limit);
        }
    }
}

#[test]
fn random_streams_at_various_sizes() {
    run_stream(0x5EED, &[(2, 1, 0), (5, 3, 10), (80, 24, 100), (17, 5, 1000)], 1 << 20);
}

#[test]
fn random_stream_chunking_is_transparent() {
    // Feeding byte-by-byte must give the same state as one big advance.
    let mut rng = Rng(0xC0FFEE);
    let mut stream = Vec::new();
    push_random_bytes(&mut stream, &mut rng, 32 * 1024);

    let mut whole = Term::new(20, 6, 100, term_core::ImagePool::process());
    whole.advance(&stream);
    let mut drip = Term::new(20, 6, 100, term_core::ImagePool::process());
    for b in stream.chunks(3) {
        drip.advance(b);
    }
    assert_eq!(
        (0..6)
            .map(|l| { whole.state.visible_row(l).cells().map(|c| c.ch()).collect::<String>() })
            .collect::<Vec<_>>(),
        (0..6)
            .map(|l| drip.state.visible_row(l).cells().map(|c| c.ch()).collect::<String>())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        (whole.state.cursor().row(), whole.state.cursor().col(), whole.state.scrollback_len()),
        (drip.state.cursor().row(), drip.state.cursor().col(), drip.state.scrollback_len())
    );
}
