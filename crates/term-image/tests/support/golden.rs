//! Golden images: decoded pixels compared with `tests/golden/<name>.png`
//! (straight RGBA). `BLESS=1` writes the PNGs instead, after checking a
//! change by eye.

use std::path::PathBuf;

fn path(name: &str) -> PathBuf {
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

/// Compares (or with `BLESS=1`, writes) each `(name, width, height,
/// rgba)`, and fails listing every image that differs.
pub fn check(images: impl IntoIterator<Item = (String, u32, u32, Vec<u8>)>) {
    let bless = std::env::var_os("BLESS").is_some();
    let mut failed = Vec::new();
    for (name, w, h, rgba) in images {
        let path = path(&name);
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
