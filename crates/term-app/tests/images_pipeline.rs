//! images through a real session with images on (`TD_IMAGES=1`, set
//! before anything asks; this binary has one test, running its scenarios
//! in turn, so nothing asks first): kitty (a PNG in the stream and another
//! by file path, the file's header read with the terminal's lock
//! released), iTerm2 (`imgcat`'s OSC 1337 `File=`) and Sixel. Each ends in
//! the snapshot as Metal pixels, decoded on the app's queue. No window.
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

/// A real session whose program prints `stream` (in `dir`) and sleeps; the
/// snapshot once `want` placements are in it.
fn show(dir: &std::path::Path, stream: &[u8], want: usize) -> (Session, Snapshot) {
    let out = dir.join(format!("out-{}.bin", stream.len()));
    std::fs::write(&out, stream).unwrap();
    let cmd = Command {
        argv: vec!["/bin/sh".into(), "-c".into(), format!("cat '{}'; sleep 30", out.display())],
        cwd: Some(dir.to_str().unwrap().into()),
        env: vec![("HOME".into(), dir.to_str().unwrap().into())],
        shell: Some("/bin/sh".into()),
        direct: false,
        integration: None,
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
            if snap.placements.len() + snap.virtual_placements.len() == want {
                break snap;
            }
        }
        assert!(Instant::now() < until, "the images never showed");
        std::thread::sleep(Duration::from_millis(20));
    };
    (session, snap)
}

fn sizes(snap: &Snapshot) -> Vec<(u32, u32)> {
    let images = snap
        .placements
        .iter()
        .map(|p| &p.image)
        .chain(snap.virtual_placements.iter().map(|v| &v.image));
    let mut sizes: Vec<_> = images
        .map(|image| {
            let m = image.pixels.as_any().downcast_ref::<MetalPixels>().expect("Metal pixels");
            (term_image::PixelTarget::width(m), term_image::PixelTarget::height(m))
        })
        .collect();
    sizes.sort();
    sizes
}

#[test]
fn images_from_a_program_end_as_metal_pixels() {
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

    kitty(&dir);
    iterm2(&dir);
    sixel(&dir);
    let _ = std::fs::remove_dir_all(&dir);
}

fn kitty(dir: &std::path::Path) {
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
    let (session, snap) = show(dir, &stream, 2);
    assert_eq!(sizes(&snap), [(8, 40), (16, 16)]);
    {
        let t = session.lock_term();
        for id in [1, 2] {
            assert!(t.state.graphics().image(ImageKey::Kitty(id)).is_some());
        }
        assert!(t.state.image_pool().used() > 0);
    }
    assert!(file.exists(), "t=f leaves the file");
    session.shut_down();
}

/// What `imgcat` sends: OSC 1337 `File=` with the PNG inline.
fn iterm2(dir: &std::path::Path) {
    let data = png(12, 20);
    let mut stream = format!(
        "\x1b]1337;File=inline=1;size={};name={}:{}\x07",
        data.len(),
        b64(b"cat.png"),
        b64(&data)
    )
    .into_bytes();
    stream.extend_from_slice(b"done\r\n");
    let (session, snap) = show(dir, &stream, 1);
    assert_eq!(sizes(&snap), [(12, 20)]);
    assert!(session.lock_term().state.image_pool().used() > 0);
    session.shut_down();
}

/// A 4 x 6 red Sixel image.
fn sixel(dir: &std::path::Path) {
    let mut stream = b"\x1bP0;1;0q\"1;1;4;6#1;2;100;0;0#1!4~\x1b\\".to_vec();
    stream.extend_from_slice(b"done\r\n");
    let (session, snap) = show(dir, &stream, 1);
    assert_eq!(sizes(&snap), [(4, 6)]);
    assert!(session.lock_term().state.image_pool().used() > 0);
    session.shut_down();
}
