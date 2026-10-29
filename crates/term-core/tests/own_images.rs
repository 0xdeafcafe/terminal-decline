//! Decline's own images (images.md §7, §8, PR I.3): iTerm2 inline files
//! (`OSC 1337 ; File=`, `MultipartFile=`) and Sixel, shown through
//! placeholder cells, with the modes (80, 1070, 8452), DA1 and
//! XTSMGRAPHICS, the budget and freeing.

use std::sync::Arc;

use term_core::image::{Fit, Image, ImageKey, ImageState, RustHost, run_job};
use term_core::{Content, ImageJob, ImagePool, Snapshot, Term};

#[path = "../../term-image/tests/support/sixel_encode.rs"]
mod sixel_encode;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64(data: &[u8]) -> String {
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A PNG of `w` × `h`; `noise` makes it hard to compress (a large file).
fn png(w: u32, h: u32, noise: bool) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut wr = e.write_header().unwrap();
        let mut x = 0x1234_5678u32;
        let data: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                if noise {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    [x as u8, (x >> 8) as u8, (x >> 16) as u8, 0xFF]
                } else {
                    [i as u8, 0x40, 0x80, 0xFF]
                }
            })
            .collect();
        wr.write_image_data(&data).unwrap();
    }
    out
}

/// What iTerm2's imgcat sends for `data`.
fn imgcat(args: &str, data: &[u8]) -> Vec<u8> {
    format!("\x1b]1337;File=inline=1;size={};{args}:{}\x07", data.len(), b64(data)).into_bytes()
}

fn term_with(cols: usize, lines: usize, budget: usize) -> Term {
    let mut t = Term::new(cols, lines, 100, ImagePool::new(budget));
    t.state.set_image_host(Arc::new(RustHost::new()));
    t.state.set_cell_pixels((10, 20));
    t
}

fn term() -> Term {
    term_with(20, 6, 32 << 20)
}

fn run_jobs(t: &mut Term) {
    let mut jobs: Vec<ImageJob> = Vec::new();
    t.state.take_image_jobs(&mut jobs);
    let host = t.state.image_host().clone();
    for job in jobs {
        let done = run_job(&*host, t.state.terminal_id(), job);
        t.state.image_job_done(done);
    }
}

fn check(t: &Term) {
    t.state.check_invariants().unwrap();
    t.state.image_pool().check().unwrap();
}

fn own(t: &Term) -> Vec<Image> {
    let mut v: Vec<Image> = t
        .state
        .graphics()
        .images()
        .filter(|i| matches!(i.key, ImageKey::Own(_)))
        .cloned()
        .collect();
    v.sort_by_key(|i| i.key);
    v
}

/// The placeholder pieces in view row `line`.
fn pieces(t: &Term, line: usize) -> Vec<Option<(u16, u16)>> {
    t.state
        .visible_row(line)
        .cells()
        .map(|c| match c.content() {
            Content::Placeholder { row, col } => Some((row, col)),
            _ => None,
        })
        .collect()
}

fn placeholder_count(t: &Term) -> usize {
    (0..t.state.lines()).map(|l| pieces(t, l).iter().flatten().count()).sum()
}

fn cursor(t: &Term) -> (usize, usize) {
    (t.state.cursor().row(), t.state.cursor().col())
}

fn replies(t: &mut Term) -> String {
    String::from_utf8(t.state.take_replies()).unwrap()
}

// ---- iTerm2 ----

#[test]
fn a_small_png_is_shown_at_the_cursor_and_the_cursor_ends_after_it() {
    let mut t = term();
    t.advance(b"ab");
    t.advance(&imgcat("", &png(30, 40, false)));
    let images = own(&t);
    assert_eq!(images.len(), 1);
    let image = &images[0];
    assert_eq!((image.width, image.height), (30, 40));
    assert_eq!(image.state, ImageState::Pending);
    // 30 × 40 px in 10 × 20 cells: 3 × 2, from column 2.
    assert_eq!(pieces(&t, 0)[..6], [None, None, Some((0, 0)), Some((0, 1)), Some((0, 2)), None]);
    assert_eq!(pieces(&t, 1)[2..5], [Some((1, 0)), Some((1, 1)), Some((1, 2))]);
    assert_eq!(cursor(&t), (1, 5), "one column past the bottom-right corner");
    let style = t.state.visible_row(0).cell(2).style();
    assert_eq!(term_core::image::placeholder_key(&style), image.key);
    run_jobs(&mut t);
    assert_eq!(own(&t)[0].state, ImageState::Ready);
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    assert_eq!(snap.virtual_placements.len(), 1);
    let v = &snap.virtual_placements[0];
    assert_eq!((v.cols, v.rows, v.fit), (3, 2, Fit::ContainTopLeft));
    assert_eq!((v.image.width, v.image.height), (30, 40));
    check(&t);
}

