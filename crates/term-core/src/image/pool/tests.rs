use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use term_image::PixelTarget;

use super::*;
use crate::image::{PAGE, VecBuffer};

/// A buffer of `pages` whole pages (64 × 64 × 4 bytes is one).
fn buffer(pages: usize) -> Arc<dyn PixelBuffer> {
    Arc::new(VecBuffer::new(64, 64 * pages as u32))
}

/// An idle, placed entry of `owner` holding `pages` of pixels.
fn image(pool: &ImagePool, owner: u64, pages: usize) -> PoolId {
    let id = pool.reserve(owner, pages * PAGE, None).unwrap();
    pool.set_pixels(id, buffer(pages));
    pool.set_placed(id, true);
    pool.set_busy(id, false);
    pool.check().unwrap();
    id
}

fn encoded(bytes: usize) -> Arc<Encoded> {
    let mut e = Encoded::new();
    e.push(&vec![7; bytes]);
    Arc::new(e)
}

fn evicted(pool: &ImagePool, owner: u64) -> Vec<PoolId> {
    let mut v = Vec::new();
    pool.take_evicted(owner, &mut v);
    v
}

#[test]
fn reserve_counts_and_release_gives_back() {
    let pool = ImagePool::new(10 * PAGE);
    let a = image(&pool, 1, 3);
    assert_eq!(pool.used(), 3 * PAGE);
    assert!(pool.pixels(a).is_some());
    pool.release(a);
    assert_eq!(pool.used(), 0);
    assert!(!pool.contains(a));
    pool.check().unwrap();
}

#[test]
fn bigger_than_the_budget_is_refused_before_evicting() {
    let pool = ImagePool::new(10 * PAGE);
    let a = image(&pool, 1, 3);
    assert_eq!(pool.reserve(1, 11 * PAGE, None), Err(Refused));
    assert!(pool.pixels(a).is_some());
    assert!(evicted(&pool, 1).is_empty());
}

#[test]
fn replacing_gives_the_old_bytes_back_first() {
    let pool = ImagePool::new(10 * PAGE);
    let other = image(&pool, 2, 2);
    let old = image(&pool, 1, 8);
    pool.drawn(1, &[old]);
    pool.drawn(2, &[other]);
    // Without giving `old` back first, 8 more pages would only fit by
    // evicting the other window's drawn image, which is protected.
    let new = pool.reserve(1, 8 * PAGE, Some(old)).unwrap();
    assert!(!pool.contains(old));
    assert!(pool.pixels(other).is_some());
    assert_eq!(pool.used(), 10 * PAGE);
    assert!(evicted(&pool, 2).is_empty());
    pool.release(new);
    pool.check().unwrap();
}

#[test]
fn hidden_windows_go_first_then_unplaced_then_oldest() {
    let pool = ImagePool::new(10 * PAGE);
    let old = image(&pool, 1, 2);
    pool.drawn(1, &[old]);
    let newer = image(&pool, 1, 2);
    pool.drawn(1, &[newer]);
    let unplaced = image(&pool, 1, 2);
    pool.set_placed(unplaced, false);
    let hidden = image(&pool, 2, 2);
    pool.drawn(2, &[hidden]);
    pool.set_shown(2, false);
    // A frame drawing nothing: no image of window 1 is protected.
    pool.drawn(1, &[]);
    pool.check().unwrap();

    let mut gone = Vec::new();
    for _ in 0..4 {
        let r = pool.reserve(3, 2 * PAGE + pool.limit() - pool.used(), None).unwrap();
        gone.extend(evicted(&pool, 1));
        gone.extend(evicted(&pool, 2));
        pool.release(r);
    }
    assert_eq!(gone, vec![hidden, unplaced, old, newer]);
    pool.check().unwrap();
}

#[test]
fn images_drawn_in_a_shown_windows_latest_frame_are_protected() {
    let pool = ImagePool::new(4 * PAGE);
    let a = image(&pool, 1, 2);
    let b = image(&pool, 1, 2);
    pool.drawn(1, &[a, b]);
    assert_eq!(pool.reserve(2, PAGE, None), Err(Refused));
    // The next frame draws only `b`: `a` may go.
    pool.drawn(1, &[b]);
    let c = pool.reserve(2, PAGE, None).unwrap();
    assert_eq!(evicted(&pool, 1), vec![a]);
    assert!(pool.pixels(b).is_some());
    // Hidden, `b` isn't protected any more.
    pool.set_shown(1, false);
    pool.reserve(2, 2 * PAGE, None).unwrap();
    assert_eq!(evicted(&pool, 1), vec![b]);
    pool.release(c);
    pool.check().unwrap();
}

#[test]
fn busy_entries_are_never_evicted() {
    let pool = ImagePool::new(4 * PAGE);
    let receiving = pool.reserve(1, 3 * PAGE, None).unwrap();
    assert_eq!(pool.reserve(2, 2 * PAGE, None), Err(Refused));
    assert_eq!(pool.grow(receiving, 2 * PAGE), Err(Refused));
    pool.grow(receiving, PAGE).unwrap();
    assert_eq!(pool.used(), 4 * PAGE);
    // Done, holding nothing: the reservation goes back.
    pool.set_busy(receiving, false);
    assert_eq!(pool.used(), 0);
    pool.check().unwrap();
}

