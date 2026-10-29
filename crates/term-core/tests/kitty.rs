//! kitty graphics (images.md §3, PR I.2): commands, replies, chunking,
//! media, deletes and their scopes, scrolling, the budget and the caps,
//! with the pure-Rust host. Conformance rows follow the examples in
//! kitty's protocol document.

use std::sync::Arc;

use term_core::image::{ImageKey, ImageState, MAX_IMAGES, RustHost, run_job};
use term_core::{ImageJob, ImagePool, Snapshot, Term};

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

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut wr = e.write_header().unwrap();
        let data: Vec<u8> = (0..w * h).flat_map(|i| [i as u8, 0x40, 0x80, 0xFF]).collect();
        wr.write_image_data(&data).unwrap();
    }
    out
}

fn term_with(cols: usize, lines: usize, budget: usize) -> Term {
    let mut t = Term::new(cols, lines, 100, ImagePool::new(budget));
    t.state.set_image_host(Arc::new(RustHost::with_media()));
    t.state.set_cell_pixels((10, 20));
    t
}

fn term() -> Term {
    term_with(20, 6, 32 << 20)
}

fn replies(t: &mut Term) -> String {
    String::from_utf8(t.state.take_replies()).unwrap()
}

/// Run the jobs queued, as the session does with the lock released.
fn run_jobs(t: &mut Term) {
    let mut jobs: Vec<ImageJob> = Vec::new();
    t.state.take_image_jobs(&mut jobs);
    let host = t.state.image_host().clone();
    for job in jobs {
        let done = run_job(&*host, t.state.terminal_id(), job);
        t.state.image_job_done(done);
    }
}

fn rgb(w: u32, h: u32) -> Vec<u8> {
    vec![0x80; (w * h * 3) as usize]
}

fn check(t: &Term) {
    t.state.check_invariants().unwrap();
    t.state.image_pool().check().unwrap();
}

#[test]
fn transmit_replies_ok_and_stores_raw_pixels() {
    let mut t = term();
    t.advance(&apc("a=t,i=1,f=24,s=2,v=2", &rgb(2, 2)));
    assert_eq!(replies(&mut t), "\x1b_Gi=1;OK\x1b\\");
    let image = t.state.graphics().image(ImageKey::Kitty(1)).unwrap();
    assert_eq!((image.width, image.height, image.state), (2, 2, ImageState::Ready));
    assert!(image.opaque);
    assert!(t.state.image_pool().used() > 0);
    check(&t);
}

#[test]
fn quiet_modes_and_unnamed_commands() {
    let mut t = term();
    t.advance(&apc("a=t,i=1,f=24,s=1,v=1,q=1", &rgb(1, 1)));
    assert_eq!(replies(&mut t), "", "q=1 drops OK");
    t.advance(&apc("a=t,i=2,f=24,s=2,v=2,q=1", &rgb(1, 1)));
    assert!(replies(&mut t).starts_with("\x1b_Gi=2;ENODATA:"), "q=1 keeps errors");
    t.advance(&apc("a=t,i=3,f=24,s=2,v=2,q=2", &rgb(1, 1)));
    assert_eq!(replies(&mut t), "", "q=2 drops errors too");
    t.advance(&apc("a=t,f=24,s=1,v=1", &rgb(1, 1)));
    assert_eq!(replies(&mut t), "", "no i or I: no reply");
    check(&t);
}

#[test]
fn a_query_is_answered_before_a_da1_sent_after_it() {
    let mut t = term();
    let mut bytes = apc("a=q,i=31,s=1,v=1,f=24", b"\0\0\0");
    bytes.extend_from_slice(b"\x1b[c");
    t.advance(&bytes);
    assert_eq!(replies(&mut t), "\x1b_Gi=31;OK\x1b\\\x1b[?62;22c");
    assert!(t.state.graphics().image(ImageKey::Kitty(31)).is_none(), "a query stores nothing");
    assert_eq!(t.state.image_pool().used(), 0);
}