#[test]
fn a_large_file_streams_and_ends_with_st_too() {
    let mut t = term();
    let data = png(64, 64, true);
    assert!(data.len() > 8 * 1024, "streamed past vte's 4 KB");
    let mut seq = format!("\x1b]1337;File=name={};inline=1:", b64(b"x.png")).into_bytes();
    seq.extend_from_slice(b64(&data).as_bytes());
    seq.extend_from_slice(b"\x1b\\");
    // In odd pieces, as a PTY delivers them.
    for piece in seq.chunks(1000) {
        t.advance(piece);
    }
    run_jobs(&mut t);
    let images = own(&t);
    assert_eq!(images.len(), 1);
    assert_eq!((images[0].width, images[0].height, images[0].state), (64, 64, ImageState::Ready));
    // 64 × 64 px: 7 × 4 cells.
    assert_eq!(placeholder_count(&t), 28);
    t.advance(b"z");
    assert_eq!(t.state.visible_row(3).cell(7).ch(), 'z');
    check(&t);
}

#[test]
fn multipart_files_append_their_parts() {
    let mut t = term();
    let data = png(64, 64, true);
    let enc = b64(&data);
    let mut seq = format!("\x1b]1337;MultipartFile=inline=1;size={}\x07", data.len()).into_bytes();
    // Small parts and one over 4 KB (streamed).
    let (a, rest) = enc.split_at(200);
    let (b, c) = rest.split_at(6000);
    for part in [a, b, c] {
        seq.extend_from_slice(format!("\x1b]1337;FilePart={part}\x07").as_bytes());
    }
    seq.extend_from_slice(b"\x1b]1337;FileEnd\x07");
    t.advance(&seq);
    run_jobs(&mut t);
    let images = own(&t);
    assert_eq!(images.len(), 1);
    assert_eq!((images[0].width, images[0].state), (64, ImageState::Ready));
    check(&t);

    // Another MultipartFile before FileEnd abandons the first.
    let mut t = term();
    t.advance(b"\x1b]1337;MultipartFile=inline=1\x07");
    t.advance(format!("\x1b]1337;FilePart={a}\x07").as_bytes());
    t.advance(b"\x1b]1337;MultipartFile=inline=1\x07");
    t.advance(format!("\x1b]1337;FilePart={}\x07\x1b]1337;FileEnd\x07", b64(&png(10, 10, false))).as_bytes());
    run_jobs(&mut t);
    assert_eq!(own(&t).len(), 1);
    assert_eq!(own(&t)[0].width, 10);
    assert_eq!(t.state.image_pool().used(), own(&t).iter().map(|_| 16384).sum::<usize>());
    check(&t);
}

#[test]
fn downloads_and_what_isnt_an_image_are_dropped_silently() {
    let mut t = term();
    let p = png(10, 10, false);
    for seq in [
        format!("\x1b]1337;File=inline=0:{}\x07", b64(&p)),
        format!("\x1b]1337;File=:{}\x07", b64(&p)),
        format!("\x1b]1337;File=inline=1:{}\x07", b64(b"just some text, not an image")),
        format!("\x1b]1337;File=inline=1:{}\x07", "!!!not base64!!!"),
        "\x1b]1337;File=inline=1\x07".to_string(),
        // Cancelled while streaming.
        format!("\x1b]1337;File=inline=1:{}\x18", b64(&png(64, 64, true))),
    ] {
        t.advance(seq.as_bytes());
    }
    run_jobs(&mut t);
    assert!(own(&t).is_empty());
    assert_eq!(placeholder_count(&t), 0);
    assert_eq!(cursor(&t), (0, 0));
    assert_eq!(replies(&mut t), "");
    assert_eq!(t.state.image_pool().used(), 0);
    check(&t);
}

