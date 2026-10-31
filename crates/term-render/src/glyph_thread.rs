//! New glyphs found and rasterised off the main thread (roadmap R.1).
//!
//! A screenful of glyphs not drawn before (CJK, emoji, symbols) costs
//! 10–200 ms: two thirds finding the fallback font, a third rasterising.
//! With a [`GlyphWaker`] set ([`Renderer::set_glyph_waker`]), a build that
//! meets such a glyph asks this font set's glyph thread for it, once, and
//! draws the cell without it; the thread finds it through a
//! [`GlyphFinder`] (the same registry as the main thread's `Fonts`, so the
//! same font ids), rasterises it, sends the bitmap back and wakes the app,
//! which builds the panes left waiting again. Bitmaps are put in the
//! glyph textures on the main thread, at the start of the next build.
//! The thread sleeps on its channel when there is nothing to do and ends
//! with the font set.
//!
//! [`Renderer::set_glyph_waker`]: crate::Renderer::set_glyph_waker

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use term_font::{GlyphFinder, GlyphKey, RasterGlyph, Style};

/// Called from the glyph thread when glyphs it was asked for are ready:
/// get the panes waiting for them built again (on the main thread). It may
/// be called again before that happens; merging calls is the app's.
pub type GlyphWaker = Arc<dyn Fn() + Send + Sync>;

/// A glyph the thread is asked for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Want {
    /// A character drawn alone: find its font and glyph, then rasterise.
    Char(u32, Style),
    /// A glyph whose key is known (shaped, or looked up before): rasterise.
    Key(GlyphKey),
}

/// What the thread sends back.
pub(crate) struct Found {
    pub want: Want,
    /// `None` when no font has the character.
    pub key: Option<GlyphKey>,
    /// `None` for glyphs with no ink.
    pub glyph: Option<RasterGlyph>,
}

/// While results keep coming, the app is woken at most this often, so a
/// screenful of new glyphs is shown as it fills in without a build per
/// glyph.
const WAKE_EVERY: Duration = Duration::from_millis(16);

/// One font set's glyph thread, as the main thread sees it.
pub(crate) struct GlyphThread {
    tx: Sender<Want>,
    rx: Receiver<Found>,
    /// Asked for and not back yet: each glyph is asked for once.
    pending: HashSet<Want>,
    /// Back, but not put in a texture yet (no room until it grows).
    pub waiting: Vec<Found>,
}

impl GlyphThread {
    pub fn spawn(finder: GlyphFinder, wake: GlyphWaker) -> GlyphThread {
        let (tx, wants) = mpsc::channel::<Want>();
        let (found, rx) = mpsc::channel::<Found>();
        std::thread::Builder::new()
            .name("glyphs".into())
            .spawn(move || run(finder, wants, found, wake))
            .expect("glyph thread");
        GlyphThread {
            tx,
            rx,
            pending: HashSet::new(),
            waiting: Vec::new(),
        }
    }

    /// Ask for `want` unless it was asked for already.
    pub fn ask(&mut self, want: Want) {
        if self.pending.insert(want) {
            // The thread outlives every sender; it can't have gone.
            let _ = self.tx.send(want);
        }
    }

    /// Whether `want` was asked for and isn't back yet.
    #[cfg(test)]
    pub fn is_pending(&self, want: Want) -> bool {
        self.pending.contains(&want)
    }

    /// What came back since the last call, with what was waiting first.
    pub fn take(&mut self) -> Vec<Found> {
        let mut out = std::mem::take(&mut self.waiting);
        while let Ok(f) = self.rx.try_recv() {
            self.pending.remove(&f.want);
            out.push(f);
        }
        out
    }

    /// Whether anything is asked for or waiting: the panes drawn without
    /// it will be woken for.
    pub fn busy(&self) -> bool {
        !self.pending.is_empty() || !self.waiting.is_empty()
    }
}

fn run(finder: GlyphFinder, wants: Receiver<Want>, found: Sender<Found>, wake: GlyphWaker) {
    // Results sent since the app was last woken, and when it was.
    let mut unsent = false;
    let mut woken = Instant::now();
    loop {
        let want = match wants.try_recv() {
            Ok(w) => w,
            Err(TryRecvError::Empty) => {
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
            // The font set is gone.
            Err(TryRecvError::Disconnected) => return,
        };
        let (key, glyph) = match want {
            Want::Char(cp, style) => {
                let key = char::from_u32(cp).and_then(|ch| finder.glyph_key(ch, style));
                (key, key.and_then(|k| finder.rasterize_key(k)))
            }
            Want::Key(key) => (Some(key), finder.rasterize_key(key)),
        };
        if found.send(Found { want, key, glyph }).is_err() {
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
        (GlyphThread::spawn(fonts.finder(), wake), rx)
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
            t.ask(han);
            t.ask(bold);
        }
        assert!(t.is_pending(han) && t.is_pending(bold));
        let got = all(&mut t, &wakes);
        assert_eq!(got.len(), 2, "one answer per glyph asked for");
        assert!(!t.is_pending(han));
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
}