#[test]
fn nothing_is_replied_with_images_off() {
    // The process pool has the hidden switch off in tests.
    let mut t = Term::new(20, 6, 100, ImagePool::process());
    if t.state.images_enabled() {
        return;
    }
    t.advance(&apc("a=q,i=31,s=1,v=1,f=24", b"\0\0\0"));
    t.advance(&apc("a=T,i=1,f=24,s=1,v=1", b"\0\0\0"));
    assert_eq!(replies(&mut t), "");
    assert!(t.state.graphics().is_empty());
    assert_eq!(t.state.cursor().col(), 0);
}

#[test]
fn image_numbers_get_fresh_ids_and_place_by_number() {
    let mut t = term();
    t.advance(&apc("a=t,i=1,f=24,s=1,v=1", &rgb(1, 1)));
    replies(&mut t);
    t.advance(&apc("a=t,I=13,f=24,s=1,v=1", &rgb(1, 1)));
    let r = replies(&mut t);
    assert!(r.starts_with("\x1b_Gi=") && r.contains(",I=13;OK"), "{r:?}");
    assert!(!r.starts_with("\x1b_Gi=1,"), "a fresh id, not one in use");
    t.advance(b"\x1b_Ga=p,I=13,p=4\x1b\\");
    let r = replies(&mut t);
    assert!(r.contains(",I=13,p=4;OK"), "{r:?}");
    assert_eq!(t.state.graphics().placements().count(), 1);
    t.advance(b"\x1b_Ga=t,i=5,I=6;\x1b\\");
    assert!(replies(&mut t).contains("EINVAL"), "i and I together");
    check(&t);
}

#[test]
fn replacing_by_id_frees_the_old_image_and_its_placements() {
    let mut t = term();
    t.advance(&apc("a=T,i=7,f=24,s=10,v=20,q=2", &rgb(10, 20)));
    assert_eq!(t.state.graphics().placements().count(), 1);
    let used = t.state.image_pool().used();
    t.advance(&apc("a=t,i=7,f=24,s=10,v=20,q=2", &rgb(10, 20)));
    assert_eq!(t.state.graphics().placements().count(), 0, "the protocol deletes its placements");
    assert_eq!(t.state.graphics().images().count(), 1);
    assert_eq!(t.state.image_pool().used(), used);
    check(&t);
}

#[test]
fn replacing_a_20mb_image_under_a_32mb_budget_succeeds() {
    let mut t = term();
    let (w, h) = (2500, 2000);
    let data = vec![0u8; (w * h * 4) as usize];
    t.advance(&apc(&format!("a=T,i=1,f=32,s={w},v={h},C=1"), &data));
    assert_eq!(replies(&mut t), "\x1b_Gi=1;OK\x1b\\");
    t.advance(&apc(&format!("a=T,i=1,f=32,s={w},v={h},C=1"), &data));
    assert_eq!(replies(&mut t), "\x1b_Gi=1;OK\x1b\\");
    assert!(t.state.image_pool().used() <= 32 << 20);
    check(&t);
}