#[test]
fn sizes_in_cells_pixels_and_percent() {
    let cases = [
        ("width=4", (4, 3)),          // 40 px wide: 40 × 60 px
        ("height=1", (2, 1)),         // 20 px tall: 13 × 20 px
        ("width=40px", (4, 3)),       // 40 × 60
        ("width=50%", (10, 8)),       // 100 px of 200: 100 × 150
        ("width=5;height=1", (5, 1)), // a box; the image fits inside
    ];
    for (args, want) in cases {
        let mut t = term_with(20, 10, 32 << 20);
        t.advance(&imgcat(args, &png(20, 30, false)));
        let mut snap = Snapshot::default();
        run_jobs(&mut t);
        t.state.snapshot(&mut snap, |_| false);
        let v = &snap.virtual_placements[0];
        assert_eq!((v.cols, v.rows), want, "{args}");
        check(&t);
    }
    // Stretched: decoded no larger than the box or the image.
    let mut t = term_with(20, 10, 32 << 20);
    t.advance(&imgcat("width=5;height=1;preserveAspectRatio=0", &png(20, 30, false)));
    run_jobs(&mut t);
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    let v = &snap.virtual_placements[0];
    assert_eq!((v.cols, v.rows, v.fit), (5, 1, Fit::Stretch));
    assert_eq!((v.image.width, v.image.height), (20, 20));
}

#[test]
fn image_pixels_count_as_points() {
    let mut t = term();
    t.state.set_image_scale(2.0);
    t.advance(&imgcat("", &png(30, 40, false)));
    let mut snap = Snapshot::default();
    run_jobs(&mut t);
    t.state.snapshot(&mut snap, |_| false);
    let v = &snap.virtual_placements[0];
    // 60 × 80 device pixels; decoded at its own size.
    assert_eq!((v.cols, v.rows), (6, 4));
    assert_eq!((v.image.width, v.image.height), (30, 40));
}

#[test]
fn a_phone_photo_is_decoded_at_the_size_it_is_drawn() {
    // 4000 × 3000 is 48 MB of pixels, past a 4 MB budget; shown on a
    // 200 px wide screen it is 200 × 150.
    let mut t = term_with(20, 10, 4 << 20);
    t.advance(&imgcat("", &png(4000, 3000, false)));
    run_jobs(&mut t);
    let images = own(&t);
    assert_eq!(images.len(), 1);
    assert_eq!((images[0].width, images[0].height, images[0].state), (200, 150, ImageState::Ready));
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    assert_eq!((snap.virtual_placements[0].cols, snap.virtual_placements[0].rows), (20, 8));
    check(&t);
}

#[test]
fn files_over_the_budget_are_refused() {
    // A quarter of 64 KB holds 16 KB of encoded bytes.
    let mut t = term_with(20, 6, 64 * 1024);
    let big = png(100, 100, true);
    assert!(big.len() > 16 * 1024);
    t.advance(&imgcat("", &big));
    // `size=` says it at once; without it, the bytes run out.
    let seq = format!("\x1b]1337;File=inline=1:{}\x07", b64(&big));
    t.advance(seq.as_bytes());
    run_jobs(&mut t);
    assert!(own(&t).is_empty());
    assert_eq!(placeholder_count(&t), 0);
    assert_eq!(t.state.image_pool().used(), 0);
    check(&t);
}

#[test]
fn do_not_move_cursor_leaves_it_at_the_top_left() {
    let mut t = term();
    t.advance(b"\x1b[3;4H");
    t.advance(&imgcat("doNotMoveCursor=1", &png(30, 40, false)));
    assert_eq!(cursor(&t), (2, 3));
    assert_eq!(pieces(&t, 2)[3], Some((0, 0)));
    // At the bottom: the screen scrolls, the cursor follows the image.
    let mut t = term();
    t.advance(b"\x1b[6;1H");
    t.advance(&imgcat("doNotMoveCursor=1", &png(30, 60, false)));
    assert_eq!(cursor(&t), (3, 0));
    assert_eq!(pieces(&t, 3)[0], Some((0, 0)));
    assert_eq!(pieces(&t, 5)[0], Some((2, 0)));
}

#[test]
fn tall_images_scroll_into_history_and_go_with_it() {
    let mut t = term_with(20, 4, 32 << 20);
    t.advance(&imgcat("height=10", &png(10, 100, false)));
    run_jobs(&mut t);
    assert_eq!(own(&t).len(), 1);
    assert_eq!(cursor(&t).0, 3);
    assert_eq!(pieces(&t, 3)[0], Some((9, 0)));
    // ED 3 then a full reset free history; RIS frees everything.
    t.advance(b"\x1bc");
    run_jobs(&mut t);
    assert!(own(&t).is_empty());
    assert_eq!(t.state.image_pool().used(), 0);
    check(&t);
}

