//! New glyphs found and rasterised, and runs shaped, off the main thread
//! (roadmap R.1).
//!
//! A screenful of glyphs not drawn before (CJK, emoji, symbols) costs
//! 10–200 ms: two thirds finding the fallback font, a third rasterising.
//! Shaping a cluster (an emoji sequence, a letter and its marks) or a
//! stretch of a face with ligatures through CTLine costs up to
//! milliseconds each, mostly the fallback search: on glyphs-30k the
//! longest frame (12–15 ms) was shaping a few clusters. These go to the
//! thread the same way ([`Want::Shape`]): it shapes them with fonts that
//! share the main thread's registry ([`Fonts::share`]), keeps what they
//! shape into where the main thread's fonts read it
//! ([`Fonts::shape_kept`]), and sends their glyphs' bitmaps back.
//! With a [`GlyphWaker`] set ([`Renderer::set_glyph_waker`]), a build that
//! meets such a glyph asks this font set's glyph thread for it, once, and
//! draws the cell without it; the thread finds it through a
//! [`GlyphFinder`] (the same registry as the main thread's `Fonts`, so the
//! same font ids), rasterises it, sends the bitmap back and wakes the app,
//! which builds the panes left waiting again. Bitmaps are put in the
//! glyph textures on the main thread, by the first build that draws them:
//! a flood scrolls most of what a frame asked for away before it is back,
//! and those are dropped unused rather than grow the textures. What no
//! pane's last build wanted any more isn't found or rasterised at all:
//! each pane's build says what it went without ([`GlyphThread::retarget`]),
//! and the thread skips the rest.
//! The thread sleeps on its channel when there is nothing to do and ends
//! with the font set.
//!
//! [`Renderer::set_glyph_waker`]: crate::Renderer::set_glyph_waker

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use term_font::{Features, Fonts, GlyphKey, RasterGlyph, RunText, Style};

/// Called from the glyph thread when glyphs it was asked for are ready:
/// get the panes waiting for them built again (on the main thread). It may
/// be called again before that happens; merging calls is the app's.
pub type GlyphWaker = Arc<dyn Fn() + Send + Sync>;

/// A glyph the thread is asked for.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Want {
    /// A character drawn alone: find its font and glyph, then rasterise.
    Char(u32, Style),
    /// A glyph whose key is known (shaped, or looked up before): rasterise.
    Key(GlyphKey),
    /// A run not shaped before (a cluster cell, a stretch of a face with
    /// ligatures): shape it, keeping what it shapes into for the main
    /// thread ([`Fonts::shape_kept`]), and rasterise its glyphs.
    Shape(Arc<Run>),
}

/// A run to shape, as [`Fonts::shape`] takes it.
#[derive(PartialEq, Eq, Hash, Debug)]
pub(crate) struct Run {
    pub text: RunText,
    pub face: Style,
    pub features: Features,
}

/// What the thread sends back.
pub(crate) struct Found {
    pub want: Want,
    /// `None` when no font has the character.
    pub key: Option<GlyphKey>,
    /// `None` for glyphs with no ink.
    pub glyph: Option<RasterGlyph>,
    /// A run's glyphs other than sprites (drawn on the main thread), each
    /// once, with their bitmaps (`None`: no ink).
    pub shaped: Vec<(GlyphKey, Option<RasterGlyph>)>,
    /// Not looked for: no pane wanted it any more by the time the thread
    /// got to it.
    pub skipped: bool,
}

/// What some pane's last build went without, shared with the thread.
type Live = Arc<Mutex<HashSet<Want>>>;

fn lock(live: &Live) -> std::sync::MutexGuard<'_, HashSet<Want>> {
    live.lock().unwrap_or_else(|e| e.into_inner())
}

/// While results keep coming, the app is woken at most this often, so a
/// screenful of new glyphs is shown as it fills in without a build per
/// glyph.
const WAKE_EVERY: Duration = Duration::from_millis(16);