#[test]
fn chunks_continue_and_abandoned_uploads_give_their_bytes_back() {
    let mut t = term();
    let data = b64(&rgb(4, 4));
    let (a, b) = data.split_at(16);
    t.advance(format!("\x1b_Ga=t,i=3,f=24,s=4,v=4,m=1;{a}\x1b\\").as_bytes());
    assert_eq!(replies(&mut t), "");
    // Some clients repeat their keys on every chunk.
    t.advance(format!("\x1b_Ga=t,i=3,f=24,s=4,v=4,m=0;{b}\x1b\\").as_bytes());
    assert_eq!(replies(&mut t), "\x1b_Gi=3;OK\x1b\\");
    // kitty's own chunks: only m on the later ones.
    let (a, b) = data.split_at(20);
    t.advance(format!("\x1b_Gi=4,f=24,s=4,v=4,m=1;{a}\x1b\\\x1b_Gm=0;{b}\x1b\\").as_bytes());
    assert_eq!(replies(&mut t), "\x1b_Gi=4;OK\x1b\\");
    let used = t.state.image_pool().used();
    // Abandoned by CAN mid-chunk.
    t.advance(format!("\x1b_Gi=5,f=24,s=4,v=4,m=1;{a}\x18").as_bytes());
    assert_eq!(t.state.image_pool().used(), used);
    assert!(t.state.graphics().image(ImageKey::Kitty(5)).is_none());
    // Abandoned by a delete between chunks.
    t.advance(format!("\x1b_Gi=6,f=24,s=4,v=4,m=1;{a}\x1b\\\x1b_Ga=d,d=z,z=99\x1b\\\x1b_Gm=0;{b}\x1b\\").as_bytes());
    assert!(t.state.graphics().image(ImageKey::Kitty(6)).is_none());
    assert_eq!(t.state.image_pool().used(), used);
    // Another command between chunks runs, and the upload waits.
    t.advance(format!("\x1b_Gi=8,f=24,s=4,v=4,m=1;{a}\x1b\\\x1b_Ga=p,i=3\x1b\\\x1b_Gm=0;{b}\x1b\\").as_bytes());
    assert_eq!(replies(&mut t), "\x1b_Gi=3;OK\x1b\\\x1b_Gi=8;OK\x1b\\");
    check(&t);
    // RIS mid-upload gives it back too.
    t.advance(format!("\x1b_Gi=9,f=24,s=4,v=4,m=1;{a}\x1b\\\x1bc").as_bytes());
    assert_eq!(t.state.image_pool().used(), 0);
    check(&t);
}

#[test]
fn transmit_and_place_moves_the_cursor_unless_c_is_1() {
    let mut t = term();
    // 25 × 30 pixels in 10 × 20 cells: 3 columns, 2 rows.
    t.advance(b"ab");
    t.advance(&apc("a=T,i=1,f=24,s=25,v=30,q=2", &rgb(25, 30)));
    let p = t.state.graphics().placements().next().unwrap().1.clone();
    assert_eq!((p.line, p.col, p.cols, p.rows), (0, 2, 3, 2));
    assert_eq!((t.state.cursor().row(), t.state.cursor().col()), (1, 5));
    t.advance(&apc("a=T,i=2,f=24,s=25,v=30,C=1,q=2", &rgb(25, 30)));
    assert_eq!((t.state.cursor().row(), t.state.cursor().col()), (1, 5));
    // Past the last column: column 0 of the next row.
    t.advance(b"\x1b[1;19H");
    t.advance(&apc("a=T,i=3,f=24,s=25,v=10,q=2", &rgb(25, 10)));
    assert_eq!((t.state.cursor().row(), t.state.cursor().col()), (1, 0));
    // c and r override the size.
    t.advance(&apc("a=T,i=4,f=24,s=1,v=1,c=5,r=3,C=1,q=2", &rgb(1, 1)));
    let p = t
        .state
        .graphics()
        .placements()
        .find(|(_, p)| p.image == ImageKey::Kitty(4))
        .unwrap()
        .1
        .clone();
    assert_eq!((p.cols, p.rows), (5, 3));
    check(&t);
}

#[test]
fn two_unnumbered_placements_on_one_row_both_stay() {
    let mut t = term();
    t.advance(&apc("a=t,i=1,f=24,s=1,v=1,q=2", &rgb(1, 1)));
    t.advance(b"\x1b_Ga=p,i=1\x1b\\\x1b_Ga=p,i=1\x1b\\");
    assert_eq!(t.state.graphics().placements().count(), 2);
    // The same placement id replaces.
    t.advance(b"\x1b_Ga=p,i=1,p=3\x1b\\\x1b_Ga=p,i=1,p=3\x1b\\");
    assert_eq!(t.state.graphics().placements().count(), 3);
    t.advance(b"\x1b_Ga=p,i=99\x1b\\");
    assert!(replies(&mut t).ends_with("\x1b_Gi=99;ENOENT:no such image\x1b\\"));
    check(&t);
}

