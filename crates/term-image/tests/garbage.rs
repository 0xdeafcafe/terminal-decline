//! Every parser and decoder on pseudo-random and mutated input: none may
//! panic. A cheap stand-in for the fuzz targets (fuzz/) that runs with the
//! normal tests, on any platform.

use term_image::sixel::{Decoder, Limits, Palette};
use term_image::{VecPixels, base64, inflate, iterm, kitty, probe};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    /// Bytes drawn mostly from `alphabet`, sometimes anything.
    fn bytes(&mut self, len: usize, alphabet: &[u8]) -> Vec<u8> {
        (0..len)
            .map(|_| {
                let r = self.next();
                if r.is_multiple_of(8) {
                    (r >> 8) as u8
                } else {
                    alphabet[(r >> 8) as usize % alphabet.len()]
                }
            })
            .collect()
    }
}

#[test]
fn sixel_garbage() {
    let mut r = Rng(7);
    for _ in 0..2000 {
        let len = (r.next() % 600) as usize;
        let data = r.bytes(len, b"0123456789;#!\"$-?@~_ABCDEFxyz");
        let mut d = Decoder::<VecPixels>::new((r.next() % 12) as u16, (r.next() % 3) as u16, Palette::vt340(), Limits::default());
        let mut alloc = |w: u32, h: u32| (w as usize * h as usize * 4 <= 1 << 20).then(|| VecPixels::new(w, h));
        d.put_slice(&data, &mut alloc);
        if let Some(img) = d.finish(&mut alloc).image {
            assert!(img.width > 0 && img.height > 0);
        }
    }
}

#[test]
fn keys_args_base64_garbage() {
    let mut r = Rng(11);
    for _ in 0..5000 {
        let len = (r.next() % 80) as usize;
        let keys = r.bytes(len, b"aAtTpdqfiIsvmoUCzxywhcrXY=,-0123456789");
        let _ = kitty::Keys::parse(&keys);
        let args = r.bytes(len, b"inlinewidthheightpx%auto=;:01name");
        let _ = iterm::Args::parse(&args);
        let b = r.bytes(len, b"QUJDZm9v=\n ");
        let _ = base64::decode(&b);
    }
}

#[test]
fn probe_and_inflate_mutations() {
    let mut r = Rng(13);
    let png = {
        let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\x01\0\0\0\x01\0\x08\x06\0\0\0".to_vec();
        v.extend_from_slice(&[0; 8]);
        v
    };
    let jpeg = b"\xff\xd8\xff\xe1\0\x22Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0\xff\xc0\0\x11\x08\0\x10\0\x20\x03\x01\x22\0\x02\x11\x01\x03\x11\x01".to_vec();
    let z = miniz_oxide::deflate::compress_to_vec_zlib(&[7u8; 5000], 6);
    for _ in 0..20_000 {
        for base in [&png, &jpeg, &z] {
            let mut m = base.clone();
            for _ in 0..1 + r.next() % 4 {
                let i = (r.next() as usize) % m.len();
                m[i] = r.next() as u8;
            }
            m.truncate((r.next() as usize) % (m.len() + 1));
            let _ = probe::probe(&m);
            let _ = inflate::inflate_all(&m, 1 << 16);
        }
    }
}