#[test]
fn pixels_go_before_encoded_copies() {
    // An encoded piece is 64 KB, four pages.
    let pool = ImagePool::new(16 * PAGE);
    let a = image(&pool, 1, 4);
    pool.set_encoded(a, Some(encoded(1000)));
    assert_eq!(pool.used(), 4 * PAGE + Encoded::PIECE);
    let b = image(&pool, 1, 1);
    pool.set_encoded(b, Some(encoded(10)));
    pool.check().unwrap();
    // `a`'s pixels free 4 pages; its encoded copy stays.
    let r = pool.reserve(2, 4 * PAGE, None).unwrap();
    assert_eq!(evicted(&pool, 1), vec![a]);
    assert!(pool.contains(a));
    assert!(pool.pixels(a).is_none());
    assert!(pool.encoded(a).is_some());
    pool.release(r);
    // Then only `b`'s pixels are left, then the oldest copy.
    pool.reserve(2, 16 * PAGE - Encoded::PIECE, None).unwrap();
    assert_eq!(evicted(&pool, 1), vec![b, a]);
    assert!(!pool.contains(a));
    assert!(pool.encoded(b).is_some());
    pool.check().unwrap();
}

#[test]
fn lowering_the_limit_evicts_at_once() {
    let pool = ImagePool::new(8 * PAGE);
    let a = image(&pool, 1, 4);
    let b = image(&pool, 1, 4);
    pool.drawn(1, &[a]);
    pool.drawn(1, &[b]);
    pool.set_limit(4 * PAGE);
    assert_eq!(evicted(&pool, 1), vec![a]);
    assert_eq!(pool.used(), 4 * PAGE);
    pool.check().unwrap();
}

#[test]
fn release_owner_frees_only_that_terminal() {
    let pool = ImagePool::new(8 * PAGE);
    image(&pool, 1, 2);
    let busy = pool.reserve(1, PAGE, None).unwrap();
    let other = image(&pool, 2, 2);
    pool.release_owner(1);
    assert!(!pool.contains(busy));
    assert!(pool.contains(other));
    assert_eq!(pool.used(), 2 * PAGE);
    pool.check().unwrap();
}

/// A buffer macOS may take while it is purgeable.
struct Purgeable {
    inner: VecBuffer,
    volatile: AtomicBool,
    taken: AtomicBool,
}

impl PixelTarget for Purgeable {
    fn width(&self) -> u32 {
        self.inner.width()
    }
    fn height(&self) -> u32 {
        self.inner.height()
    }
    fn row_pitch(&self) -> usize {
        self.inner.row_pitch()
    }
    fn bytes_mut(&mut self) -> &mut [u8] {
        self.inner.bytes_mut()
    }
}

impl PixelBuffer for Purgeable {
    fn allocated_size(&self) -> usize {
        self.inner.allocated_size()
    }
    fn set_volatile(&self, volatile: bool) -> bool {
        self.volatile.store(volatile, Ordering::Relaxed);
        volatile || !self.taken.load(Ordering::Relaxed)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[test]
fn hidden_windows_buffers_are_purgeable_and_may_be_taken() {
    let pool = ImagePool::new(8 * PAGE);
    let buf = Arc::new(Purgeable {
        inner: VecBuffer::new(64, 64),
        volatile: AtomicBool::new(false),
        taken: AtomicBool::new(false),
    });
    let with_source = pool.reserve(1, PAGE, None).unwrap();
    pool.set_pixels(with_source, buf.clone());
    pool.set_encoded(with_source, Some(encoded(10)));
    pool.set_busy(with_source, false);
    let without = image(&pool, 1, 1);
    pool.set_shown(1, false);
    assert!(buf.volatile.load(Ordering::Relaxed));
    buf.taken.store(true, Ordering::Relaxed);
    pool.set_shown(1, true);
    assert!(!buf.volatile.load(Ordering::Relaxed));
    assert!(pool.pixels(with_source).is_none());
    assert!(pool.encoded(with_source).is_some());
    assert!(pool.pixels(without).is_some());
    assert_eq!(evicted(&pool, 1), vec![with_source]);
    assert_eq!(pool.used(), PAGE + Encoded::PIECE);
    pool.check().unwrap();
}

#[test]
fn a_poisoned_lock_is_ignored() {
    let pool = ImagePool::new(8 * PAGE);
    let p = pool.clone();
    let _ = std::thread::spawn(move || {
        let _g = p.inner.lock().unwrap();
        panic!("poison it");
    })
    .join();
    assert!(pool.inner.is_poisoned());
    let a = image(&pool, 1, 1);
    assert_eq!(pool.used(), PAGE);
    pool.release(a);
}

#[test]
fn set_pixels_moves_the_generation_on() {
    let pool = ImagePool::new(8 * PAGE);
    let a = pool.reserve(1, PAGE, None).unwrap();
    let g1 = pool.set_pixels(a, buffer(1));
    let g2 = pool.set_pixels(a, buffer(1));
    assert!(g2 > g1);
    assert_eq!(pool.pixels(a).unwrap().1, g2);
}

#[test]
fn budget_is_the_larger_of_32_mb_and_two_window_images() {
    assert_eq!(ImagePool::budget_for(1000, 800), MIN_LIMIT);
    // A 5K window.
    assert_eq!(ImagePool::budget_for(5120, 2880), 5120 * 2880 * 8);
}

#[test]
fn encoded_bytes_grow_in_pieces() {
    let mut e = Encoded::new();
    e.push(&vec![1; Encoded::PIECE - 1]);
    e.push(&[2, 3]);
    assert_eq!(e.len(), Encoded::PIECE + 1);
    assert_eq!(e.pieces().count(), 2);
    let v = e.to_vec();
    assert_eq!(v.len(), Encoded::PIECE + 1);
    assert_eq!(&v[Encoded::PIECE - 1..], &[2, 3]);
}
