//! Theme transitions: the old theme's colours blend into the new one's on
//! the GPU, with no row built again while it plays.
//!
//! Frame building resolves colours to RGBA on the CPU, as always, and the
//! rows are built once with the new theme. Each instance also carries
//! where its colour came from (a [`Tag`](crate::frame::Tag)): one of the
//! theme's [`SLOTS`] configurable colours (the 16 ANSI colours,
//! foreground, background, cursor, the text under it, selection, the two
//! search-hit colours and the pane divider), or a colour of its own.
//! While a transition plays, each draw also gets tables of those colours:
//! the ones under it and the new theme's, and a tagged instance is drawn
//! blended from its slot's colour under it. A change while one plays goes
//! on top of it ([`LEVELS`] at most), from the colours on screen pixel by
//! pixel, as the one under it carries on. So palette-indexed colours
//! and the defaults animate, while truecolour (even one equal to a theme
//! colour), images, emoji, the 240 fixed colours of the 256-colour cube
//! and colours a program set (OSC 4, 10, 11) stay put.
//!
//! The maths is here, in Rust, for tests and for a transition that starts
//! while another plays (it starts from the colours on screen); the
//! shaders in `shaders.metal` do the same per pixel.

use std::time::{Duration, Instant};

use crate::frame::{Rgba, Theme};

/// The theme colours a transition blends.
pub const SLOTS: usize = 24;

/// Where each colour is in [`Slots`], after the 16 ANSI colours.
pub mod slot {
    pub const FOREGROUND: usize = 16;
    pub const BACKGROUND: usize = 17;
    pub const CURSOR: usize = 18;
    pub const CURSOR_TEXT: usize = 19;
    pub const SELECTION: usize = 20;
    pub const SEARCH_HIT: usize = 21;
    pub const SEARCH_CURRENT: usize = 22;
    pub const DIVIDER: usize = 23;
}
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

/// Transitions a draw blends at most, one over another: a change while
/// some play adds one more, so what is on screen goes on moving where it
/// was (a wave half way across stays half way across, under the new one
/// starting). Past this many (a key held down in the theme list), the
/// oldest is folded into the colours under it as its cross-fade, the
/// only time anything jumps.
pub const LEVELS: usize = 8;

/// One transition of a [`Blend`]: how it moves, how far along it is and
/// the theme it goes to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    pub kind: Kind,
    pub progress: f32,
    pub to: Slots,
}

/// What one draw needs: the colours under every transition playing, and
/// those transitions, oldest first. A tagged colour starts from its slot
/// in `from`, and each level moves it on towards its own table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blend {
    pub from: Slots,
    pub levels: [Level; LEVELS],
    pub count: usize,
}

impl Blend {
    /// One transition from `old` to `new`.
    pub fn single(kind: Kind, progress: f32, old: Slots, new: Slots) -> Blend {
        let mut b = Blend {
            from: old,
            levels: [Level {
                kind: Kind::None,
                progress: 1.0,
                to: new,
            }; LEVELS],
            count: 1,
        };
        b.levels[0] = Level { kind, progress, to: new };
        b
    }

    pub fn levels(&self) -> &[Level] {
        &self.levels[..self.count]
    }

    /// Whether this draw is the new theme's own: the last transition has
    /// ended (or none plays).
    pub fn finished(&self) -> bool {
        self.levels().last().is_none_or(|l| l.kind == Kind::None || l.progress >= 1.0)
    }

    /// The colour of slot `slot` at `(x, y)` (0–1 across the drawable),
    /// with block noise `n`, as the shaders draw it.
    pub fn colour(&self, slot: usize, x: f32, y: f32, n: f32) -> Rgba {
        self.levels().iter().fold(self.from[slot], |c, l| {
            blend(c, l.to[slot], local(l.kind, l.progress, x, y, n), lift(l.kind, l.progress, x, y))
        })
    }
}

/// The shaders' `Level` and `Blend` (shaders.metal): the same layouts.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LevelUniforms {
    pub progress: f32,
    pub kind: u32,
    pub pad: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Uniforms {
    pub viewport: [f32; 2],
    /// Levels; 0 while none plays.
    pub count: u32,
    pub pad: u32,
    pub levels: [LevelUniforms; LEVELS],
    pub from: Slots,
    pub to: [Slots; LEVELS],
}

const _: () = {
    use std::mem::{offset_of, size_of};
    assert!(size_of::<LevelUniforms>() == 16);
    assert!(offset_of!(Uniforms, count) == 8);
    assert!(offset_of!(Uniforms, levels) == 16);
    assert!(offset_of!(Uniforms, from) == 16 + 16 * LEVELS);
    assert!(offset_of!(Uniforms, to) == 16 + 16 * LEVELS + 4 * SLOTS);
    assert!(size_of::<Uniforms>() == 16 + 16 * LEVELS + 4 * SLOTS * (LEVELS + 1));
    // setVertexBytes takes up to 4 KB.
    assert!(size_of::<Uniforms>() <= 4096);
};

