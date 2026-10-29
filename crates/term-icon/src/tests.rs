//! The tests of pins design §9 for term-icon.

use crate::outline::{ink_bounds, parse, tests::template_initial};
use crate::*;

const BRAND: &str = include_str!("../../../docs/brand/README.md");
const TEMPLATE: &str = include_str!("../../../docs/brand/pin-template.svg");

#[test]
fn brand_check_values() {
    for (name, canon, hash, index, initial) in [
        ("Rush", "rush", 20943315, 5, "R"),
        ("htop", "htop", 3212443446, 6, "H"),
        ("Claude", "claude", 3791422743, 3, "C"),
        ("Logs", "logs", 3473062022, 2, "L"),
    ] {
        let p = plan(name);
        assert_eq!(
            (p.canonical.as_str(), p.hash, p.index, p.initial.as_str()),
            (canon, hash, index, initial),
            "{name}"
        );
        assert_eq!((p.top, p.bottom), (PALETTE[index as usize].top, PALETTE[index as usize].bottom));
        assert_eq!(p.generator, "slope-pins-2");
    }
}

/// Roadmap P.3: eight sample names give distinct (initial, hue).
#[test]
fn eight_sample_names_are_distinct() {
    let want = [
        ("Rush", "R", 5),
        ("htop", "H", 6),
        ("Claude", "C", 3),
        ("Logs", "L", 2),
        ("Vim", "V", 9),
        ("Kube", "K", 0),
        ("Psql", "P", 7),
        ("Docker", "D", 1),
    ];
    let mut seen = std::collections::HashSet::new();
    for (name, initial, index) in want {
        let p = plan(name);
        assert_eq!((p.initial.as_str(), p.index), (initial, index), "{name}");
        assert!(seen.insert((p.initial.clone(), p.index)), "{name} repeats");
    }
    let indices: std::collections::HashSet<u8> = want.iter().map(|w| w.2).collect();
    assert_eq!(indices.len(), 8);
}

/// The palette is the brand doc's table, in its order.
#[test]
fn palette_matches_the_brand_doc() {
    let hex = |s: &str| {
        let s = s.trim().trim_matches('`').trim_start_matches('#');
        let v = u32::from_str_radix(s, 16).unwrap();
        [(v >> 16) as u8, (v >> 8) as u8, v as u8]
    };
    let rows: Vec<Vec<&str>> = BRAND
        .lines()
        .filter(|l| l.starts_with("| ") && l.contains("° |") && l.contains(":1"))
        .map(|l| l.split('|').map(str::trim).filter(|c| !c.is_empty()).collect())
        .collect();
    assert_eq!(rows.len(), 10, "the doc's palette table");
    for (i, row) in rows.iter().enumerate() {
        let entry = PALETTE[i];
        assert_eq!(row[0].parse::<usize>().unwrap(), i);
        assert_eq!(row[1], entry.name);
        assert_eq!(row[2].trim_end_matches('°').parse::<u16>().unwrap(), entry.degrees);
        assert_eq!(hex(row[3]), entry.top, "{} top", entry.name);
        assert_eq!(hex(row[4]), entry.bottom, "{} bottom", entry.name);
    }
}

#[test]
fn fnv1a_reference_values() {
    assert_eq!(fnv1a32(b""), 2166136261);
    assert_eq!(fnv1a32(b"a"), 0xe40c292c);
    assert_eq!(fnv1a32(b"foobar"), 0xbf9cf968);
}

#[test]
fn canonical_trims_every_listed_white_space() {
    for &ws in &WHITE_SPACE {
        let name = format!("{ws}{ws}Rush{ws}box{ws}");
        // NFC first: U+2000 and U+2001 are U+2002 and U+2003.
        let inner: String = unicode_normalization::UnicodeNormalization::nfc(ws.to_string().as_str()).collect();
        assert_eq!(canonical(&name), format!("rush{inner}box"), "U+{:04X}", ws as u32);
        assert_eq!(plan(&name).initial, "R");
    }
    // Not White_Space: zero-width space and the BOM stay.
    assert_eq!(canonical("\u{200B}rush\u{FEFF}"), "\u{200B}rush\u{FEFF}");
    assert_eq!(canonical("ssh  prod"), "ssh  prod");
}

#[test]
fn canonical_is_nfc_and_folds_ascii_only() {
    assert_eq!(canonical("Cafe\u{301}"), "caf\u{E9}");
    assert_eq!(canonical("\u{C9}CLAIR"), "\u{C9}clair");
    assert_eq!(canonical("ΣΙΣΥΦΟΣ"), "ΣΙΣΥΦΟΣ");
    assert_eq!(canonical("İSTANBUL"), "İstanbul");
    assert_eq!(plan("Cafe\u{301}").hash, plan("caf\u{E9}").hash);
}