/// A terminal with image 1 placed at (row 0, col 0), image 2 at (row 2,
/// col 5) with z=3, image 3 virtual, image 4 never placed.
fn scene() -> Term {
    let mut t = term();
    t.advance(&apc("a=T,i=1,f=24,s=20,v=20,C=1,q=2", &rgb(20, 20)));
    t.advance(b"\x1b[3;6H");
    t.advance(&apc("a=T,i=2,f=24,s=10,v=20,z=3,C=1,q=2", &rgb(10, 20)));
    t.advance(&apc("a=T,i=3,f=24,s=10,v=20,U=1,q=2", &rgb(10, 20)));
    t.advance(&apc("a=t,i=4,f=24,s=1,v=1,q=2", &rgb(1, 1)));
    t.advance(b"\x1b[H");
    t
}

fn placed(t: &Term) -> Vec<u32> {
    let mut v: Vec<u32> = t
        .state
        .graphics()
        .placements()
        .map(|(_, p)| match p.image {
            ImageKey::Kitty(i) => i,
            _ => 0,
        })
        .collect();
    v.sort();
    v
}

fn has(t: &Term, id: u32) -> bool {
    t.state.graphics().image(ImageKey::Kitty(id)).is_some()
}

#[test]
fn every_delete_specifier_with_its_scope() {
    type Check = fn(&Term) -> bool;
    let rows: &[(&str, Check)] = &[
        ("a=d", |t| {
            placed(t).is_empty() && has(t, 1) && t.state.graphics().virtual_placements().count() == 1
        }),
        ("a=d,d=A", |t| {
            placed(t).is_empty() && !has(t, 1) && !has(t, 2) && has(t, 3) && has(t, 4)
        }),
        ("a=d,d=i,i=1", |t| placed(t) == [2] && has(t, 1)),
        ("a=d,d=I,i=1", |t| placed(t) == [2] && !has(t, 1)),
        ("a=d,d=I,i=4", |t| !has(t, 4)),
        ("a=d,d=i,i=3", |t| t.state.graphics().virtual_placements().count() == 0 && has(t, 3)),
        ("a=d,d=r,x=1,y=2", |t| placed(t).is_empty() && has(t, 2)),
        ("a=d,d=R,x=2,y=4", |t| placed(t) == [1] && !has(t, 2) && !has(t, 3) && !has(t, 4)),
        ("a=d,d=c", |t| placed(t) == [2]),
        ("a=d,d=p,x=6,y=3", |t| placed(t) == [1]),
        ("a=d,d=p,x=5,y=3", |t| placed(t) == [1, 2]),
        ("a=d,d=q,x=6,y=3,z=0", |t| placed(t) == [1, 2]),
        ("a=d,d=Q,x=6,y=3,z=3", |t| placed(t) == [1] && !has(t, 2)),
        ("a=d,d=x,x=6", |t| placed(t) == [1]),
        ("a=d,d=y,y=1", |t| placed(t) == [2]),
        ("a=d,d=z,z=3", |t| placed(t) == [1]),
        ("a=d,d=f", |t| placed(t) == [1, 2]),
    ];
    for (keys, ok) in rows {
        let mut t = scene();
        assert_eq!(placed(&t), [1, 2]);
        t.advance(format!("\x1b_G{keys}\x1b\\").as_bytes());
        assert!(ok(&t), "{keys}: placed {:?}", placed(&t));
        check(&t);
    }
}

#[test]
fn deletes_by_id_reach_history_and_screen_deletes_do_not() {
    let mut t = term();
    t.advance(&apc("a=T,i=1,f=24,s=10,v=20,q=2", &rgb(10, 20)));
    t.advance(b"\r\n\r\n\r\n\r\n\r\n\r\n\r\n\r\n");
    let p = t.state.graphics().placements().next().unwrap().1.clone();
    assert_eq!(p.line, 0, "it scrolled into history with its line");
    t.advance(b"\x1b_Ga=d\x1b\\");
    assert_eq!(placed(&t), [1], "d=a is the visible screen only");
    t.advance(b"\x1b_Ga=d,d=i,i=1\x1b\\");
    assert!(placed(&t).is_empty(), "d=i reaches history");
    check(&t);
}

