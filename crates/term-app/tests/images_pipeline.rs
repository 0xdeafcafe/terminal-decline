//! kitty images through a real session with images on (`TD_IMAGES=1`, set
//! before anything asks; this binary has one test so nothing asks first):
//! a program prints a PNG in the stream and another by file path, and both
//! end in the snapshot as Metal pixels, decoded on the app's queue, the
//! file's header read with the terminal's lock released. No window.
#![cfg(target_os = "macos")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use term_core::image::ImageKey;
use term_core::{ImagePool, Snapshot, Term};
use term_pty::{Command, Size};
use term_render::MetalPixels;
use terminal_decline_app::session::{Hooks, Session};

const DEADLINE: Duration = Duration::from_secs(60);
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

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    let mut wr = e.write_header().unwrap();
    wr.write_image_data(&vec![0xFF; (w * h * 4) as usize]).unwrap();
    wr.finish().unwrap();
    out
}

#[test]
fn kitty_images_from_a_program_end_as_metal_pixels() {
    // SAFETY: the only test in this binary, before any thread starts.
    unsafe { std::env::set_var("TD_IMAGES", "1") };
    assert!(ImagePool::process().enabled());
    if objc2_metal::MTLCreateSystemDefaultDevice().is_none() {
        assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
        eprintln!("skipping: no Metal device");
        return;
    }
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("td-images-pipeline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("by-path.png");
    std::fs::write(&file, png(8, 40)).unwrap();
    let mut stream = format!("\x1b_Ga=T,f=100,i=1,q=2;{}\x1b\\", b64(&png(16, 16))).into_bytes();
    stream.extend(
        format!(
            "\x1b_Ga=T,f=100,t=f,i=2,q=2;{}\x1b\\done\r\n",
            b64(file.to_str().unwrap().as_bytes())
        )
        .into_bytes(),
    );
    let out = dir.join("out.bin");
    std::fs::write(&out, &stream).unwrap();

    let cmd = Command {
        argv: vec!["/bin/sh".into(), "-c".into(), format!("cat '{}'; sleep 30", out.display())],
        cwd: Some(dir.to_str().unwrap().into()),
        env: vec![("HOME".into(), dir.to_str().unwrap().into())],
        shell: Some("/bin/sh".into()),
        direct: false,
    };
    let woken = Arc::new(AtomicBool::new(false));
    let w = woken.clone();
    let hooks = Hooks::new(Arc::new(move || w.store(true, Ordering::Release)), Box::new(|_| {}));
    let mut term = Term::new(40, 10, 100, ImagePool::process());
    term.state.set_cell_pixels((8, 16));
    let size = Size {
        cols: 40,
        lines: 10,
        cell_width: 8,
        cell_height: 16,
    };
    let session = Session::spawn(&cmd, size, term, hooks).expect("spawn");

    let until = Instant::now() + DEADLINE;
    let snap = loop {
        {
            let mut t = session.lock_term();
            let mut snap = Snapshot::default();
            t.state.snapshot(&mut snap, |_| false);
            session.take_image_jobs(&mut t);
            if snap.placements.len() == 2 {
                break snap;
            }
        }
        assert!(Instant::now() < until, "the images never showed");
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut sizes: Vec<_> = snap
        .placements
        .iter()
        .map(|p| {
            let m = p.image.pixels.as_any().downcast_ref::<MetalPixels>().expect("Metal pixels");
            (term_image::PixelTarget::width(m), term_image::PixelTarget::height(m))
        })
        .collect();
    sizes.sort();
    assert_eq!(sizes, [(8, 40), (16, 16)]);
    {
        let t = session.lock_term();
        for id in [1, 2] {
            assert!(t.state.graphics().image(ImageKey::Kitty(id)).is_some());
        }
        assert!(t.state.image_pool().used() > 0);
    }
    assert!(file.exists(), "t=f leaves the file");
    session.shut_down();
    let _ = std::fs::remove_dir_all(&dir);
}
