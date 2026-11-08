//! Replays the crashes the `inflate` and `sixel` fuzz targets found
//! (`fuzz/regressions/inflate`, `fuzz/regressions/sixel`) with the same
//! checks as the targets, on stable Rust, so every PR runs them without
//! cargo-fuzz; and the cases behind them, as tables.

use std::fs;
use std::path::{Path, PathBuf};
use term_image::inflate::{Inflate, InflateError, inflate_all};
use term_image::sixel::{Decoder, Limits, Palette};
use term_image::{PixelTarget, VecPixels};

fn inputs(dir: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/regressions").join(dir);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file() && !p.file_name().unwrap().to_string_lossy().starts_with('.'))
        .filter(|p| p.extension().is_none_or(|e| e != "md"))
        .collect();
    files.sort();
    files
}

/// `data` fed in slices of `step` bytes, then finished.
fn inflate_sliced(data: &[u8], step: usize, cap: usize) -> Result<Vec<u8>, InflateError> {
    let mut z = Inflate::new(cap);
    let mut out = Vec::new();
    for slice in data.chunks(step) {
        z.feed(slice, &mut out)?;
    }
    z.finish()?;
    assert!(out.len() <= cap);
    Ok(out)
}

/// fuzz/fuzz_targets/inflate.rs: the first byte is the slice size less one.
fn inflate_target(input: &[u8]) {
    const CAP: usize = 1 << 20;
    let Some((&step, data)) = input.split_first() else { return };
    assert_eq!(inflate_all(data, CAP), inflate_sliced(data, step as usize + 1, CAP));
}

/// fuzz/fuzz_targets/sixel.rs: the first byte picks P1 and P2; the
/// image's size, if there is one.
fn sixel_target(input: &[u8]) -> Option<(u32, u32)> {
    const BUDGET: usize = 1 << 20;
    let (&head, data) = input.split_first()?;
    let (p1, p2) = ((head & 0x0F) as u16, (head >> 4) as u16 % 3);
    let mut d = Decoder::<VecPixels>::new(p1, p2, Palette::vt340(), Limits::default());
    let mut alloc = |w: u32, h: u32| (w as usize * h as usize * 4 <= BUDGET).then(|| VecPixels::new(w, h));
    for slice in data.chunks(97) {
        d.put_slice(slice, &mut alloc);
    }
    let failed = d.failed();
    let img = d.finish(&mut alloc).image?;
    assert!(!failed);
    assert!(img.width <= img.pixels.width() && img.height <= img.pixels.height());
    assert!(img.width > 0 && img.height > 0, "a {} x {} image", img.width, img.height);
    Some((img.width, img.height))
}

fn replay_all(files: &[PathBuf], run: impl Fn(&[u8])) {
    let mut failed = Vec::new();
    for path in files {
        let data = fs::read(path).unwrap();
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&data))).is_err() {
            failed.push(path.display().to_string());
        }
    }
    assert!(failed.is_empty(), "inputs that panicked:\n  {}", failed.join("\n  "));
}

#[test]
fn inflate_regressions() {
    let files = inputs("inflate");
    assert!(!files.is_empty(), "no inputs in fuzz/regressions/inflate");
    replay_all(&files, inflate_target);
}

#[test]
fn sixel_regressions() {
    let files = inputs("sixel");
    assert!(!files.is_empty(), "no inputs in fuzz/regressions/sixel");
    replay_all(&files, |data| {
        sixel_target(data);
    });
}

/// Issue #6: a slice that inflates to more than miniz's 32 KB window
/// left the rest of its output behind, so the last slice ended short of
/// the end of the stream (and the result depended on the slicing).
#[test]
fn inflate_gives_the_same_in_any_slices() {
    let zeros = vec![0u8; 1 << 20];
    let mixed: Vec<u8> = (0..300_000u32)
        .map(|i| if i % 70_000 < 60_000 { 0 } else { (i % 251) as u8 })
        .collect();
    for (name, data) in [("1 MB of zeros", &zeros), ("runs and noise", &mixed)] {
        for level in [1, 6, 9] {
            let z = miniz_oxide::deflate::compress_to_vec_zlib(data, level);
            assert_eq!(inflate_all(&z, data.len()).as_ref(), Ok(data), "{name}, level {level}, whole");
            for step in [1, 7, 256, 1000, 4096] {
                assert_eq!(
                    inflate_sliced(&z, step, data.len()).as_ref(),
                    Ok(data),
                    "{name}, level {level}, slices of {step}"
                );
            }
        }
    }
}

/// The error doesn't depend on the slicing either.
#[test]
fn inflate_errors_are_the_same_in_any_slices() {
    let data = vec![0u8; 1 << 20];
    let z = miniz_oxide::deflate::compress_to_vec_zlib(&data, 9);
    let cut = &z[..z.len() - 4];
    let mut bad = z.clone();
    *bad.last_mut().unwrap() ^= 1;
    for (name, input, cap, want) in [
        ("over the cap", &z[..], data.len() - 1, InflateError::TooLarge),
        ("over the cap, cut off", cut, 1 << 16, InflateError::TooLarge),
        ("no checksum", cut, data.len(), InflateError::Truncated),
        ("bad checksum", &bad[..], data.len(), InflateError::Corrupt),
    ] {
        assert_eq!(inflate_all(input, cap), Err(want), "{name}, whole");
        for step in [1, 7, 256, 4096] {
            assert_eq!(inflate_sliced(input, step, cap), Err(want), "{name}, slices of {step}");
        }
    }
}

/// Issue #8: raster attributes that set a size and then a second set that
/// set none ended in a 0 × 0 image. The last raster attributes before the
/// first pixel give the size; with no size and nothing painted, there is
/// no image.
#[test]
fn sixel_raster_attributes_given_twice() {
    // The first byte: P1 = 0, P2 = 0 (as the fuzz target reads it).
    for (data, want) in [
        (&b"\x00\"1;;8;12\"12;;;"[..], None),
        (b"\x00\"1;1;8;12\"1;1;0;0", None),
        (b"\x00\"1;1;8;12\"1;1;8;0", None),
        (b"\x00\"1;1;8;0", None),
        (b"\x00\"1;1;8;12", Some((8, 12))),
        (b"\x00\"1;1;8;12\"1;1;8;12", Some((8, 12))),
        (b"\x00\"1;1;8;12\"1;1;20;10", Some((20, 10))),
        (b"\x00\"1;1;0;0\"1;1;8;12", Some((8, 12))),
        // Painting grows past the last raster size.
        (b"\x00\"1;1;8;12\"1;1;4;4#1~", Some((4, 6))),
        // After the first pixel, raster attributes are ignored.
        (b"\x00\"1;1;8;12#1~\"1;1;0;0", Some((8, 12))),
    ] {
        assert_eq!(sixel_target(data), want, "{:?}", data.escape_ascii().to_string());
    }
}
