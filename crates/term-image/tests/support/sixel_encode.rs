//! A small Sixel encoder for tests and benches, in the style of img2sixel:
//! raster attributes, RGB colour definitions, per band one pass per colour
//! with `!` repeats.
//!
//! Colours are quantised to a 6 × 6 × 6 cube whose levels (0, 20, … 100
//! percent) are exact in Sixel's percentages, so decoding the output gives
//! back exactly [`quantise`]d pixels.

#![allow(dead_code)]

/// A level 0–5 to 0–255 (exactly `percent(level * 20)`).
fn level(l: u8) -> u8 {
    l * 51
}

/// The cube index of an RGB pixel.
pub fn index(r: u8, g: u8, b: u8) -> u16 {
    let q = |c: u8| (c as u16 + 25) / 51;
    q(r) * 36 + q(g) * 6 + q(b)
}

/// The colour of a cube index, as RGB.
pub fn colour(i: u16) -> [u8; 3] {
    [level((i / 36) as u8), level((i / 6 % 6) as u8), level((i % 6) as u8)]
}

/// RGB pixels to the cube's colours.
pub fn quantise(rgb: &[u8]) -> Vec<u8> {
    rgb.as_chunks::<3>()
        .0
        .iter()
        .flat_map(|p| colour(index(p[0], p[1], p[2])))
        .collect()
}

/// A photo-like test image: smooth gradients, a few shapes, and noise, so
/// runs are short as in chafa's output of a real photo.
pub fn photo(w: u32, h: u32) -> Vec<u8> {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut out = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let noise = (seed % 40) as i32 - 20;
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let dx = fx - 0.6;
            let dy = fy - 0.4;
            let disc = if dx * dx + dy * dy < 0.05 { 90 } else { 0 };
            let r = (fx * 255.0) as i32 + noise + disc;
            let g = (fy * 200.0) as i32 + noise / 2;
            let b = ((1.0 - fx) * fy * 255.0) as i32 - noise + disc / 2;
            out.extend([r, g, b].map(|c| c.clamp(0, 255) as u8));
        }
    }
    out
}

/// Encode RGB pixels as the data of a Sixel string (what follows `DCS q`),
/// at 1:1 aspect with raster attributes.
pub fn encode(rgb: &[u8], w: u32, h: u32) -> Vec<u8> {
    let idx: Vec<u16> = rgb.as_chunks::<3>().0.iter().map(|p| index(p[0], p[1], p[2])).collect();
    let mut out = format!("\"1;1;{w};{h}").into_bytes();
    let mut used = [false; 216];
    for &i in &idx {
        used[i as usize] = true;
    }
    for (i, _) in used.iter().enumerate().filter(|(_, u)| **u) {
        let c = colour(i as u16);
        let p = |v: u8| v as u32 * 100 / 255;
        out.extend(format!("#{i};2;{};{};{}", p(c[0]), p(c[1]), p(c[2])).bytes());
    }
    let mut row = vec![0u8; w as usize];
    for band in 0..h.div_ceil(6) {
        let y0 = band * 6;
        let mut present = [false; 216];
        for y in y0..(y0 + 6).min(h) {
            for x in 0..w {
                present[idx[(y * w + x) as usize] as usize] = true;
            }
        }
        let mut first = true;
        for c in (0..216u16).filter(|&c| present[c as usize]) {
            for (x, bits) in row.iter_mut().enumerate() {
                *bits = 0;
                for dy in 0..6 {
                    let y = y0 + dy;
                    if y < h && idx[(y * w) as usize + x] == c {
                        *bits |= 1 << dy;
                    }
                }
            }
            if !first {
                out.push(b'$');
            }
            first = false;
            out.extend(format!("#{c}").bytes());
            // Trailing empty columns are left out.
            let end = row.iter().rposition(|&b| b != 0).map_or(0, |e| e + 1);
            let mut x = 0;
            while x < end {
                let b = row[x];
                let mut n = 1;
                while x + n < end && row[x + n] == b {
                    n += 1;
                }
                let ch = b'?' + b;
                if n > 3 {
                    out.extend(format!("!{n}").bytes());
                    out.push(ch);
                } else {
                    out.extend(std::iter::repeat_n(ch, n));
                }
                x += n;
            }
        }
        out.push(b'-');
    }
    out
}