#[test]
fn initials() {
    assert_eq!(initial("  42 things"), "4");
    assert_eq!(initial("🚀 Rocket"), "R");
    assert_eq!(initial("ßeta"), "SS");
    assert_eq!(initial("!!!"), "#");
    assert_eq!(initial(""), "#");
    assert_eq!(initial("   "), "#");
    assert_eq!(initial("e\u{301}lan"), "\u{C9}");
    assert_eq!(initial("ssh prod"), "S");
    assert_eq!(initial("日本"), "日");
    // A keycap's cluster holds a Number, so it is the initial…
    assert_eq!(initial("1\u{FE0F}\u{20E3} one"), "1\u{FE0F}\u{20E3}");
}

/// …but it has no outline, so the drawn initial is its digit.
#[test]
fn keycap_falls_back_to_its_digit() {
    assert!(outline("1\u{FE0F}\u{20E3}").is_err());
    let icon = generate("1\u{FE0F}\u{20E3} one").unwrap();
    assert_eq!(icon.initial, "1");
    assert_eq!(icon.outline, outline("1").unwrap());
}

#[test]
fn no_letter_draws_a_hash() {
    let icon = generate("!!!").unwrap();
    assert_eq!(icon.initial, "#");
    assert!(icon.outline.starts_with('M'));
}

/// Pins design §9: `outline("R")` matches the template's R (ink x
/// 367.925–656.075, y 265–625) within 0.01.
#[test]
fn outline_of_r_matches_the_template() {
    let ours = parse(&outline("R").unwrap()).unwrap();
    let (x0, y0, x1, y1) = ink_bounds(&ours).unwrap();
    for (got, want) in [(x0, 367.925), (y0, 265.0), (x1, 656.075), (y1, 625.0)] {
        assert!((got - want).abs() <= 0.01, "{got} vs {want}");
    }
    // Point for point too: the template was made the same way.
    let theirs = parse(template_initial(TEMPLATE)).unwrap();
    assert_eq!(ours.len(), theirs.len());
    for (a, b) in ours.iter().zip(&theirs) {
        let (a, b) = (format!("{a:?}"), format!("{b:?}"));
        let nums = |s: &str| -> Vec<f64> {
            s.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                .filter_map(|t| t.parse().ok())
                .collect()
        };
        assert_eq!(a.split('(').next(), b.split('(').next());
        for (x, y) in nums(&a).iter().zip(nums(&b)) {
            assert!((x - y).abs() <= 0.01, "{a} vs {b}");
        }
    }
}

#[test]
fn every_initial_fits_the_box() {
    for name in ["Rush", "htop", "ssh prod", "Wq", "ßeta", "日本", "Ωmega", "jq", "1"] {
        let icon = generate(name).unwrap();
        let (x0, y0, x1, y1) = ink_bounds(&parse(&icon.outline).unwrap()).unwrap();
        let (w, h) = (x1 - x0, y1 - y0);
        assert!(w <= 400.01 && h <= 360.01, "{name}: {w} × {h}");
        assert!((w - 400.0).abs() < 0.01 || (h - 360.0).abs() < 0.01, "{name}: fitted {w} × {h}");
        assert!(
            ((x0 + x1) / 2.0 - 512.0).abs() < 0.01 && ((y0 + y1) / 2.0 - 445.0).abs() < 0.01,
            "{name}"
        );
        assert!(y1 <= 625.01, "{name}: the initial ends at y625");
    }
}

/// The same name gives the same icon, record and pixels, every time.
#[test]
fn deterministic() {
    for name in ["Rush", "lazygit", "ssh prod"] {
        let a = generate(name).unwrap();
        assert_eq!(a, generate(name).unwrap());
        for px in [16, 128, 1024] {
            assert_eq!(render_rgba(&a, px).unwrap(), render_rgba(&a, px).unwrap(), "{name} at {px}");
        }
        assert_eq!(render_icns(&a).unwrap(), render_icns(&a).unwrap(), "{name}: .icns bytes");
    }
}

/// The kept record is all drawing needs: an icon whose outline was
/// written by an older run draws the same.
#[test]
fn draws_from_the_kept_record() {
    let mut icon = plan("Rush");
    icon.outline = template_initial(TEMPLATE).to_owned();
    let ours = generate("Rush").unwrap();
    let (a, b) = (render_rgba(&icon, 128).unwrap(), render_rgba(&ours, 128).unwrap());
    let worst = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
    assert!(worst <= 1, "{worst}");
    // A damaged record is refused, not drawn.
    icon.outline = "M1 2 <svg>".to_owned();
    assert!(matches!(render_rgba(&icon, 16), Err(IconError::BadOutline(_))));
}

