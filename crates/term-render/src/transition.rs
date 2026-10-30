//! Theme transitions: the old theme's colours blend into the new one's on
//! the GPU, with no row built again while it plays.
//!
//! Frame building resolves colours to RGBA on the CPU, as always, and the
//! rows are built once with the new theme. While a transition plays, each
//! draw also gets two tables of the theme's [`SLOTS`] configurable colours
//! (the 16 ANSI colours, foreground, background, cursor, the text under
//! it, selection, the two search-hit colours and the pane divider): the
//! old theme's and the new one's. An instance whose colour (RGB, alpha
//! aside) equals a slot of the new table is drawn blended from that slot's
//! old colour; any other colour stays as it is. So palette-indexed colours
//! and the defaults animate, while truecolour, images, emoji and the 240
//! fixed colours of the 256-colour cube stay put. The rule is a match, not
//! a tag: a truecolour equal to a theme colour animates with it, and when
//! two slots of the new theme are the same colour the first one wins.
//! Colours a program set (OSC 4, 10, 11) aren't the theme's, so they stay
//! too.
//!
//! The maths is here, in Rust, for tests and for a transition that starts
//! while another plays (it starts from the colours on screen); the
//! shaders in `shaders.metal` do the same per pixel.

use std::time::{Duration, Instant};

use crate::frame::{Rgba, Theme};

/// The theme colours a transition blends.
pub const SLOTS: usize = 24;
pub type Slots = [Rgba; SLOTS];

/// How the new theme comes in (`theme_transition`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum Kind {
    /// At once.
    None = 0,
    /// A cross-fade.
    #[default]
    Fade = 1,
    /// A soft diagonal sweep from the top left.
    Wave = 2,
    /// A cross-fade with a brief lift in brightness half way.
    Pulse = 3,
    /// A shimmer of small blocks that settles into the new theme.
    Glimmer = 4,
}

impl Kind {
    /// With Reduce Motion on, the moving ones become a cross-fade, which
    /// Apple's guidance offers in place of motion.
    pub fn reduced(self, reduce_motion: bool) -> Kind {
        match self {
            Kind::Wave | Kind::Pulse | Kind::Glimmer if reduce_motion => Kind::Fade,
            k => k,
        }
    }
}

/// The colours of `theme` a transition blends, with the divider between
/// panes last.
pub fn slots(theme: &Theme, divider: Rgba) -> Slots {
    let mut s = [0; SLOTS];
    s[..16].copy_from_slice(&theme.palette[..16]);
    s[16..].copy_from_slice(&[
        theme.foreground,
        theme.background,
        theme.cursor,
        theme.cursor_text,
        theme.selection,
        theme.search_hit,
        theme.search_current,
        divider,
    ]);
    s
}

/// Smoothstep: slow in, slow out.
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How far along a transition of `duration` is after `elapsed`, 0–1.
pub fn progress(elapsed: Duration, duration: Duration) -> f32 {
    if duration.is_zero() {
        return 1.0;
    }
    (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
}

/// The wave's soft edge, as a share of the diagonal.
pub const WAVE_SOFT: f32 = 0.35;
/// How long each glimmer block takes to turn, as a share of the whole.
pub const GLIMMER_TURN: f32 = 0.4;
/// Glimmer blocks are this many pixels square.
pub const GLIMMER_BLOCK: f32 = 8.0;

/// A block's noise, 0–1: an integer hash, so the shader gets the same.
pub fn noise(bx: u32, by: u32) -> f32 {
    let mut h = bx.wrapping_mul(374_761_393).wrapping_add(by.wrapping_mul(668_265_263));
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0x00FF_FFFF) as f32 / 16_777_216.0
}