impl Uniforms {
    pub(crate) fn of(blend: Option<&Blend>, viewport: [f32; 2]) -> Uniforms {
        let mut u = Uniforms {
            viewport,
            count: 0,
            pad: 0,
            levels: [LevelUniforms {
                progress: 1.0,
                kind: 0,
                pad: [0; 2],
            }; LEVELS],
            from: [0; SLOTS],
            to: [[0; SLOTS]; LEVELS],
        };
        if let Some(b) = blend.filter(|b| !b.finished()) {
            u.count = b.count as u32;
            u.from = b.from;
            for (i, l) in b.levels().iter().enumerate() {
                u.levels[i] = LevelUniforms {
                    progress: l.progress.clamp(0.0, 1.0),
                    kind: l.kind as u32,
                    pad: [0; 2],
                };
                u.to[i] = l.to;
            }
        }
        u
    }

    /// Whether a transition plays in this draw.
    pub(crate) fn playing(&self) -> bool {
        self.count != 0
    }
}

/// One transition of a [`Transition`].
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub kind: Kind,
    pub start: Instant,
    pub duration: Duration,
    pub to: Slots,
}

impl Step {
    pub fn progress(&self, now: Instant) -> f32 {
        progress(now.saturating_duration_since(self.start), self.duration)
    }

    fn done(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }
}

/// The transitions playing in a window: from `from`, each step on from
/// where the ones before it have got to, oldest first.
#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub from: Slots,
    pub steps: Vec<Step>,
}

impl Transition {
    /// From the colours on screen to `new`: from `old` if nothing plays,
    /// else on top of `playing`, which goes on as it was, so nothing on
    /// screen moves at the change. `None` when there is nothing to
    /// animate: the colours are already `new`, `kind` is `None` or
    /// `duration` is zero (the new theme at once).
    pub fn start(playing: Option<&Transition>, old: Slots, new: Slots, kind: Kind, duration: Duration, now: Instant) -> Option<Transition> {
        if kind == Kind::None || duration.is_zero() {
            return None;
        }
        let step = Step {
            kind,
            start: now,
            duration,
            to: new,
        };
        let Some(mut t) = playing.filter(|t| !t.done(now)).cloned() else {
            return (old != new).then(|| Transition {
                from: old,
                steps: vec![step],
            });
        };
        t.settle(now);
        if t.steps.last().is_some_and(|s| s.to == new) {
            return Some(t);
        }
        t.steps.push(step);
        while t.steps.len() > LEVELS {
            // The oldest, folded into the colours under it as its
            // cross-fade so far.
            let s = t.steps.remove(0);
            let e = ease(s.progress(now));
            t.from = std::array::from_fn(|i| blend(t.from[i], s.to[i], e, 0.0));
        }
        Some(t)
    }

    /// Drops the steps a later one has covered: once a step has ended,
    /// the colours are its own, whatever was under it.
    fn settle(&mut self, now: Instant) {
        if let Some(i) = self.steps.iter().rposition(|s| s.done(now)) {
            self.from = self.steps[i].to;
            self.steps.drain(..=i);
        }
    }

    /// Whether the newest step has ended, and with it everything.
    pub fn done(&self, now: Instant) -> bool {
        self.steps.last().is_none_or(|s| s.done(now))
    }

    /// The draw at `now`.
    pub fn blend(&self, now: Instant) -> Blend {
        let mut t = self.clone();
        t.settle(now);
        let mut b = Blend::single(Kind::None, 1.0, t.from, t.from);
        b.count = t.steps.len().min(LEVELS);
        for (i, s) in t.steps.iter().take(LEVELS).enumerate() {
            b.levels[i] = Level {
                kind: s.kind,
                progress: s.progress(now),
                to: s.to,
            };
        }
        b
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
        assert_eq!(first.blend(t0 + ms(150)).levels()[0].progress, 0.5);
        assert_eq!(first.blend(t0 + ms(150)).colour(0, 0.5, 0.5, 0.0), rgb(0x808080));
        let second = Transition::start(Some(&first), white, red, Kind::Wave, ms(300), t0 + ms(150)).unwrap();
        assert_eq!(second.from, black, "the first goes on under it");
        assert_eq!(second.steps.len(), 2);
        assert_eq!(
            second.blend(t0 + ms(150)).colour(0, 0.5, 0.5, 0.0),
            rgb(0x808080),
            "from the screen"
        );
        assert!(!second.done(t0 + ms(300)) && second.done(t0 + ms(450)));
        // Once the first has ended, only the second is left, from white.
        let b = second.blend(t0 + ms(320));
        assert_eq!((b.count, b.from), (1, white));
        // A finished one is the old colours again.
        let third = Transition::start(Some(&first), white, red, Kind::Fade, ms(300), t0 + ms(400)).unwrap();
        assert_eq!((third.from, third.steps.len()), (white, 1));
        // The same theme again changes nothing.
        assert_eq!(
            Transition::start(Some(&first), white, white, Kind::Fade, ms(300), t0 + ms(10)).unwrap(),
            first
        );
    }