#[test]
fn images_are_off_behind_the_switch() {
    // The process pool is off unless TD_IMAGES=1.
    if term_core::image::images_switch() {
        return;
    }
    let mut t = Term::new(20, 6, 100, ImagePool::process());
    t.advance(&imgcat("", &png(10, 10, false)));
    t.advance(b"\x1bP0;1q\"1;1;4;6#1;2;100;0;0#1!4~\x1b\\");
    assert!(t.state.graphics().images().next().is_none());
    assert_eq!(placeholder_count(&t), 0);
    t.advance(b"\x1b[c\x1b[?1;1;0S");
    assert_eq!(replies(&mut t), "\x1b[?62;22c");
}

// ---- Sixel ----

const RED_4X6: &[u8] = b"\x1bP0;1;0q\"1;1;4;6#1;2;100;0;0#1!4~\x1b\\";

#[test]
fn a_sixel_image_is_ready_at_once_and_the_cursor_goes_below_it() {
    let mut t = term();
    t.advance(b"\x1b[2;3H");
    t.advance(RED_4X6);
    let images = own(&t);
    assert_eq!(images.len(), 1);
    assert_eq!((images[0].width, images[0].height, images[0].state), (4, 6, ImageState::Ready));
    assert_eq!(pieces(&t, 1)[2], Some((0, 0)));
    assert_eq!(placeholder_count(&t), 1);
    assert_eq!(cursor(&t), (2, 2), "the row after, at its left column");
    let mut jobs = Vec::new();
    t.state.take_image_jobs(&mut jobs);
    assert!(jobs.is_empty(), "no decode job");
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    assert_eq!(snap.virtual_placements[0].fit, Fit::Actual);
    check(&t);
}

#[test]
fn mode_8452_leaves_the_cursor_right_of_the_image() {
    let mut t = term();
    t.advance(b"\x1b[?8452h\x1b[2;3H");
    // 25 × 30 px: 3 × 2 cells.
    t.advance(b"\x1bP0;1q\"1;1;25;30#1;2;0;100;0#1!25~-!25~-!25~-!25~-!25~\x1b\\");
    assert_eq!(cursor(&t), (2, 5));
    assert_eq!(pieces(&t, 2)[2..5], [Some((1, 0)), Some((1, 1)), Some((1, 2))]);
    t.advance(b"\x1b[?8452$p");
    assert_eq!(replies(&mut t), "\x1b[?8452;1$y");
}

#[test]
fn display_mode_puts_the_image_at_the_top_left_without_scrolling() {
    let mut t = term_with(20, 3, 32 << 20);
    t.advance(b"\x1b[?80h\x1b[3;5H");
    // 10 × 120 px: 6 rows, clipped to the screen's 3 (60 px).
    let mut seq = b"\x1bP0;1q#1;2;0;0;100".to_vec();
    for _ in 0..20 {
        seq.extend_from_slice(b"#1!10~-");
    }
    seq.extend_from_slice(b"\x1b\\");
    t.advance(&seq);
    assert_eq!(cursor(&t), (2, 4), "the cursor doesn't move");
    assert_eq!(pieces(&t, 0)[0], Some((0, 0)));
    assert_eq!(pieces(&t, 2)[0], Some((2, 0)));
    assert_eq!(own(&t)[0].height, 60, "clipped at the bottom");
    assert_eq!(t.state.scrollback_len(), 0, "no scrolling");
    t.advance(b"\x1b[?80$p\x1b[?80l\x1b[?80$p");
    assert_eq!(replies(&mut t), "\x1b[?80;1$y\x1b[?80;2$y");
}

#[test]
fn sixel_scrolls_the_screen_when_it_reaches_the_bottom() {
    let mut t = term_with(20, 3, 32 << 20);
    t.advance(b"\x1b[3;1H");
    // 4 × 42 px: 3 rows from the last row: two scrolls, then the cursor's
    // row after it a third.
    t.advance(b"\x1bP0;1q\"1;1;4;42#1;2;100;0;0#1!4~-!4~-!4~-!4~-!4~-!4~-!4~\x1b\\");
    assert_eq!(cursor(&t), (2, 0));
    assert_eq!(pieces(&t, 0)[0], Some((1, 0)));
    assert_eq!(pieces(&t, 1)[0], Some((2, 0)));
    assert_eq!(pieces(&t, 2)[0], None);
    check(&t);
}

