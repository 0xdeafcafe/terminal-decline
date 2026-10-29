//! Sixel golden images: each string below is decoded and compared pixel by
//! pixel with `tests/golden/<name>.png` (straight RGBA). Run with
//! `BLESS=1` to write the PNGs after checking a change by eye.
//!
//! Plus a round trip through an img2sixel-style encoder
//! (`support/sixel_encode.rs`), whose expected pixels are known exactly.

use std::path::PathBuf;
use term_image::VecPixels;
use term_image::sixel::{Decoder, Limits, Palette};

#[path = "support/sixel_encode.rs"]
mod sixel_encode;

/// Whole strings, `ESC P … q` to `ESC \`, as a program sends them.
const CASES: &[(&str, &[u8])] = &[
    ("one_colour", b"\x1bP0;1;0q\"1;1;4;6#1;2;100;0;0#1!4~\x1b\\"),
    // A stripe of each bit, then repeats of every length form.
    ("repeats", b"\x1bP0;1q\"1;1#1;2;0;0;100#1?@ABCDEFGHIJKLMNOPQRSTUVWXYZ!3_!10~\x1b\\"),
    // Three colours over one band with `$`, then a second band with `-`.
    ("bands", b"\x1bP;1q\"1;1;12;12#1;2;100;0;0#2;2;0;100;0#3;2;0;0;100#1!12F$#2!12w-#3!6~$!6?#1!6~\x1b\\"),
    // HLS: hue 0 is blue, 120 red, 240 green; then a grey and white.
    ("hls", b"\x1bP0;1q\"1;1#1;1;0;50;100#2;1;120;50;100#3;1;240;50;100#4;1;77;50;0#5;1;0;100;0#1!4~#2!4~#3!4~#4!4~#5!4~\x1b\\"),
    // RGB percentages, including over 100.
    ("rgb", b"\x1bP0;1q\"1;1#1;2;100;50;0#2;2;20;40;60#3;2;250;100;0#1!4~#2!4~#3!4~\x1b\\"),
    // P2 = 1 leaves unset pixels transparent; P2 = 0 paints them colour 0.
    ("transparent", b"\x1bP0;1q\"1;1;6;6#1;2;100;100;0#1!6U\x1b\\"),
    ("background", b"\x1bP0;0q\"1;1;6;6#0;2;0;0;40#1;2;100;100;0#1!6U\x1b\\"),
    // A raster larger than the painted area keeps its size.
    ("raster_larger", b"\x1bP0;1q\"1;1;10;10#1;2;0;100;100#1!3~\x1b\\"),
    // Pad = 0 is taken as 1: aspect 2:1.
    ("pad_zero", b"\x1bP0;1q\"2;0#1;2;100;0;100#1!4F\x1b\\"),
    // P1 omitted and no raster attributes: 2:1.
    ("aspect_default", b"\x1bPq#1;2;100;60;0#1!4F-!4~\x1b\\"),
    // No raster attributes and wider and taller than the first allocation.
    ("growth", b"\x1bP0;1q#1;2;100;0;0#2;2;0;0;100#1!90~-#2!90~-#1!45~#2!45~-#1!90~-#2!90~-#1!90~-#2!90~-#1!90~-#2!90~-#1!90~-#2!90~-#1!90~-#2!90~\x1b\\"),
    // Redefining register 1 after painting with it leaves those pixels.
    ("redefine", b"\x1bP0;1q\"1;1#1;2;100;0;0#1!4~#1;2;0;100;0#1!4~\x1b\\"),
    // The VT340 palette, registers 0-15 without defining any.
    ("vt340", b"\x1bP0;1q\"1;1#0!2~#1!2~#2!2~#3!2~#4!2~#5!2~#6!2~#7!2~#8!2~#9!2~#10!2~#11!2~#12!2~#13!2~#14!2~#15!2~\x1b\\"),
];

/// The introducer's numbers and the data after `q`.
fn split(seq: &[u8]) -> (u16, u16, &[u8]) {
    let body = seq.strip_prefix(b"\x1bP").unwrap().strip_suffix(b"\x1b\\").unwrap();
    let q = body.iter().position(|&b| b == b'q').unwrap();
    let mut params = body[..q]
        .split(|&b| b == b';')
        .map(|p| std::str::from_utf8(p).unwrap().parse::<u16>().unwrap_or(0));
    let p1 = params.next().unwrap_or(0);
    let p2 = params.next().unwrap_or(0);
    (p1, p2, &body[q + 1..])
}