fn px(img: &[u8], size: usize, x: usize, y: usize) -> [u8; 4] {
    let i = (y * size + x) * 4;
    [img[i], img[i + 1], img[i + 2], img[i + 3]]
}

fn mix(a: [u8; 3], b: [u8; 3], t: f64) -> [f64; 3] {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| a[i] as f64 + (b[i] as f64 - a[i] as f64) * t)
}

/// The light overlay at a point: white, 6% at (.25, 0) of the inner
/// rect, fading to 0 at its width.
fn lit(c: [f64; 3], x: f64, y: f64) -> [f64; 3] {
    let (u, v) = ((x - 103.0) / 818.0, (y - 103.0) / 818.0);
    let d = ((u - 0.25).powi(2) + v * v).sqrt();
    let a = 0.06 * (1.0 - d).max(0.0);
    c.map(|c| c * (1.0 - a) + 255.0 * a)
}

fn assert_near(got: [u8; 4], want: [f64; 3], what: &str) {
    assert_eq!(got[3], 255, "{what}: opaque");
    for i in 0..3 {
        assert!((got[i] as f64 - want[i]).abs() <= 2.0, "{what}: {got:?} vs {want:?}");
    }
}

/// Colours at interior points are what the template's gradients give.
#[test]
fn colours_follow_the_gradients() {
    for name in ["Rush", "htop", "Claude", "Logs"] {
        let icon = generate(name).unwrap();
        let img = render_rgba(&icon, 1024).unwrap();
        // Body, away from the foreground, its depth plane and the rim.
        for (x, y) in [(180, 500), (850, 300), (500, 180), (200, 880)] {
            let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
            let t = (0.7 * (cx - 100.0) / 824.0 + (cy - 100.0) / 824.0) / 1.49;
            assert_near(
                px(&img, 1024, x, y),
                lit(mix(icon.top, icon.bottom, t), cx, cy),
                &format!("{name} body ({x}, {y})"),
            );
        }
        // The Slope's level part, its box (326, 702)–(704, 822).
        for (x, y) in [(600, 822), (500, 810)] {
            let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
            let t = (0.3 * (cx - 326.0) / 378.0 + (cy - 702.0) / 120.0) / 1.09;
            assert_near(
                px(&img, 1024, x, y),
                lit(mix([255; 3], [185, 229, 204], t), cx, cy),
                &format!("{name} slope"),
            );
        }
        // Transparent outside the body.
        for (x, y) in [(0, 0), (1023, 0), (0, 1023), (1023, 1023), (110, 110), (50, 512)] {
            assert_eq!(px(&img, 1024, x, y), [0; 4], "{name} ({x}, {y})");
        }
    }
    // Inside Rush's R: the stem, x 367.9–443.3.
    let icon = generate("Rush").unwrap();
    let img = render_rgba(&icon, 1024).unwrap();
    let (x0, y0, x1, y1) = ink_bounds(&parse(&icon.outline).unwrap()).unwrap();
    for (x, y) in [(405, 300), (405, 445), (405, 600)] {
        let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
        let t = (0.3 * (cx - x0) / (x1 - x0) + (cy - y0) / (y1 - y0)) / 1.09;
        assert_near(px(&img, 1024, x, y), lit(mix([255; 3], [185, 229, 204], t), cx, cy), "Rush's R");
    }
}

/// The initial never touches the Slope: a gap of body colour between
/// them, so they stay separate at 16 px.
#[test]
fn gap_between_initial_and_slope() {
    let icon = generate("Rush").unwrap();
    let img = render_rgba(&icon, 1024).unwrap();
    // Between the initial's bottom (625) plus its depth plane (633) and the
    // Slope's nearest edge (670): nothing lighter than the body.
    for y in 640..665 {
        for x in (300..720).step_by(7) {
            let p = px(&img, 1024, x, y);
            assert!(p[0] < 100 && p[1] < 120, "({x}, {y}): {p:?}");
        }
    }
}