#[test]
fn the_background_and_p2() {
    // P2 = 1: unset pixels stay transparent; otherwise colour 0.
    for (p2, alpha) in [(b'1', 0u8), (b'0', 255)] {
        let mut t = term();
        let seq = format!("\x1bP0;{}q\"1;1;4;6#0;2;0;0;100#1;2;100;0;0#1!4@\x1b\\", p2 as char);
        t.advance(seq.as_bytes());
        let mut snap = Snapshot::default();
        t.state.snapshot(&mut snap, |_| false);
        let px = snap.virtual_placements[0]
            .image
            .pixels
            .as_any()
            .downcast_ref::<term_core::image::VecBuffer>()
            .unwrap()
            .pixels()
            .pixel(0, 5);
        assert_eq!(px[3], alpha, "P2 = {}", p2 as char);
    }
}

#[test]
fn sixel_parameter_edge_cases() {
    // P1 = 2 (5:1) with no raster attributes: one band is 30 px tall.
    let mut t = term();
    t.advance(b"\x1bP2;1q#1;2;100;0;0#1!4~\x1b\\");
    assert_eq!((own(&t)[0].width, own(&t)[0].height), (4, 30));
    // Raster larger than what is painted keeps its size.
    let mut t = term();
    t.advance(b"\x1bP0;1q\"1;1;30;40#1;2;0;100;100#1!3~\x1b\\");
    assert_eq!((own(&t)[0].width, own(&t)[0].height), (30, 40));
    // Nothing painted and no raster: no image, nothing written.
    let mut t = term();
    t.advance(b"\x1bPq#1;2;100;0;0\x1b\\");
    assert!(own(&t).is_empty());
    assert_eq!(cursor(&t), (0, 0));
    // Abandoned with CAN: dropped, its bytes back.
    let mut t = term();
    t.advance(b"\x1bP0;1q\"1;1;4;6#1;2;100;0;0#1!4~\x18");
    assert!(own(&t).is_empty());
    assert_eq!(t.state.image_pool().used(), 0);
    // Huge repeats are clamped to the screen's width.
    let mut t = term();
    t.advance(b"\x1bP0;1q#1!99999999~\x1b\\");
    assert_eq!(own(&t)[0].width, 200);
    // Parameters past the introducer's three are ignored; an intermediate
    // makes it something else.
    let mut t = term();
    t.advance(b"\x1bP0;1;0;7;8q#1!4~\x1b\\\x1bP$q\"q\x1b\\");
    assert_eq!(own(&t).len(), 1);
    assert_eq!(replies(&mut t), "\x1bP0$r\x1b\\");
    check(&t);
}

#[test]
fn colour_registers_are_private_unless_1070_is_reset() {
    let colour_of = |t: &Term, n: usize| {
        let images = own(t);
        let id = images[n].pool.unwrap();
        let (pixels, _) = t.state.image_pool().pixels(id).unwrap();
        pixels
            .as_any()
            .downcast_ref::<term_core::image::VecBuffer>()
            .unwrap()
            .pixels()
            .pixel(0, 0)
    };
    let define = b"\x1bP0;1q#1;2;0;100;0#1!4~\x1b\\";
    let reuse = b"\x1bP0;1q#1!4~\x1b\\";
    // 1070 set (the default): the second image's #1 is the VT340's.
    let mut t = term();
    t.advance(define);
    t.advance(reuse);
    let (green, vt340) = (colour_of(&t, 0), colour_of(&t, 1));
    assert_ne!(green, vt340);
    // Reset: carried over.
    let mut t = term();
    t.advance(b"\x1b[?1070l");
    t.advance(define);
    t.advance(reuse);
    assert_eq!(colour_of(&t, 0), colour_of(&t, 1));
    assert_eq!(colour_of(&t, 0), green);
    // Per screen: the alternate screen's start from the VT340's.
    t.advance(b"\x1b[?1049h");
    t.advance(reuse);
    assert_eq!(colour_of(&t, 0), vt340);
    t.advance(b"\x1b[?1070$p");
    assert_eq!(replies(&mut t), "\x1b[?1070;2$y");
}