#[test]
fn placements_follow_scroll_regions_and_clears() {
    let mut t = term();
    // Region rows 2–5 (1-based 3–6); an image at row 3, one at row 0.
    t.advance(&apc("a=T,i=1,f=24,s=10,v=20,C=1,q=2", &rgb(10, 20)));
    t.advance(b"\x1b[4;1H");
    t.advance(&apc("a=T,i=2,f=24,s=10,v=20,C=1,q=2", &rgb(10, 20)));
    t.advance(b"\x1b[3;6r\x1b[6;1H\n");
    let line = |t: &Term, id| {
        t.state
            .graphics()
            .placements()
            .find(|(_, p)| p.image == ImageKey::Kitty(id))
            .map(|(_, p)| p.line)
    };
    assert_eq!(line(&t, 1), Some(0), "outside the region: stays");
    assert_eq!(line(&t, 2), Some(2), "inside: up a row");
    t.advance(b"\n");
    assert_eq!(line(&t, 2), None, "scrolled out of the region: gone");
    t.advance(b"\x1b[r");
    // ED 2 deletes what is on screen; ED 3 what is in history.
    t.advance(b"\x1b[H");
    t.advance(&apc("a=T,i=3,f=24,s=10,v=20,C=1,q=2", &rgb(10, 20)));
    t.advance(b"\x1b[2J");
    assert!(placed(&t).is_empty());
    t.advance(&apc("a=T,i=3,f=24,s=10,v=20,C=1,q=2", &rgb(10, 20)));
    t.advance(b"\x1b[20B\n\n\n\n\n\n\n\n");
    assert_eq!(placed(&t), [3]);
    t.advance(b"\x1b[3J");
    assert!(placed(&t).is_empty());
    check(&t);
}

#[test]
fn the_alternate_screen_has_its_own_images() {
    let mut t = term();
    t.advance(&apc("a=T,i=1,f=24,s=10,v=20,C=1,q=2", &rgb(10, 20)));
    t.advance(b"\x1b[?1049h");
    assert!(placed(&t).is_empty());
    t.advance(&apc("a=T,i=1,f=24,s=10,v=20,C=1,q=2", &rgb(10, 20)));
    assert_eq!(placed(&t), [1]);
    // Scrolled off the alternate screen: gone.
    t.advance(b"\x1b[6;1H\n\n\n\n\n\n\n");
    assert!(placed(&t).is_empty());
    t.advance(b"\x1b[?1049l");
    assert_eq!(placed(&t), [1], "the primary screen's image is back");
    check(&t);
}

#[test]
fn png_decodes_off_the_lock_and_replies_when_done() {
    let mut t = term();
    t.advance(&apc("a=T,i=9,f=100", &png(30, 10)));
    assert_eq!(replies(&mut t), "", "the reply waits for the decode");
    assert_eq!(t.state.graphics().image(ImageKey::Kitty(9)).unwrap().state, ImageState::Pending);
    assert_eq!(t.state.cursor().col(), 3, "moved by the header's size at once");
    run_jobs(&mut t);
    assert_eq!(replies(&mut t), "\x1b_Gi=9;OK\x1b\\");
    assert_eq!(t.state.graphics().image(ImageKey::Kitty(9)).unwrap().state, ImageState::Ready);
    t.advance(&apc("a=t,i=10,f=100", b"not a png"));
    assert!(replies(&mut t).starts_with("\x1b_Gi=10;EBADPNG"));
    check(&t);
}

#[test]
fn zlib_compressed_raw_data() {
    let mut t = term();
    let z = miniz_compress(&rgb(8, 8));
    t.advance(&apc("a=t,i=1,f=24,s=8,v=8,o=z", &z));
    assert_eq!(replies(&mut t), "\x1b_Gi=1;OK\x1b\\");
    check(&t);
}

/// A stored (uncompressed) zlib stream: enough to exercise `o=z`.
fn miniz_compress(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    for (i, block) in data.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= data.len();
        out.push(last as u8);
        let n = block.len() as u16;
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&(!n).to_le_bytes());
        out.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&(b << 16 | a).to_be_bytes());
    out
}

