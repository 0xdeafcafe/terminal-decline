//! Writes sample pin icons as PNGs, for looking at:
//!
//! ```sh
//! cargo run -p term-icon --example samples -- docs/brand/samples-generated
//! ```
//!
//! Each name gets `<name>.png` at 512 px, and `<name>-32.png` and
//! `<name>-16.png` drawn at those Dock sizes (not scaled down).

use std::path::PathBuf;

const NAMES: [&str; 12] = [
    "Rush", "Claude", "htop", "lazygit", "btop", "nvim", "ssh prod", "yazi", "Logs", "Vim", "Kube", "Docker",
];

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "docs/brand/samples-generated".to_owned()));
    std::fs::create_dir_all(&dir).expect("create the output folder");
    for name in NAMES {
        let start = std::time::Instant::now();
        let icon = term_icon::generate(name).expect("generate");
        let icns = term_icon::render_icns(&icon).expect("icns");
        let took = start.elapsed();
        let file = name.to_lowercase().replace(' ', "-");
        for (px, suffix) in [(512, ""), (32, "-32"), (16, "-16")] {
            let png = term_icon::render_png(&icon, px).expect("render");
            std::fs::write(dir.join(format!("{file}{suffix}.png")), png).expect("write");
        }
        let hue = term_icon::PALETTE[icon.index as usize];
        println!(
            "{name:10} {:>10} index {} {:10} initial {}  (.icns {} KB in {} ms)",
            icon.hash,
            icon.index,
            hue.name,
            icon.initial,
            icns.len() / 1024,
            took.as_millis()
        );
    }
}