    /// The reviewer's case: half way through a wave from black to white
    /// the top left is white and the bottom right black. Another change
    /// then starts from exactly that, pixel by pixel, for every kind that
    /// moves or lifts, and whatever the new one is; nothing jumps to a
    /// uniform grey.
    #[test]
    fn an_interrupted_wave_glimmer_or_pulse_starts_from_each_pixel() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let (black, white, red) = (table(0x000000), table(0xffffff), table(0xff0000));
        let half = t0 + ms(150);
        let points = [(0.0, 0.0), (0.2, 0.1), (0.5, 0.5), (0.7, 0.9), (1.0, 1.0)];
        for first_kind in [Kind::Wave, Kind::Glimmer, Kind::Pulse] {
            let first = Transition::start(None, black, white, first_kind, ms(300), t0).unwrap();
            let shown = first.blend(half);
            if first_kind == Kind::Wave {
                assert_eq!(shown.colour(0, 0.0, 0.0, 0.0), rgb(0xffffff), "top left white");
                assert_eq!(shown.colour(0, 1.0, 1.0, 0.0), rgb(0x000000), "bottom right black");
            }
            for second_kind in [Kind::Fade, Kind::Wave, Kind::Glimmer, Kind::Pulse] {
                let second = Transition::start(Some(&first), white, red, second_kind, ms(300), half).unwrap();
                let now = second.blend(half);
                for (x, y) in points {
                    for n in [0.0, 0.3, 0.9] {
                        assert_eq!(
                            now.colour(0, x, y, n),
                            shown.colour(0, x, y, n),
                            "{first_kind:?} then {second_kind:?} at ({x}, {y}), noise {n}"
                        );
                    }
                }
                // And it ends at red everywhere.
                let end = second.blend(half + ms(300));
                assert!(end.finished());
                assert_eq!(end.colour(0, 0.3, 0.6, 0.5), rgb(0xff0000));
                // Under way, it is still moving smoothly: a little later is
                // a little different.
                let soon = second.blend(half + ms(5));
                for (x, y) in points {
                    let (a, b) = (shown.colour(0, x, y, 0.3), soon.colour(0, x, y, 0.3));
                    let d = (0..3)
                        .map(|i| ((a >> (8 * i)) & 0xff).abs_diff((b >> (8 * i)) & 0xff))
                        .max()
                        .unwrap();
                    assert!(d <= 40, "{first_kind:?} then {second_kind:?} at ({x}, {y}): {a:08x} to {b:08x}");
                }
            }
        }
    }

    /// A key held down in the theme list: a change every 10 ms. The steps
    /// under way stay at most `LEVELS`, and those that ended drop out.
    #[test]
    fn many_changes_keep_a_bounded_stack() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut playing: Option<Transition> = None;
        let mut shown = table(0x000000);
        for i in 0..50u32 {
            let now = t0 + ms(10 * i as u64);
            let new = table(i * 0x050301 + 0x101010);
            playing = Transition::start(playing.as_ref(), shown, new, Kind::Wave, ms(300), now);
            shown = new;
            let t = playing.as_ref().unwrap();
            assert!(t.steps.len() <= LEVELS);
        }
        let t = playing.unwrap();
        let end = t0 + ms(10 * 49 + 300);
        assert!(t.done(end) && t.blend(end).finished());
        assert_eq!(t.blend(end).colour(3, 0.5, 0.5, 0.5), shown[3]);
        // Spaced out, they drop out as they end.
        let first = Transition::start(None, table(0), table(0xffffff), Kind::Fade, ms(100), t0).unwrap();
        let second = Transition::start(Some(&first), table(0xffffff), table(0xff0000), Kind::Fade, ms(100), t0 + ms(50)).unwrap();
        let third = Transition::start(Some(&second), table(0xff0000), table(0x00ff00), Kind::Fade, ms(100), t0 + ms(120)).unwrap();
        assert_eq!((third.steps.len(), third.from), (2, table(0xffffff)));
    }

    #[test]
    fn slots_and_uniforms() {
        let theme = Theme::default();
        let s = slots(&theme, rgb(0x123456));
        assert_eq!(s[1], theme.palette[1]);
        assert_eq!(s[slot::FOREGROUND], theme.foreground);
        assert_eq!(s[slot::BACKGROUND], theme.background);
        assert_eq!(s[slot::SELECTION], theme.selection);
        assert_eq!(s[slot::DIVIDER], rgb(0x123456));
        let none = Uniforms::of(None, [10.0, 10.0]);
        assert!(!none.playing());
        let b = Blend::single(Kind::Fade, 0.5, s, s);
        let u = Uniforms::of(Some(&b), [10.0, 20.0]);
        assert_eq!((u.levels[0].kind, u.count, u.viewport), (1, 1, [10.0, 20.0]));
        assert!(
            !Uniforms::of(Some(&Blend::single(Kind::Fade, 1.0, s, s)), [1.0, 1.0]).playing(),
            "ended"
        );
    }
}