#[test]
fn files_temp_files_and_shared_memory_go_through_the_host() {
    let dir = std::env::temp_dir().join(format!("td-kitty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut t = term();
    // A PNG file: opened and its header read at once, decoded by a job.
    let file = dir.join("a.png");
    std::fs::write(&file, png(20, 20)).unwrap();
    t.advance(&apc("a=T,i=1,t=f,f=100", file.to_str().unwrap().as_bytes()));
    assert_eq!(t.state.cursor().col(), 2);
    run_jobs(&mut t);
    assert_eq!(replies(&mut t), "\x1b_Gi=1;OK\x1b\\");
    // A temporary file is removed when opened.
    let tmp = std::env::temp_dir().join(format!("tty-graphics-protocol-{}.rgb", std::process::id()));
    std::fs::write(&tmp, rgb(2, 2)).unwrap();
    t.advance(&apc("a=t,i=2,t=t,f=24,s=2,v=2", tmp.to_str().unwrap().as_bytes()));
    assert!(!tmp.exists(), "removed at open");
    run_jobs(&mut t);
    assert_eq!(replies(&mut t), "\x1b_Gi=2;OK\x1b\\");
    // Not in the temp dir, or not named so: refused.
    let wrong = dir.join("tty-graphics-protocol-x");
    std::fs::write(&wrong, rgb(2, 2)).unwrap();
    t.advance(&apc("a=t,i=3,t=t,f=24,s=2,v=2", wrong.to_str().unwrap().as_bytes()));
    assert!(replies(&mut t).starts_with("\x1b_Gi=3;EBADF"));
    assert!(wrong.exists());
    // Shared memory, unlinked at open (mpv reuses the name every frame).
    let host = Arc::new(RustHost::with_media());
    t.state.set_image_host(host.clone());
    host.put_shared_memory(b"/mpv-frame", rgb(4, 4));
    t.advance(&apc("a=t,i=4,t=s,f=24,s=4,v=4", b"/mpv-frame"));
    assert!(!host.has_shared_memory(b"/mpv-frame"));
    run_jobs(&mut t);
    assert_eq!(replies(&mut t), "\x1b_Gi=4;OK\x1b\\");
    t.advance(&apc("a=t,i=5,t=s,f=24,s=4,v=4", b"/missing"));
    assert!(replies(&mut t).starts_with("\x1b_Gi=5;EBADF"));
    // Media off: refused, so icat falls back to direct.
    t.state.set_image_host(Arc::new(RustHost::new()));
    t.advance(&apc("a=q,i=6,t=f,f=100", file.to_str().unwrap().as_bytes()));
    assert!(replies(&mut t).starts_with("\x1b_Gi=6;EBADF"));
    check(&t);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_header_read_can_wait_for_the_lock_to_be_released() {
    let dir = std::env::temp_dir().join(format!("td-kitty-head-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("b.png");
    std::fs::write(&file, png(40, 20)).unwrap();
    let mut t = term();
    t.state.set_reads_off_lock(true);
    let mut bytes = apc("a=T,i=1,t=f,f=100", file.to_str().unwrap().as_bytes());
    bytes.extend_from_slice(b"xy");
    let n = t.advance_until_head(&bytes);
    assert_eq!(n, bytes.len() - 2, "stopped right after the command");
    let read = t.state.take_head_read().expect("a header to read");
    assert_eq!(t.advance_until_head(&bytes[n..]), 0, "nothing parsed while waiting");
    // The session reads it unlocked, then locks again.
    let head = t.state.image_host().read_head(read.medium(), read.max);
    t.state.head_read_done(read, head);
    assert_eq!(t.state.cursor().col(), 4);
    assert_eq!(t.advance_until_head(&bytes[n..]), 2);
    assert_eq!(t.state.cursor().col(), 6);
    // Term::advance does the same by itself.
    t.advance(&apc("a=T,i=2,t=f,f=100,C=1", file.to_str().unwrap().as_bytes()));
    run_jobs(&mut t);
    assert_eq!(replies(&mut t), "\x1b_Gi=1;OK\x1b\\\x1b_Gi=2;OK\x1b\\");
    check(&t);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn mpv_id_less_frames_stay_within_one_image() {
    // A frame every time at the same place, no ids, no deletes.
    let mut t = term_with(80, 24, 32 << 20);
    let (w, h) = (160, 100);
    let frame = apc(&format!("a=T,f=24,s={w},v={h},C=1,q=2"), &rgb(w, h));
    for _ in 0..1000 {
        t.advance(b"\x1b[H");
        t.advance(&frame);
    }
    assert_eq!(t.state.graphics().images().count(), 1);
    assert_eq!(t.state.graphics().placements().count(), 1);
    assert_eq!(t.state.image_pool().used(), (w * h * 4) as usize / 16384 * 16384 + 16384);
    assert_eq!(replies(&mut t), "");
    // Deleting its placement frees an id-less image.
    t.advance(b"\x1b_Ga=d\x1b\\");
    assert_eq!(t.state.graphics().images().count(), 0);
    assert_eq!(t.state.image_pool().used(), 0);
    check(&t);
}

#[test]
fn full_screen_mpv_frames_near_the_budget_still_fit() {
    let mut t = term_with(80, 24, 32 << 20);
    let (w, h) = (1600, 1000);
    let frame = apc(&format!("a=T,f=24,s={w},v={h},C=1,q=2"), &rgb(w, h));
    for _ in 0..20 {
        t.advance(b"\x1b[H");
        t.advance(&frame);
        assert!(t.state.image_pool().used() <= 32 << 20);
    }
    assert_eq!(t.state.graphics().images().count(), 1);
    check(&t);
}

#[test]
fn over_the_budget_or_the_side_cap_is_refused() {
    let mut t = term_with(20, 6, 1 << 20);
    t.advance(&apc("a=t,i=1,f=32,s=1000,v=1000", &[]));
    assert!(replies(&mut t).starts_with("\x1b_Gi=1;EFBIG"));
    let mut t = term();
    t.advance(&apc("a=t,i=1,f=24,s=10001,v=1", &[]));
    assert!(replies(&mut t).starts_with("\x1b_Gi=1;EFBIG"));
    assert_eq!(t.state.image_pool().used(), 0);
}

#[test]
fn the_image_cap_holds() {
    let mut t = term_with(20, 6, 1 << 30);
    for i in 1..=MAX_IMAGES as u32 + 10 {
        t.advance(&apc(&format!("a=t,i={i},f=24,s=1,v=1,q=2"), &rgb(1, 1)));
    }
    assert_eq!(t.state.graphics().images().count(), MAX_IMAGES);
    assert!(has(&t, MAX_IMAGES as u32 + 10));
    assert!(!has(&t, 1), "the oldest unplaced went first");
    check(&t);
}

#[test]
fn the_snapshot_lists_placements_in_view_and_virtual_ones_with_placeholders() {
    let mut t = term();
    t.advance(b"\r\n");
    t.advance(&apc("a=T,i=1,f=24,s=20,v=40,C=1,q=2", &rgb(20, 40)));
    let mut snap = Snapshot::default();
    t.state.snapshot(&mut snap, |_| false);
    assert_eq!(snap.placements.len(), 1);
    let p = &snap.placements[0];
    assert_eq!((p.view_row, p.col, p.cols, p.rows), (1, 0, 2, 2));
    assert_eq!((p.image.width, p.image.height), (20, 40));
    assert!(snap.virtual_placements.is_empty());
    // Scrolled half out of the top: still listed, at row −1.
    t.advance(b"\x1b[6;1H\n\n");
    t.state.snapshot(&mut snap, |_| false);
    assert_eq!(snap.placements.len(), 1);
    assert_eq!(snap.placements[0].view_row, -1);
    // A virtual placement and a placeholder cell naming image 2.
    t.advance(&apc("a=T,i=2,f=24,s=10,v=20,U=1,q=2", &rgb(10, 20)));
    t.advance("\x1b[38;5;2m\u{10EEEE}\u{305}\u{305}\x1b[m".as_bytes());
    t.state.snapshot(&mut snap, |_| false);
    assert_eq!(snap.virtual_placements.len(), 1);
    assert_eq!(snap.virtual_placements[0].key, (ImageKey::Kitty(2), 0));
    // Pending and evicted images aren't listed.
    t.advance(&apc("a=T,i=3,f=100,C=1", &png(5, 5)));
    t.state.snapshot(&mut snap, |_| false);
    assert_eq!(snap.placements.len(), 1);
    run_jobs(&mut t);
    t.state.snapshot(&mut snap, |_| false);
    assert_eq!(snap.placements.len(), 2);
    check(&t);
}

/// kitty's protocol document, example by example.
#[test]
fn conformance_with_kittys_examples() {
    let red = b64(&[255, 0, 0]);
    let rows: Vec<(String, &str)> = vec![
        // "Querying support and available transmission mediums"
        (
            format!("\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;{}\x1b\\", b64(&[0, 0, 0])),
            "\x1b_Gi=31;OK\x1b\\",
        ),
        // Transmit with an id, then display it.
        (format!("\x1b_Gi=10,f=24,s=1,v=1;{red}\x1b\\"), "\x1b_Gi=10;OK\x1b\\"),
        ("\x1b_Ga=p,i=10\x1b\\".into(), "\x1b_Gi=10;OK\x1b\\"),
        ("\x1b_Ga=p,i=10,p=7\x1b\\".into(), "\x1b_Gi=10,p=7;OK\x1b\\"),
        // "Suppressing responses from the terminal"
        (format!("\x1b_Gi=11,f=24,s=1,v=1,q=1;{red}\x1b\\"), ""),
        // Display an image that doesn't exist.
        ("\x1b_Ga=p,i=12\x1b\\".into(), "\x1b_Gi=12;ENOENT:no such image\x1b\\"),
        // Image numbers.
        (format!("\x1b_GI=93,f=24,s=1,v=1;{red}\x1b\\"), ",I=93;OK\x1b\\"),
        // Delete replies only with errors.
        ("\x1b_Ga=d,d=i,i=10\x1b\\".into(), ""),
        // Animation is not supported: no reply, as kitty with an unknown action.
        ("\x1b_Ga=f,i=10,r=1\x1b\\".into(), ""),
        // Without i or I nothing is replied.
        (format!("\x1b_Gf=24,s=1,v=1;{red}\x1b\\"), ""),
    ];
    let mut t = term();
    for (input, want) in rows {
        t.advance(input.as_bytes());
        let got = replies(&mut t);
        if want.starts_with(',') {
            assert!(got.ends_with(want), "{input:?}: {got:?}");
        } else {
            assert_eq!(got, want, "{input:?}");
        }
    }
    check(&t);
}

#[test]
fn reply_codes_are_kittys() {
    let mut t = term_with(20, 6, 1 << 20);
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (b"\x1b_Ga=t,i=1,f=24\x1b\\".to_vec(), "EINVAL"),
        (b"\x1b_Ga=p,i=2\x1b\\".to_vec(), "ENOENT"),
        (apc("a=t,i=3,t=f,f=24,s=1,v=1", b"/no/such/file"), "EBADF"),
        (apc("a=t,i=4,f=24,s=2,v=2", &[0; 3]), "ENODATA"),
        (apc("a=t,i=5,f=32,s=1000,v=1000", &[]), "EFBIG"),
        (apc("a=t,i=6,f=100", b"\x89PNG\r\n\x1a\n...."), "EBADPNG"),
        (b"\x1b_Ga=t,i=7,f=99\x1b\\".to_vec(), "EINVAL"),
    ];
    for (input, code) in cases {
        t.advance(&input);
        let r = replies(&mut t);
        assert!(r.contains(&format!(";{code}:")), "{code}: {r:?}");
    }
}

#[test]
fn other_apc_strings_are_skipped() {
    let mut t = term();
    t.advance(b"\x1b_Xsomething;else\x1b\\ok");
    t.advance(b"\x1b_\x1b\\");
    assert_eq!(replies(&mut t), "");
    assert_eq!(t.state.cursor().col(), 2);
    // Keys over 4 KB: ignored whole.
    let long = format!("\x1b_Gi=1,{}\x1b\\", "z=1,".repeat(1100));
    t.advance(long.as_bytes());
    assert_eq!(replies(&mut t), "");
}