/// How much of the new colour a pixel shows at progress `p`, at `(x, y)`
/// (0–1 across the drawable) with block noise `n`.
pub fn local(kind: Kind, p: f32, x: f32, y: f32, n: f32) -> f32 {
    if p >= 1.0 || kind == Kind::None {
        return 1.0;
    }
    if p <= 0.0 {
        return 0.0;
    }
    match kind {
        Kind::None => 1.0,
        Kind::Fade | Kind::Pulse => ease(p),
        Kind::Wave => {
            let d = (x + y) * 0.5;
            let front = p * (1.0 + WAVE_SOFT);
            ease((front - d) / WAVE_SOFT)
        }
        Kind::Glimmer => ease((p - n * (1.0 - GLIMMER_TURN)) / GLIMMER_TURN),
    }
}

/// The brightness added (towards white) at progress `p`, at `(x, y)`.
pub fn lift(kind: Kind, p: f32, x: f32, y: f32) -> f32 {
    match kind {
        Kind::Pulse => {
            let k = (p - 0.5) / 0.18;
            0.22 * (-k * k).exp()
        }
        Kind::Glimmer => {
            // A sheen along the diagonal, ahead of the blocks.
            let d = (x + y) * 0.5;
            0.16 * (std::f32::consts::PI * p).sin() * (1.0 - (d - p).abs() * 5.0).max(0.0)
        }
        _ => 0.0,
    }
}

/// `old` moved `t` of the way to `new`, then lifted `lift` of the way to
/// white; alpha blends too.
pub fn blend(old: Rgba, new: Rgba, t: f32, lift: f32) -> Rgba {
    let ch = |shift: u32, lifted: bool| {
        let (a, b) = (((old >> shift) & 0xFF) as f32, ((new >> shift) & 0xFF) as f32);
        let mut v = a + (b - a) * t;
        if lifted {
            v += (255.0 - v) * lift;
        }
        (v.round().clamp(0.0, 255.0) as u32) << shift
    };
    ch(0, true) | ch(8, true) | ch(16, true) | ch(24, false)
}

/// What one draw needs: the two tables and how far along it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blend {
    pub kind: Kind,
    pub progress: f32,
    pub old: Slots,
    pub new: Slots,
}

/// The shaders' `Blend` (shaders.metal): the same layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Uniforms {
    pub progress: f32,
    pub kind: u32,
    pub viewport: [f32; 2],
    pub count: u32,
    pub pad: [u32; 3],
    pub old: Slots,
    pub new: Slots,
}

const _: () = {
    use std::mem::{offset_of, size_of};
    assert!(size_of::<Uniforms>() == 224);
    assert!(offset_of!(Uniforms, viewport) == 8);
    assert!(offset_of!(Uniforms, count) == 16);
    assert!(offset_of!(Uniforms, old) == 32);
    assert!(offset_of!(Uniforms, new) == 128);
};

impl Uniforms {
    pub(crate) fn of(blend: Option<&Blend>, viewport: [f32; 2]) -> Uniforms {
        match blend.filter(|b| b.kind != Kind::None) {
            Some(b) => Uniforms {
                progress: b.progress.clamp(0.0, 1.0),
                kind: b.kind as u32,
                viewport,
                count: SLOTS as u32,
                pad: [0; 3],
                old: b.old,
                new: b.new,
            },
            None => Uniforms {
                progress: 1.0,
                kind: 0,
                viewport,
                count: 0,
                pad: [0; 3],
                old: [0; SLOTS],
                new: [0; SLOTS],
            },
        }
    }
}

/// A transition playing in a window.
#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub kind: Kind,
    pub start: Instant,
    pub duration: Duration,
    pub old: Slots,
    pub new: Slots,
}

impl Transition {
    /// From the colours on screen to `new`: `old` if nothing plays, else
    /// where `playing` has got to by `now`. `None` when there is nothing
    /// to animate: the colours are the same, `kind` is `None` or
    /// `duration` is zero.
    pub fn start(playing: Option<&Transition>, old: Slots, new: Slots, kind: Kind, duration: Duration, now: Instant) -> Option<Transition> {
        let from = playing.filter(|t| !t.done(now)).map_or(old, |t| t.current(now));
        if kind == Kind::None || duration.is_zero() || from == new {
            return None;
        }
        Some(Transition {
            kind,
            start: now,
            duration,
            old: from,
            new,
        })
    }