/// How long a bitmap back from the thread that no pane waits for any
/// more is kept, in case a build draws it after all. Those some pane
/// waits for are kept until it draws them.
const READY_FOR: Duration = Duration::from_millis(500);

/// One font set's glyph thread, as the main thread sees it.
pub(crate) struct GlyphThread {
    tx: Sender<Want>,
    rx: Receiver<Found>,
    /// Asked for and not back yet: each glyph is asked for once.
    pending: HashSet<Want>,
    /// Bitmaps back, not yet put in a texture: when it was sent back,
    /// the bitmap, and what it was asked for as.
    ready: HashMap<GlyphKey, (Instant, RasterGlyph, Want)>,
    /// How many panes' last builds went without each glyph.
    wanted: HashMap<Want, u32>,
    /// The glyphs in `wanted`, or asked for since: the thread skips others.
    live: Live,
}

impl GlyphThread {
    /// Start the thread for `fonts` ([`Fonts::share`] of the main
    /// thread's).
    pub fn spawn(fonts: Fonts, wake: GlyphWaker) -> GlyphThread {
        let (tx, wants) = mpsc::channel::<Want>();
        let (found, rx) = mpsc::channel::<Found>();
        let live = Live::default();
        let shared = live.clone();
        std::thread::Builder::new()
            .name("glyphs".into())
            .spawn(move || run(fonts, wants, found, wake, shared))
            .expect("glyph thread");
        GlyphThread {
            tx,
            rx,
            pending: HashSet::new(),
            ready: HashMap::new(),
            wanted: HashMap::new(),
            live,
        }
    }

    /// Ask for `want` unless it was asked for already.
    pub fn ask(&mut self, want: Want) {
        if !self.pending.contains(&want) {
            self.pending.insert(want.clone());
            lock(&self.live).insert(want.clone());
            // The thread outlives every sender; it can't have gone.
            let _ = self.tx.send(want);
        }
    }

    /// A pane's build went without `new`, its last one without `old`:
    /// glyphs no pane wants any more are no longer looked for, and their
    /// bitmaps are dropped when they come. A pane dropped goes without
    /// nothing ([`PaneRender`]'s drop).
    ///
    /// [`PaneRender`]: crate::PaneRender
    pub fn retarget(&mut self, old: &[Want], new: &[Want]) {
        if old.is_empty() && new.is_empty() {
            return;
        }
        for w in new {
            match self.wanted.get_mut(w) {
                Some(n) => *n += 1,
                None => {
                    self.wanted.insert(w.clone(), 1);
                }
            }
        }
        let mut gone = Vec::new();
        for w in old {
            if let Some(n) = self.wanted.get_mut(w) {
                *n -= 1;
                if *n == 0 {
                    self.wanted.remove(w);
                    gone.push(w);
                }
            }
        }
        if !gone.is_empty() {
            let mut live = lock(&self.live);
            for w in gone {
                live.remove(w);
            }
        }
    }

    /// Whether some pane's last build went without `want`.
    pub fn is_wanted(&self, want: &Want) -> bool {
        self.wanted.contains_key(want)
    }

    /// How many glyphs some pane's last build went without.
    pub fn wanted_len(&self) -> usize {
        self.wanted.len()
    }

    /// Whether `want` was asked for and isn't back yet.
    pub fn is_pending(&self, want: &Want) -> bool {
        self.pending.contains(want)
    }

    /// What came back since the last call; bitmaps no pane waits for
    /// that were kept for [`READY_FOR`] go.
    pub fn take(&mut self) -> Vec<Found> {
        let now = Instant::now();
        let wanted = &self.wanted;
        self.ready
            .retain(|_, (at, _, w)| wanted.contains_key(w) || now.duration_since(*at) < READY_FOR);
        let mut out = Vec::new();
        while let Ok(f) = self.rx.try_recv() {
            self.pending.remove(&f.want);
            out.push(f);
        }
        out
    }