/// The `.icns` holds every size, as `iconutil` reads it.
#[test]
fn icns_round_trips_through_iconutil() {
    let icns = render_icns(&generate("Rush").unwrap()).unwrap();
    assert_eq!(&icns[..4], b"icns");
    assert_eq!(u32::from_be_bytes(icns[4..8].try_into().unwrap()) as usize, icns.len());
    // ImageIO's entries (macOS 27): the 16 and 32 px ones in the old
    // packed form with a mask, the rest as PNG.
    let mut kinds = Vec::new();
    let mut i = 8;
    while i < icns.len() {
        kinds.push(String::from_utf8_lossy(&icns[i..i + 4]).into_owned());
        i += u32::from_be_bytes(icns[i + 4..i + 8].try_into().unwrap()) as usize;
    }
    kinds.sort();
    let mut want = [
        "TOC ", "is32", "s8mk", "il32", "l8mk", "ic07", "ic08", "ic09", "ic10", "ic11", "ic12", "ic13", "ic14",
    ]
    .map(String::from);
    want.sort();
    assert_eq!(kinds, want);
    let dir = std::env::temp_dir().join(format!("term-icon-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("icon.icns");
    std::fs::write(&file, &icns).unwrap();
    let set = dir.join("icon.iconset");
    let ok = std::process::Command::new("iconutil")
        .arg("-c")
        .arg("iconset")
        .arg(&file)
        .arg("-o")
        .arg(&set)
        .status()
        .unwrap();
    assert!(ok.success());
    let png_size = |name: &str| {
        let b = std::fs::read(set.join(name)).unwrap_or_else(|_| panic!("{name} missing"));
        assert_eq!(&b[1..4], b"PNG", "{name}");
        (
            u32::from_be_bytes(b[16..20].try_into().unwrap()),
            u32::from_be_bytes(b[20..24].try_into().unwrap()),
        )
    };
    for (pt, scale) in [
        (16, 1),
        (16, 2),
        (32, 1),
        (32, 2),
        (128, 1),
        (128, 2),
        (256, 1),
        (256, 2),
        (512, 1),
        (512, 2),
    ] {
        let name = if scale == 1 {
            format!("icon_{pt}x{pt}.png")
        } else {
            format!("icon_{pt}x{pt}@2x.png")
        };
        let n = (pt * scale) as u32;
        assert_eq!(png_size(&name), (n, n), "{name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decodes an image with ImageIO into RGBA, premultiplied, sRGB, top row
/// first.
fn decode(bytes: &[u8]) -> (usize, usize, Vec<u8>) {
    use objc2_core_foundation::{CFData, CGPoint, CGRect, CGSize};
    use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo, kCGColorSpaceSRGB};
    use objc2_image_io::CGImageSource;
    unsafe {
        let source = CGImageSource::with_data(&CFData::from_bytes(bytes), None).unwrap();
        let image = source.image_at_index(0, None).unwrap();
        let (w, h) = (CGImage::width(Some(&image)), CGImage::height(Some(&image)));
        let srgb = CGColorSpace::with_name(Some(kCGColorSpaceSRGB)).unwrap();
        let mut data = vec![0u8; w * h * 4];
        let ctx = CGBitmapContextCreate(
            data.as_mut_ptr() as *mut std::ffi::c_void,
            w,
            h,
            8,
            w * 4,
            Some(&srgb),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
        .unwrap();
        let r = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: w as f64,
                height: h as f64,
            },
        };
        CGContext::draw_image(Some(&ctx), r, Some(&image));
        drop(ctx);
        (w, h, data)
    }
}

/// Our 128, 32 and 16 px renders of the brand doc's four pins, laid on
/// the Dock colour, against `docs/brand/pins.png` (drawn with AppKit from
/// the same artwork), light and dark. Measured: mean difference 0.35–0.46
/// per channel, worst single pixel 8–14 of 255 (antialiased edges).
#[test]
fn matches_the_brand_sheet() {
    let (sw, _, sheet) = decode(include_bytes!("../../../docs/brand/pins.png"));
    // Where the sheet puts each 1024-grid icon's top-left corner.
    for (size, top) in [(128usize, 100usize), (32, 308), (16, 408)] {
        for (col, name) in ["Rush", "htop", "Claude", "Logs"].iter().enumerate() {
            let ours = render_rgba(&generate(name).unwrap(), size).unwrap();
            for half in [0, 720] {
                let (left, what) = (
                    40 + half + col * 164,
                    format!("{name} at {size} px, {}", if half == 0 { "light" } else { "dark" }),
                );
                // The icon's corner is transparent: the sheet there is the Dock.
                let dock = px(&sheet, sw, left, top);
                let (mut sum, mut worst) = (0.0, 0.0f64);
                for y in 0..size {
                    for x in 0..size {
                        let (o, s) = (px(&ours, size, x, y), px(&sheet, sw, left + x, top + y));
                        let a = o[3] as f64 / 255.0;
                        for i in 0..3 {
                            let d = (o[i] as f64 + dock[i] as f64 * (1.0 - a) - s[i] as f64).abs();
                            sum += d;
                            worst = worst.max(d);
                        }
                    }
                }
                let mean = sum / (size * size * 3) as f64;
                assert!(mean <= 1.0, "{what}: mean difference {mean:.2}");
                assert!(worst <= 24.0, "{what}: worst pixel {worst}");
            }
        }
    }
}