    pub fn progress(&self, now: Instant) -> f32 {
        progress(now.saturating_duration_since(self.start), self.duration)
    }

    pub fn done(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }

    /// The colours at `now` as a whole (the cross-fade part; a wave's or
    /// glimmer's position and the pulse's lift aside), for a transition
    /// starting from here.
    pub fn current(&self, now: Instant) -> Slots {
        let t = ease(self.progress(now));
        std::array::from_fn(|i| blend(self.old[i], self.new[i], t, 0.0))
    }

    /// The draw at `now`.
    pub fn blend(&self, now: Instant) -> Blend {
        Blend {
            kind: self.kind,
            progress: self.progress(now),
            old: self.old,
            new: self.new,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::rgb;

    fn table(c: u32) -> Slots {
        [rgb(c); SLOTS]
    }

    #[test]
    fn easing_and_progress() {
        assert_eq!(ease(0.0), 0.0);
        assert_eq!(ease(1.0), 1.0);
        assert_eq!(ease(0.5), 0.5);
        assert!(ease(0.1) < 0.1 && ease(0.9) > 0.9, "slow in, slow out");
        assert_eq!(ease(-1.0), 0.0);
        assert_eq!(ease(2.0), 1.0);
        let ms = Duration::from_millis;
        assert_eq!(progress(ms(150), ms(300)), 0.5);
        assert_eq!(progress(ms(600), ms(300)), 1.0);
        assert_eq!(progress(ms(5), Duration::ZERO), 1.0);
    }

    #[test]
    fn blending_channels() {
        assert_eq!(blend(rgb(0x000000), rgb(0xffffff), 0.5, 0.0), rgb(0x808080));
        assert_eq!(blend(rgb(0x204060), rgb(0x204060), 0.3, 0.0), rgb(0x204060));
        assert_eq!(blend(rgb(0x102030), rgb(0x405060), 0.0, 0.0), rgb(0x102030));
        assert_eq!(blend(rgb(0x102030), rgb(0x405060), 1.0, 0.0), rgb(0x405060));
        // A lift goes towards white; alpha is blended but never lifted.
        assert_eq!(blend(rgb(0x000000), rgb(0x000000), 1.0, 0.5), rgb(0x808080));
        let half = rgb(0x000000) & 0x00FF_FFFF | 0x80 << 24;
        assert_eq!(blend(half, half, 1.0, 1.0) >> 24, 0x80);
    }

    #[test]
    fn every_kind_starts_old_and_ends_new() {
        for kind in [Kind::Fade, Kind::Wave, Kind::Pulse, Kind::Glimmer] {
            for (x, y) in [(0.0, 0.0), (1.0, 1.0), (0.3, 0.8)] {
                for n in [0.0, 0.5, 0.999] {
                    assert_eq!(local(kind, 0.0, x, y, n), 0.0, "{kind:?} at the start");
                    assert_eq!(local(kind, 1.0, x, y, n), 1.0, "{kind:?} at the end");
                }
                assert!(lift(kind, 0.0, x, y) < 0.01 && lift(kind, 1.0, x, y) < 0.01, "{kind:?}");
            }
        }
        assert_eq!(local(Kind::None, 0.0, 0.0, 0.0, 0.0), 1.0);
    }

    #[test]
    fn wave_sweeps_from_the_top_left_and_pulse_peaks_half_way() {
        let at = |x, y| local(Kind::Wave, 0.4, x, y, 0.0);
        assert!(at(0.0, 0.0) > at(0.5, 0.5) && at(0.5, 0.5) > at(1.0, 1.0));
        assert_eq!(at(1.0, 1.0), 0.0, "the far corner hasn't started");
        let p = |p| lift(Kind::Pulse, p, 0.0, 0.0);
        assert!(p(0.5) > p(0.3) && p(0.5) > p(0.7) && (p(0.5) - 0.22).abs() < 1e-6);
        // Fade has no lift, and is the same everywhere.
        assert_eq!(lift(Kind::Fade, 0.5, 0.2, 0.2), 0.0);
        assert_eq!(local(Kind::Fade, 0.5, 0.0, 0.0, 0.0), local(Kind::Fade, 0.5, 1.0, 1.0, 0.7));
    }

    #[test]
    fn glimmer_blocks_turn_at_different_times() {
        let ns: Vec<f32> = (0..64).map(|i| noise(i % 8, i / 8)).collect();
        assert!(ns.iter().all(|n| (0.0..1.0).contains(n)));
        let mean = ns.iter().sum::<f32>() / ns.len() as f32;
        assert!((0.3..0.7).contains(&mean), "{mean}");
        let turned = ns.iter().filter(|&&n| local(Kind::Glimmer, 0.5, 0.0, 0.0, n) > 0.5).count();
        assert!(turned > 5 && turned < 59, "some blocks turned, some not: {turned}");
    }

    #[test]
    fn reduce_motion_turns_motion_into_a_fade() {
        assert_eq!(Kind::Wave.reduced(true), Kind::Fade);
        assert_eq!(Kind::Glimmer.reduced(true), Kind::Fade);
        assert_eq!(Kind::Pulse.reduced(true), Kind::Fade);
        assert_eq!(Kind::None.reduced(true), Kind::None);
        assert_eq!(Kind::Wave.reduced(false), Kind::Wave);
    }

    #[test]
    fn a_change_mid_way_starts_from_the_colours_on_screen() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let (black, white, red) = (table(0x000000), table(0xffffff), table(0xff0000));
        assert!(
            Transition::start(None, black, black, Kind::Fade, ms(300), t0).is_none(),
            "nothing changes"
        );
        assert!(Transition::start(None, black, white, Kind::None, ms(300), t0).is_none());
        assert!(Transition::start(None, black, white, Kind::Fade, Duration::ZERO, t0).is_none());
        let first = Transition::start(None, black, white, Kind::Fade, ms(300), t0).unwrap();
        assert_eq!(first.blend(t0 + ms(150)).progress, 0.5);
        assert_eq!(first.current(t0 + ms(150)), table(0x808080));
        let second = Transition::start(Some(&first), white, red, Kind::Wave, ms(300), t0 + ms(150)).unwrap();
        assert_eq!(second.old, table(0x808080), "from the blend, not from white");
        assert_eq!(second.start, t0 + ms(150));
        assert!(!second.done(t0 + ms(300)) && second.done(t0 + ms(450)));
        // A finished one is the old colours again.
        let third = Transition::start(Some(&first), white, red, Kind::Fade, ms(300), t0 + ms(400)).unwrap();
        assert_eq!(third.old, white);
    }

    #[test]
    fn slots_and_uniforms() {
        let theme = Theme::default();
        let s = slots(&theme, rgb(0x123456));
        assert_eq!(s[1], theme.palette[1]);
        assert_eq!(s[16], theme.foreground);
        assert_eq!(s[17], theme.background);
        assert_eq!(s[23], rgb(0x123456));
        let none = Uniforms::of(None, [10.0, 10.0]);
        assert_eq!((none.kind, none.count), (0, 0));
        let b = Blend {
            kind: Kind::Fade,
            progress: 0.5,
            old: s,
            new: s,
        };
        let u = Uniforms::of(Some(&b), [10.0, 20.0]);
        assert_eq!((u.kind, u.count, u.viewport), (1, SLOTS as u32, [10.0, 20.0]));
    }
}
