use term_font::{Fonts, Style};

#[test]
fn metrics_and_glyphs() {
    let f = Fonts::new("Menlo", 26.0);
    let m = f.metrics;
    assert!(m.cell_width > 10 && m.cell_width < 25, "{m:?}");
    assert!(m.cell_height > m.cell_width);
    let a = f.rasterize('A', Style::Regular).unwrap();
    assert!(!a.color && a.data.iter().any(|&b| b > 200));
    assert!(f.rasterize(' ', Style::Regular).is_none());
    let e = f.rasterize('😀', Style::Regular).unwrap();
    assert!(e.color, "emoji should come from a colour font");
    assert!(f.rasterize('漢', Style::Bold).is_some(), "CJK via fallback");
}
