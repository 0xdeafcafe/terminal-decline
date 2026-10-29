//! The host, the queue and a terminal together: a PNG sent through the
//! session's side of the queue ends as Metal pixels in the pool, ready to
//! draw; a decoder that panics fails its image only; jobs of a closed
//! session don't run; the budget follows the largest window.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use term_core::image::{
    Encoded, Image, ImageError, ImageHost, ImageJob, ImageKey, ImageState, JobId, MediumRequest, OpenMedium, PixelBuffer, VecBuffer,
};
use term_core::{ImagePool, Term};
use term_image::PixelTarget;
use term_render::{MetalPixels, PixelAlloc};

use super::host::AppHost;
use super::queue::DecodeQueue;
use super::{SessionImages, window_size};

const WAIT: Duration = Duration::from_secs(10);

/// A PNG of `w` × `h` RGBA pixels.
fn png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(rgba).unwrap();
    w.finish().unwrap();
    out
}

struct Setup {
    term: Arc<Mutex<Term>>,
    pool: Arc<ImagePool>,
    images: Arc<SessionImages>,
    wakes: Arc<AtomicUsize>,
}

fn setup(host: Arc<dyn ImageHost>, pool: Arc<ImagePool>) -> Setup {
    let term = Arc::new(Mutex::new(Term::new(20, 5, 100, pool.clone())));
    let id = term.lock().unwrap().state.terminal_id();
    let wakes = Arc::new(AtomicUsize::new(0));
    let w = wakes.clone();
    let images = SessionImages::new(
        &term,
        id,
        pool.clone(),
        Arc::new(AtomicBool::new(false)),
        Arc::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        }),
        host,
        DecodeQueue::new(),
    );
    Setup { term, pool, images, wakes }
}

impl Setup {
    /// An image waiting for its decode, and the job that decodes `bytes`
    /// into it at `size`.
    fn pending(&self, id: u32, bytes: &[u8], size: (u32, u32)) -> (ImageKey, ImageJob) {
        let mut t = self.term.lock().unwrap();
        let owner = t.state.terminal_id();
        let key = ImageKey::Kitty(id);
        let mut image = Image::new(key, size.0, size.1);
        image.pool = Some(self.pool.reserve(owner, size.0 as usize * size.1 as usize * 4, None).unwrap());
        let generation = image.generation;
        t.state.graphics_mut_for_test().insert(image);
        let mut enc = Encoded::new();
        enc.push(bytes);
        let job = ImageJob::Decode {
            job: JobId::next(),
            image: key,
            generation,
            bytes: Arc::new(enc),
            size,
        };
        (key, job)
    }

    fn submit(&self, job: ImageJob) {
        let id = self.term.lock().unwrap().state.terminal_id();
        self.images.submit(id, job);
    }

    fn state(&self, key: ImageKey) -> ImageState {
        self.term.lock().unwrap().state.graphics().image(key).unwrap().state
    }

    fn pixels(&self, key: ImageKey) -> Arc<dyn PixelBuffer> {
        let t = self.term.lock().unwrap();
        let id = t.state.graphics().image(key).unwrap().pool.unwrap();
        self.pool.pixels(id).expect("pixels in the pool").0
    }
}

#[test]
fn a_png_ends_as_metal_pixels_ready_to_draw() {
    let Some(alloc) = PixelAlloc::system_default() else {
        assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
        eprintln!("skipping: no Metal device");
        return;
    };
    let pool = ImagePool::new(32 << 20);
    let s = setup(Arc::new(AppHost::with_alloc(Some(alloc), pool.clone(), true)), pool);
    // White, black / transparent, grey: colours Display P3 leaves alone.
    #[rustfmt::skip]
    let rgba = [
        255, 255, 255, 255,   0, 0, 0, 255,
        0, 0, 0, 0,           128, 128, 128, 255,
    ];
    let (key, job) = s.pending(1, &png(2, 2, &rgba), (2, 2));
    assert_eq!(s.state(key), ImageState::Pending);
    s.submit(job);
    assert!(s.images.queue().wait_idle(WAIT));
    assert_eq!(s.state(key), ImageState::Ready);
    assert_eq!(s.wakes.load(Ordering::SeqCst), 1, "the window is woken once");

    let pixels = s.pixels(key);
    let metal = pixels
        .as_any()
        .downcast_ref::<MetalPixels>()
        .expect("Metal pixels, which the renderer draws");
    assert_eq!((metal.width(), metal.height()), (2, 2));
    let pitch = metal.row_pitch();
    let at = |x: usize, y: usize| -> [u8; 4] { metal.bytes()[y * pitch + x * 4..][..4].try_into().unwrap() };
    let near = |got: [u8; 4], want: [u8; 4]| got.iter().zip(want).all(|(&g, w)| (g as i32 - w as i32).abs() <= 2);
    // Premultiplied BGRA.
    for (x, y, want) in [
        (0, 0, [255, 255, 255, 255]),
        (1, 0, [0, 0, 0, 255]),
        (0, 1, [0, 0, 0, 0]),
        (1, 1, [128, 128, 128, 255]),
    ] {
        assert!(near(at(x, y), want), "({x}, {y}): got {:?}, want {want:?}", at(x, y));
    }
    s.pool.check().unwrap();
}

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