fn decode(seq: &[u8]) -> (u32, u32, Vec<u8>) {
    let (p1, p2, data) = split(seq);
    let mut d = Decoder::<VecPixels>::new(p1, p2, Palette::vt340(), Limits::default());
    let mut alloc = |w, h| Some(VecPixels::new(w, h));
    d.put_slice(data, &mut alloc);
    let s = d.finish(&mut alloc).image.expect("an image");
    assert_eq!((s.pixels.width(), s.pixels.height()), (s.width, s.height), "cropped exactly");
    (s.width, s.height, s.pixels.to_rgba())
}

use term_image::PixelTarget;

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.png"))
}

fn read_png(path: &PathBuf) -> (u32, u32, Vec<u8>) {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e} (run with BLESS=1)", path.display()));
    let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

fn write_png(path: &PathBuf, w: u32, h: u32, rgba: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgba).unwrap();
}

#[test]
fn golden_images() {
    let bless = std::env::var_os("BLESS").is_some();
    let mut failed = Vec::new();
    for (name, seq) in CASES {
        let (w, h, rgba) = decode(seq);
        let path = golden(name);
        if bless {
            write_png(&path, w, h, &rgba);
            continue;
        }
        let (ew, eh, expected) = read_png(&path);
        if (w, h) != (ew, eh) {
            failed.push(format!("{name}: {w}x{h}, expected {ew}x{eh}"));
        } else if let Some(i) = rgba.chunks(4).zip(expected.chunks(4)).position(|(a, b)| a != b) {
            let (x, y) = (i as u32 % w, i as u32 / w);
            failed.push(format!(
                "{name}: pixel ({x}, {y}) is {:?}, expected {:?}",
                &rgba[i * 4..i * 4 + 4],
                &expected[i * 4..i * 4 + 4]
            ));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

#[test]
fn golden_checks_by_hand() {
    // A few pixels pinned in code too, so blessing can't hide a mistake.
    let px = |name: &str, x: u32, y: u32| {
        let (_, seq) = CASES.iter().find(|(n, _)| *n == name).unwrap();
        let (w, _, rgba) = decode(seq);
        let i = ((y * w + x) * 4) as usize;
        <[u8; 4]>::try_from(&rgba[i..i + 4]).unwrap()
    };
    assert_eq!(px("hls", 0, 0), [0, 0, 255, 255], "hue 0 is blue");
    assert_eq!(px("hls", 4, 0), [255, 0, 0, 255], "hue 120 is red");
    assert_eq!(px("hls", 8, 0), [0, 255, 0, 255], "hue 240 is green");
    assert_eq!(px("transparent", 0, 0), [0, 0, 0, 0]);
    assert_eq!(px("background", 0, 0), [0, 0, 102, 255]);
    assert_eq!(px("redefine", 0, 0), [255, 0, 0, 255]);
    assert_eq!(px("redefine", 4, 0), [0, 255, 0, 255]);
    let (_, seq) = CASES.iter().find(|(n, _)| *n == "aspect_default").unwrap();
    assert_eq!(decode(seq).1, 24, "two bands at 2:1");
    let (_, seq) = CASES.iter().find(|(n, _)| *n == "growth").unwrap();
    assert_eq!(decode(seq).0, 90);
    assert_eq!(decode(seq).1, 13 * 12, "13 bands at 2:1");
}

#[test]
fn encoder_round_trip_is_exact() {
    for (w, h) in [(1, 1), (7, 5), (200, 150), (333, 77)] {
        let photo = sixel_encode::photo(w, h);
        let data = sixel_encode::encode(&photo, w, h);
        let mut seq = b"\x1bP0;1;0q".to_vec();
        seq.extend_from_slice(&data);
        seq.extend_from_slice(b"\x1b\\");
        let (dw, dh, rgba) = decode(&seq);
        assert_eq!((dw, dh), (w, h));
        let want = sixel_encode::quantise(&photo);
        for (i, (got, want)) in rgba.chunks(4).zip(want.chunks(3)).enumerate() {
            assert_eq!(&got[..3], want, "{w}x{h} pixel {i}");
            assert_eq!(got[3], 255);
        }
    }
}

#[test]
fn fast_and_slow_paths_agree() {
    // The encoder's single sixels take the fast path; written as `!1c`
    // they go through the other one.
    let (w, h) = (123, 47);
    let data = sixel_encode::encode(&sixel_encode::photo(w, h), w, h);
    let mut slow = Vec::new();
    let mut repeat = false;
    for &b in &data {
        if (b'?'..=b'~').contains(&b) && !repeat {
            slow.extend_from_slice(b"!1");
        }
        repeat = b == b'!' || (repeat && b.is_ascii_digit());
        slow.push(b);
    }
    let wrap = |d: &[u8]| [&b"\x1bP0;1q"[..], d, b"\x1b\\"].concat();
    assert_eq!(decode(&wrap(&data)), decode(&wrap(&slow)));
}