#[test]
fn an_img2sixel_style_image_round_trips() {
    let (w, h) = (97, 61);
    let rgb = sixel_encode::photo(w, h);
    // As img2sixel sends it: `DCS 0 ; 1 ; 0 q`, the data, ST.
    let mut seq = b"\x1bP0;1;0q".to_vec();
    seq.extend_from_slice(&sixel_encode::encode(&rgb, w, h));
    seq.extend_from_slice(b"\x1b\\");
    let mut t = term_with(20, 6, 32 << 20);
    for piece in seq.chunks(777) {
        t.advance(piece);
    }
    let images = own(&t);
    assert_eq!(images.len(), 1);
    assert_eq!((images[0].width, images[0].height), (w, h));
    let id = images[0].pool.unwrap();
    let (pixels, _) = t.state.image_pool().pixels(id).unwrap();
    let px = pixels.as_any().downcast_ref::<term_core::image::VecBuffer>().unwrap().pixels();
    // Every pixel painted, opaque (colours are converted to Display P3).
    for y in 0..h {
        for x in 0..w {
            assert_eq!(px.pixel(x, y)[3], 255);
        }
    }
    // 97 × 61 px: 10 × 4 cells.
    assert_eq!(placeholder_count(&t), 40);
    check(&t);
}

#[test]
fn sixel_images_over_the_budget_are_refused() {
    let mut t = term_with(20, 6, 64 * 1024);
    // 200 × 200 is 160 KB of pixels.
    t.advance(b"\x1bP0;1q\"1;1;200;200#1;2;100;0;0#1!200~\x1b\\");
    assert!(own(&t).is_empty());
    assert_eq!(placeholder_count(&t), 0);
    assert_eq!(t.state.image_pool().used(), 0);
    // Without raster attributes, growth past it is refused too.
    let mut seq = b"\x1bP0;1q#1;2;100;0;0".to_vec();
    for _ in 0..40 {
        seq.extend_from_slice(b"#1!200~-");
    }
    seq.extend_from_slice(b"\x1b\\");
    t.advance(&seq);
    assert!(own(&t).is_empty());
    assert_eq!(t.state.image_pool().used(), 0);
    // A small one still fits after.
    t.advance(RED_4X6);
    assert_eq!(own(&t).len(), 1);
    check(&t);
}

#[test]
fn overwritten_images_are_freed_when_their_style_is_swept() {
    let mut t = term();
    t.state.set_sweep_at(Some(4));
    for _ in 0..6 {
        t.advance(RED_4X6);
    }
    // Clear the screen, then intern new styles to make the table sweep.
    t.advance(b"\x1b[2J\x1b[H");
    for i in 0..8 {
        t.advance(format!("\x1b[38;5;{}mx", i + 1).as_bytes());
    }
    let mut jobs = Vec::new();
    t.state.take_image_jobs(&mut jobs);
    assert!(own(&t).len() < 6, "{} left", own(&t).len());
    // One drawn after stays.
    t.advance(RED_4X6);
    t.state.take_image_jobs(&mut jobs);
    assert!(!own(&t).is_empty());
    check(&t);
}

// ---- What programs ask ----

#[test]
fn da1_reports_sixel_while_images_are_on() {
    let mut t = term();
    t.advance(b"\x1b[c\x1b[0c\x1bZ");
    assert_eq!(replies(&mut t), "\x1b[?62;4;22c".repeat(3));
}

#[test]
fn xtsmgraphics() {
    let mut t = term();
    for (q, want) in [
        ("1;1", "\x1b[?1;0;1024S"),
        ("1;4", "\x1b[?1;0;1024S"),
        ("1;3;256", "\x1b[?1;0;1024S"),
        ("2;1", "\x1b[?2;0;200;120S"),
        ("2;4", "\x1b[?2;0;10000;10000S"),
        ("2;3;100;100", "\x1b[?2;0;200;120S"),
        ("1;9", "\x1b[?1;2;0S"),
        ("3;1", "\x1b[?3;1;0S"),
    ] {
        t.advance(format!("\x1b[?{q}S").as_bytes());
        assert_eq!(replies(&mut t), want, "{q}");
    }
    // Plain `CSI S` still scrolls.
    t.advance(b"top\x1b[S");
    assert_eq!(t.state.visible_row(0).text().trim(), "");
}