fn apc(keys: &str, payload: &[u8]) -> Vec<u8> {
    format!("\x1b_G{keys};{}\x1b\\", b64(payload)).into_bytes()
}

/// A terminal with the app's host on the Metal device, as a session makes
/// it, or None without a Metal device.
fn metal_setup() -> Option<Setup> {
    let Some(alloc) = PixelAlloc::system_default() else {
        assert!(std::env::var_os("CI").is_none(), "no Metal device on CI");
        eprintln!("skipping: no Metal device");
        return None;
    };
    let pool = ImagePool::new(32 << 20);
    let host = Arc::new(AppHost::with_alloc(Some(alloc), pool.clone(), true));
    let s = setup(host.clone(), pool);
    {
        let mut t = s.term.lock().unwrap();
        t.state.set_image_host(host);
        t.state.set_reads_off_lock(true);
        t.state.set_cell_pixels((10, 20));
    }
    Some(s)
}

impl Setup {
    /// Parse as the session's reader does: headers read unlocked, then the
    /// jobs queued and run, and a snapshot taken.
    fn feed(&self, bytes: &[u8]) -> term_core::Snapshot {
        {
            let (mut t, r) = crate::session::parse_with_head_reads(&self.term, self.term.lock().unwrap(), bytes);
            r.unwrap();
            self.images.take_jobs(&mut t.state);
        }
        assert!(self.images.queue().wait_idle(WAIT));
        let mut t = self.term.lock().unwrap();
        let mut snap = term_core::Snapshot::default();
        t.state.snapshot(&mut snap, |_| false);
        self.images.take_jobs(&mut t.state);
        snap
    }
}

/// The one placement in `snap`, as Metal pixels of `w` × `h`.
fn drawn_metal(snap: &term_core::Snapshot, w: u32, h: u32) {
    assert_eq!(snap.placements.len(), 1, "one placement in view");
    let image = &snap.placements[0].image;
    assert_eq!((image.width, image.height), (w, h));
    let metal = image.pixels.as_any().downcast_ref::<MetalPixels>().expect("Metal pixels");
    assert_eq!((metal.width(), metal.height()), (w, h));
}

#[test]
fn a_kitty_png_sent_in_the_stream_is_drawn_from_metal_pixels() {
    let Some(s) = metal_setup() else { return };
    let rgba: Vec<u8> = (0..12).flat_map(|_| [255, 255, 255, 255]).collect();
    let snap = s.feed(&apc("a=T,f=100,i=7,q=2", &png(4, 3, &rgba)));
    drawn_metal(&snap, 4, 3);
    let t = s.term.lock().unwrap();
    assert_eq!(t.state.graphics().image(ImageKey::Kitty(7)).unwrap().state, ImageState::Ready);
    s.pool.check().unwrap();
}

#[test]
fn a_kitty_png_from_a_temp_file_is_read_unlocked_and_the_file_removed() {
    let Some(s) = metal_setup() else { return };
    let rgba: Vec<u8> = (0..150).flat_map(|_| [0, 0, 0, 255]).collect();
    let tmp = std::env::temp_dir().canonicalize().unwrap();
    let path = tmp.join(format!("tty-graphics-protocol-td-pipe-{}.png", std::process::id()));
    std::fs::write(&path, png(3, 50, &rgba)).unwrap();
    let snap = s.feed(&apc("a=T,f=100,t=t,i=8,q=2", path.as_os_str().as_encoded_bytes()));
    assert!(!path.exists(), "removed when opened");
    drawn_metal(&snap, 3, 50);
    // The cursor moved past the image's rows, which needed its header.
    let t = s.term.lock().unwrap();
    assert_eq!(t.state.cursor().row(), 2, "three rows of 20 px for 50 px");
}

#[test]
fn raw_kitty_pixels_land_in_metal_pixels() {
    let Some(s) = metal_setup() else { return };
    let snap = s.feed(&apc("a=T,f=24,s=2,v=2,i=9,q=2", &[200; 12]));
    drawn_metal(&snap, 2, 2);
}

/// Vec pixels; decoding bytes starting with `boom` panics.
struct PanickyHost;