    /// Keep `glyph`, asked for as `want`, until a build draws `key`.
    pub fn keep(&mut self, key: GlyphKey, glyph: RasterGlyph, want: Want) {
        self.ready.insert(key, (Instant::now(), glyph, want));
    }

    /// The bitmap kept for `key`, if one is, and what it was asked for as.
    pub fn ready(&mut self, key: GlyphKey) -> Option<(RasterGlyph, Want)> {
        self.ready.remove(&key).map(|(_, g, w)| (g, w))
    }

    /// Drop the bitmaps kept that no pane waits for: with split panes, one
    /// pane's complete build leaves those another pane's last build went
    /// without until it draws them.
    pub fn drop_unwanted(&mut self) {
        let wanted = &self.wanted;
        self.ready.retain(|_, (_, _, w)| wanted.contains_key(w));
        if self.ready.is_empty() {
            self.ready = HashMap::new();
        }
    }

    /// Whether anything asked for isn't back yet: the panes drawn without
    /// it will be woken for.
    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }
}

fn run(fonts: Fonts, wants: Receiver<Want>, found: Sender<Found>, wake: GlyphWaker, live: Live) {
    // Results sent since the app was last woken, and when it was.
    let mut unsent = false;
    let mut woken = Instant::now();
    // Newest first: in a flood, what older frames asked for has likely
    // scrolled away, and is no longer wanted by the time it comes up.
    let mut stack: Vec<Want> = Vec::new();
    loop {
        loop {
            match wants.try_recv() {
                Ok(w) => stack.push(w),
                Err(TryRecvError::Empty) => break,
                // The font set is gone.
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let want = match stack.pop() {
            Some(w) => w,
            None => {
                if unsent {
                    // Nothing more asked for: show what is done.
                    woken = Instant::now();
                    wake();
                }
                match wants.recv() {
                    Ok(w) => w,
                    Err(_) => return,
                }
            }
        };
        let skipped = !lock(&live).contains(&want);
        let mut shaped = Vec::new();
        let (key, glyph) = match &want {
            _ if skipped => (None, None),
            &Want::Char(cp, style) => {
                let key = char::from_u32(cp).and_then(|ch| fonts.glyph_key(ch, style));
                (key, key.and_then(|k| fonts.rasterize_key(k)))
            }
            &Want::Key(key) => (Some(key), fonts.rasterize_key(key)),
            Want::Shape(run) => {
                let mut glyphs = Vec::new();
                fonts.shape_and_keep(&run.text, run.face, run.features, &mut glyphs);
                let mut seen = HashSet::new();
                for g in glyphs {
                    if g.key.sprite_id().is_none() && seen.insert(g.key) {
                        shaped.push((g.key, fonts.rasterize_key(g.key)));
                    }
                }
                (None, None)
            }
        };
        let f = Found {
            want,
            key,
            glyph,
            shaped,
            skipped,
        };
        if found.send(f).is_err() {
            return; // the font set is gone
        }
        unsent = true;
        if woken.elapsed() >= WAKE_EVERY {
            unsent = false;
            woken = Instant::now();
            wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use term_font::Fonts;

    /// A thread whose waker sends on a channel.
    fn thread(fonts: &Fonts) -> (GlyphThread, Receiver<()>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let wake: GlyphWaker = Arc::new(move || {
            let _ = tx.lock().unwrap().send(());
        });
        (GlyphThread::spawn(fonts.share(), wake), rx)
    }

    /// Everything asked for, gathered over the wakes.
    fn all(t: &mut GlyphThread, wakes: &Receiver<()>) -> Vec<Found> {
        let mut out = Vec::new();
        while t.busy() {
            wakes.recv_timeout(Duration::from_secs(20)).expect("woken");
            out.extend(t.take());
        }
        out
    }

    #[test]
    fn each_glyph_is_asked_for_once() {
        let fonts = Fonts::new("Menlo", 24.0);
        let (mut t, wakes) = thread(&fonts);
        let han = Want::Char('漢' as u32, Style::Regular);
        let bold = Want::Char('漢' as u32, Style::Bold);
        for _ in 0..3 {
            t.ask(han.clone());
            t.ask(bold.clone());
        }
        assert!(t.is_pending(&han) && t.is_pending(&bold));
        let got = all(&mut t, &wakes);
        assert_eq!(got.len(), 2, "one answer per glyph asked for");
        assert!(!t.is_pending(&han));
        let key = got.iter().find(|f| f.want == han).and_then(|f| f.key).expect("found");
        // Found on the glyph thread, with the id the main thread's fonts
        // give it.
        assert_eq!(fonts.cached_glyph_key('漢', Style::Regular), Some(Some(key)));
        assert!(got.iter().all(|f| f.glyph.is_some()), "rasterised");
        // Once back, it can be asked for again (the textures may have
        // forgotten it), here by key: once.
        t.ask(Want::Key(key));
        t.ask(Want::Key(key));
        let again = all(&mut t, &wakes);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].key, Some(key));
    }

    /// A run is shaped on the thread, its glyphs rasterised, and what it
    /// shaped into kept for the main thread's fonts.
    #[test]
    fn runs_are_shaped_and_kept() {
        let fonts = Fonts::new("Menlo", 24.0);
        let (mut t, wakes) = thread(&fonts);
        let mut text = RunText::new();
        text.push("\u{1F469}\u{200D}\u{1F4BB}", 2);
        text.push("e\u{301}", 1);
        let mut got = Vec::new();
        assert!(
            !fonts.shape_kept(&text, Style::Regular, Features::default(), &mut got),
            "not shaped yet"
        );
        let run = Run {
            text: text.clone(),
            face: Style::Regular,
            features: Features::default(),
        };
        t.ask(Want::Shape(Arc::new(run)));
        let back = all(&mut t, &wakes);
        assert_eq!(back.len(), 1);
        assert!(
            back[0].shaped.len() >= 2 && back[0].shaped.iter().all(|(_, g)| g.is_some()),
            "rasterised"
        );
        let mut want = Vec::new();
        fonts.shape(&text, Style::Regular, Features::default(), &mut want);
        assert!(fonts.shape_kept(&text, Style::Regular, Features::default(), &mut got));
        assert_eq!(got, want, "kept as shaped here");
    }

    #[test]
    fn missing_characters_come_back_without_a_glyph() {
        let fonts = Fonts::new("Menlo", 24.0);
        let (mut t, wakes) = thread(&fonts);
        // Not a character.
        t.ask(Want::Char(0xD800, Style::Regular));
        let got = all(&mut t, &wakes);
        assert_eq!(got.len(), 1);
        assert!(got[0].key.is_none() && got[0].glyph.is_none());
    }

    /// A glyph stays wanted while some pane's last build went without it;
    /// once none did, the thread skips it if it hasn't got to it yet.
    #[test]
    fn glyphs_no_pane_wants_are_skipped() {
        let fonts = Fonts::new("Menlo", 24.0);
        let (mut t, wakes) = thread(&fonts);
        let [a, b] = ['漢', '字'].map(|c| Want::Char(c as u32, Style::Regular));
        // Two panes went without `a`, one also without `b`.
        t.retarget(&[], std::slice::from_ref(&a));
        t.retarget(&[], &[a.clone(), b.clone()]);
        assert!(t.is_wanted(&a) && t.is_wanted(&b));
        // The second pane, built again, has them.
        t.retarget(&[a.clone(), b.clone()], &[]);
        assert!(t.is_wanted(&a), "the first pane still waits");
        assert!(!t.is_wanted(&b));
        // Asked for, and no longer wanted by the time the thread gets to
        // it (unless it got there first): skipped.
        t.ask(b.clone());
        lock(&t.live).remove(&b);
        let got = all(&mut t, &wakes);
        assert_eq!(got.len(), 1);
        assert!(
            got[0].skipped || got[0].glyph.is_some(),
            "skipped unless the thread got there first"
        );
        // Skipped, it can be asked for again.
        assert!(!t.is_pending(&b));
    }
}