impl ImageHost for PanickyHost {
    fn alloc(&self, width: u32, height: u32) -> Option<Box<dyn PixelBuffer>> {
        Some(Box::new(VecBuffer::new(width, height)))
    }
    fn open_medium(&self, _: &MediumRequest) -> Result<OpenMedium, ImageError> {
        Err(ImageError::Refused)
    }
    fn read_head(&self, _: &OpenMedium, _: usize) -> Result<Vec<u8>, ImageError> {
        Err(ImageError::Io)
    }
    fn decode(&self, bytes: &[u8], into: &mut dyn PixelBuffer, _: (u32, u32)) -> Result<(), ImageError> {
        if bytes.starts_with(b"boom") {
            panic!("a decoder bug");
        }
        let target: &mut dyn PixelTarget = into;
        term_image::decode::decode(bytes, target, &Default::default())
            .map(|_| ())
            .map_err(|_| ImageError::BadData("bad"))
    }
}

#[test]
fn a_panicking_decoder_fails_that_image_only() {
    let pool = ImagePool::new(32 << 20);
    let s = setup(Arc::new(PanickyHost), pool);
    let (bad, bad_job) = s.pending(1, b"boom", (2, 2));
    let (good, good_job) = s.pending(2, &png(1, 1, &[1, 2, 3, 255]), (1, 1));
    let (garbage, garbage_job) = s.pending(3, b"not an image", (2, 2));
    s.submit(bad_job);
    s.submit(good_job);
    s.submit(garbage_job);
    assert!(s.images.queue().wait_idle(WAIT));
    assert_eq!(s.state(bad), ImageState::Failed);
    assert_eq!(s.state(good), ImageState::Ready);
    assert_eq!(s.state(garbage), ImageState::Failed);
    s.pixels(good);
    s.pool.check().unwrap();
}

#[test]
fn jobs_of_a_closed_session_dont_run() {
    let pool = ImagePool::new(32 << 20);
    let s = setup(Arc::new(PanickyHost), pool.clone());
    // Hold the queue's thread so the session's jobs wait.
    let (go, wait) = std::sync::mpsc::channel::<()>();
    s.images.queue().push(super::queue::Task {
        owner: u64::MAX,
        bytes: 0,
        work: Box::new(move || {
            wait.recv_timeout(WAIT).unwrap();
            term_core::image::run_job(
                &PanickyHost,
                0,
                ImageJob::Decode {
                    job: JobId::next(),
                    image: ImageKey::Kitty(0),
                    generation: 0,
                    bytes: Arc::new(Encoded::new()),
                    size: (1, 1),
                },
            )
        }),
        done: Box::new(|_| {}),
    });
    let (key, job) = s.pending(1, &png(1, 1, &[1, 2, 3, 255]), (1, 1));
    s.submit(job);
    assert!(s.images.queue().queued(s.images.owner) > 0);
    let queue = s.images.queue().clone();
    let Setup { term, images, .. } = s;
    // The session closes: its side of the queue goes, and its jobs with it.
    drop(images);
    go.send(()).unwrap();
    assert!(queue.wait_idle(WAIT));
    assert_eq!(term.lock().unwrap().state.graphics().image(key).unwrap().state, ImageState::Pending);
    drop(term);
    assert_eq!(pool.used(), 0);
}

#[test]
fn a_job_for_a_replaced_terminal_is_dropped() {
    let pool = ImagePool::new(32 << 20);
    let s = setup(Arc::new(PanickyHost), pool.clone());
    let (_, job) = s.pending(1, &png(1, 1, &[1, 2, 3, 255]), (1, 1));
    let old = s.term.lock().unwrap().state.terminal_id();
    // As after a caught panic: a fresh terminal of the same size.
    crate::session::replace_term(&mut s.term.lock().unwrap());
    s.images.submit(old, job);
    assert!(s.images.queue().wait_idle(WAIT));
    assert_eq!(s.wakes.load(Ordering::SeqCst), 0);
    assert_eq!(pool.used(), 0, "the old terminal's bytes went with it");
}

#[test]
fn the_budget_follows_the_largest_window() {
    let pool = ImagePool::new(32 << 20);
    // Window ids no real window has in tests.
    let (a, b) = (usize::MAX - 1, usize::MAX - 2);
    window_size(&pool, a, 1600, 1000);
    assert_eq!(pool.limit(), 32 << 20, "two 1600 × 1000 images fit in 32 MB");
    window_size(&pool, b, 5120, 2880);
    assert_eq!(pool.limit(), ImagePool::budget_for(5120, 2880));
    window_size(&pool, a, 2000, 1000);
    assert_eq!(pool.limit(), ImagePool::budget_for(5120, 2880), "the largest wins");
    window_size(&pool, b, 0, 0);
    assert_eq!(pool.limit(), 32 << 20, "closed: back to the smaller window's");
    window_size(&pool, a, 0, 0);
}
