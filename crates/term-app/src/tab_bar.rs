//! The tab bar (docs/design/tabs.md §3, look A): the window's tabs in
//! its title row, beside the window's buttons, while it has two or more.
//! A band in the theme's colours as a pane's strip is (`pane_strip.rs`),
//! with a view a tab on it: a status dot and what the tab does. The
//! selected tab is cut out of the band in the terminal's own background,
//! with a rail in the cursor colour at its leading edge. A click selects,
//! a middle click closes, `+` opens another. The band between and beside
//! the tabs still moves the window.

use std::cell::{Cell, RefCell};

use objc2::rc::{Retained, Weak};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSBox, NSEvent, NSFont, NSLineBreakMode, NSResponder, NSTextField, NSView};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use crate::pane_strip::{Look, mix, ns_colour, plain_box};
use crate::tab_host::TabHost;

/// Left of the first tab, points: the window's three buttons.
pub const LEADING: f64 = 80.0;
/// The widest a tab gets, points.
const MAX: f64 = 220.0;
/// The `+` after the last tab, and the room kept right of it.
const PLUS: f64 = 28.0;
const TRAILING: f64 = 8.0;
/// A tab's text from its edges, the dot's size and its gap to the text.
const INSET: f64 = 10.0;
const DOT: f64 = 6.0;
const GAP: f64 = 6.0;
/// The selected tab's rail, points.
const RAIL: f64 = 2.0;
/// How far the band is tinted from the background to the foreground (a
/// pane strip's tint), and the others' text.
const TINT: f32 = 0.05;
const TEXT_FAINT: f32 = 0.5;
/// A finished command's dot: green, and red for one that failed.
const DONE: u32 = 0x57AB5A;
const FAILED: u32 = 0xE5534B;

/// How wide each of `tabs` tabs is in a bar `width` points wide: equal
/// shares of the room between the window's buttons and the `+`, never
/// wider than [`MAX`].
pub fn tab_width(tabs: usize, width: f64) -> f64 {
    if tabs == 0 {
        return 0.0;
    }
    let room = (width - LEADING - PLUS - TRAILING).max(0.0);
    (room / tabs as f64).min(MAX).floor()
}

/// What a tab's dot says.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dot {
    /// At its prompt, or a program that doesn't say.
    Quiet,
    /// A command is running.
    Running,
    /// A command finished a moment ago.
    Done,
    /// A command failed a moment ago.
    Failed,
}

/// A tab's dot and text from its pane's status (a pane strip's text,
/// `pane_strip::title_text`): "waiting" becomes the dot and the folder
/// its last two parts, a result's mark becomes the dot's colour.
pub fn label(status: &str) -> (Dot, String) {
    if let Some(folder) = status.strip_prefix("waiting · ") {
        return (Dot::Quiet, short_path(folder));
    }
    if status == "waiting" {
        return (Dot::Quiet, "shell".into());
    }
    if let Some(rest) = status.strip_prefix("✗ ") {
        return (Dot::Failed, rest.into());
    }
    if let Some(rest) = status.strip_prefix("✓ ") {
        return (Dot::Done, rest.into());
    }
    // `make all · 12s`: a running command and its time.
    let timed = status
        .rsplit_once(" · ")
        .is_some_and(|(_, t)| t.starts_with(|c: char| c.is_ascii_digit()));
    (if timed { Dot::Running } else { Dot::Quiet }, status.into())
}

/// The last two parts of `path`, or all of a path that short.
fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [.., a, b] if parts.len() > 2 => format!("{a}/{b}"),
        _ => path.into(),
    }
}

/// Which tab an item is, or the `+`.
const PLUS_INDEX: usize = usize::MAX;

pub struct ItemIvars {
    host: Weak<TabHost>,
    index: Cell<usize>,
    band: Retained<NSBox>,
    rail: Retained<NSBox>,
    dot: Retained<NSBox>,
    label: Retained<NSTextField>,
}

define_class!(
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeclineTabItem"]
    #[ivars = ItemIvars]
    pub struct TabItem;

    impl TabItem {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        /// A press on a tab is the tab's, not a drag of the window.
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        /// The label is for reading only: clicks on it are the tab's.
        /// `point` is in the superview's coordinates, as the frame is.
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
            let f = self.frame();
            let inside = point.x >= f.origin.x
                && point.y >= f.origin.y
                && point.x < f.origin.x + f.size.width
                && point.y < f.origin.y + f.size.height;
            (inside && !self.isHidden()).then(|| Retained::into_super(objc2::Message::retain(self)))
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            let Some(host) = self.ivars().host.load() else { return };
            match self.ivars().index.get() {
                PLUS_INDEX => host.new_tab(),
                i => host.select(i),
            }
        }

        /// A middle click closes the tab, asking as Close Tab does.
        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, _event: &NSEvent) {
            let Some(host) = self.ivars().host.load() else { return };
            let i = self.ivars().index.get();
            if i != PLUS_INDEX {
                host.close_asking(i);
            }
        }
    }

    unsafe impl NSObjectProtocol for TabItem {}
);

impl TabItem {
    fn new(mtm: MainThreadMarker, host: &TabHost, index: usize) -> Retained<TabItem> {
        let label = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        label.setFont(Some(&NSFont::systemFontOfSize(NSFont::smallSystemFontSize())));
        label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        let band = plain_box(mtm);
        let rail = plain_box(mtm);
        let dot = plain_box(mtm);
        dot.setCornerRadius(DOT / 2.0);
        let this = Self::alloc(mtm).set_ivars(ItemIvars {
            host: Weak::new(host),
            index: Cell::new(index),
            band: band.clone(),
            rail: rail.clone(),
            dot: dot.clone(),
            label: label.clone(),
        });
        let this: Retained<TabItem> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.addSubview(&band);
        this.addSubview(&rail);
        this.addSubview(&dot);
        this.addSubview(&label);
        this
    }

    /// Place it at `frame` in the bar, saying `text` after `dot` (none:
    /// the `+`, its text centred), `selected` or not, in `look`'s colours.
    fn show(&self, frame: NSRect, text: &str, dot: Option<Dot>, selected: bool, look: Look) {
        self.setFrame(frame);
        let iv = self.ivars();
        let (w, h) = (frame.size.width, frame.size.height);
        iv.band.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h)));
        iv.rail.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(RAIL, h)));
        iv.band.setHidden(!selected);
        iv.rail.setHidden(!selected);
        iv.dot
            .setFrame(NSRect::new(NSPoint::new(INSET, ((h - DOT) / 2.0).round()), NSSize::new(DOT, DOT)));
        iv.dot.setHidden(dot.is_none());
        let label = &iv.label;
        if label.stringValue().to_string() != text {
            label.setStringValue(&NSString::from_str(text));
        }
        let size = label.intrinsicContentSize();
        let lh = size.height.min(h);
        let (x, lw) = match dot {
            Some(_) => (INSET + DOT + GAP, (w - INSET - DOT - GAP - INSET).max(0.0)),
            None => (((w - size.width) / 2.0).round().max(0.0), size.width.min(w)),
        };
        label.setFrame(NSRect::new(NSPoint::new(x, ((h - lh) / 2.0).round()), NSSize::new(lw, lh)));
        let (fg, bg) = (look.foreground, look.background);
        let ink = if selected { fg } else { mix(bg, fg, TEXT_FAINT) };
        label.setTextColor(Some(&ns_colour(ink, 1.0)));
        iv.band.setFillColor(&ns_colour(bg, look.opacity as f64 / 255.0));
        iv.rail.setFillColor(&ns_colour(look.accent, 1.0));
        let dot_colour = match dot {
            Some(Dot::Running) => mix(bg, fg, 0.8),
            Some(Dot::Done) => DONE,
            Some(Dot::Failed) => FAILED,
            Some(Dot::Quiet) | None => mix(bg, fg, 0.3),
        };
        iv.dot.setFillColor(&ns_colour(dot_colour, 1.0));
    }
}

pub struct Ivars {
    host: Weak<TabHost>,
    band: Retained<NSBox>,
    rule: Retained<NSBox>,
    items: RefCell<Vec<Retained<TabItem>>>,
    plus: Retained<TabItem>,
}

define_class!(
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeclineTabBar"]
    #[ivars = Ivars]
    pub struct TabBar;

    impl TabBar {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// The band is the window's title bar still: a drag moves the
        /// window, a double click zooms it.
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            true
        }
    }

    unsafe impl NSObjectProtocol for TabBar {}
);

impl TabBar {
    /// A hidden bar for `host`.
    pub fn new(mtm: MainThreadMarker, host: &TabHost) -> Retained<TabBar> {
        let band = plain_box(mtm);
        let rule = plain_box(mtm);
        let plus = TabItem::new(mtm, host, PLUS_INDEX);
        let this = Self::alloc(mtm).set_ivars(Ivars {
            host: Weak::new(host),
            band: band.clone(),
            rule: rule.clone(),
            items: RefCell::new(Vec::new()),
            plus: plus.clone(),
        });
        let this: Retained<TabBar> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.addSubview(&band);
        this.addSubview(&rule);
        this.addSubview(&plus);
        this.setHidden(true);
        this
    }

    /// Show it at `frame` in the host with a tab for each of `tabs`
    /// (dot and text), the `selected` one cut out, in `look`'s colours.
    pub fn show(&self, frame: NSRect, tabs: &[(Dot, String)], selected: usize, look: Look) {
        let mtm = MainThreadMarker::from(self);
        self.setFrame(frame);
        let iv = self.ivars();
        let (w, h) = (frame.size.width, frame.size.height);
        iv.band.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h - 1.0)));
        iv.rule.setFrame(NSRect::new(NSPoint::new(0.0, h - 1.0), NSSize::new(w, 1.0)));
        iv.band
            .setFillColor(&ns_colour(mix(look.background, look.foreground, TINT), look.opacity as f64 / 255.0));
        iv.rule.setFillColor(&ns_colour(look.divider, 1.0));
        let mut items = iv.items.borrow_mut();
        while items.len() > tabs.len() {
            if let Some(item) = items.pop() {
                item.removeFromSuperview();
            }
        }
        while items.len() < tabs.len() {
            let Some(host) = iv.host.load() else { break };
            let item = TabItem::new(mtm, &host, items.len());
            self.addSubview(&item);
            items.push(item);
        }
        let each = tab_width(tabs.len(), w);
        for (i, (item, (dot, text))) in items.iter().zip(tabs).enumerate() {
            let at = NSRect::new(NSPoint::new(LEADING + each * i as f64, 0.0), NSSize::new(each, h));
            item.show(at, text, Some(*dot), i == selected, look);
        }
        let end = LEADING + each * tabs.len() as f64;
        iv.plus
            .show(NSRect::new(NSPoint::new(end, 0.0), NSSize::new(PLUS, h)), "+", None, false, look);
        if self.isHidden() {
            self.setHidden(false);
        }
    }

    pub fn hide(&self) {
        if !self.isHidden() {
            self.setHidden(true);
        }
    }

    /// What each tab says (for the test dumps); `None` while hidden.
    pub fn texts(&self) -> Option<Vec<String>> {
        (!self.isHidden()).then(|| {
            self.ivars()
                .items
                .borrow()
                .iter()
                .map(|i| i.ivars().label.stringValue().to_string())
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_share_the_row_up_to_a_width() {
        assert_eq!(tab_width(0, 1000.0), 0.0);
        assert_eq!(tab_width(2, 1400.0), MAX, "two tabs in a wide window: never a slab each");
        let room = 600.0 - LEADING - PLUS - TRAILING;
        assert_eq!(
            tab_width(4, 600.0),
            (room / 4.0).floor(),
            "equal shares when they don't all fit at full width"
        );
        assert_eq!(tab_width(3, 50.0), 0.0, "no room: no width, never negative");
    }

    #[test]
    fn a_tab_says_what_it_does_in_few_words() {
        assert_eq!(
            label("waiting · ~/Source/github.com/0xdeafcafe/h2r"),
            (Dot::Quiet, "0xdeafcafe/h2r".into())
        );
        assert_eq!(label("waiting · ~/src"), (Dot::Quiet, "~/src".into()));
        assert_eq!(label("waiting · /"), (Dot::Quiet, "/".into()));
        assert_eq!(label("waiting"), (Dot::Quiet, "shell".into()));
        assert_eq!(label("cargo run · 39m 41s"), (Dot::Running, "cargo run · 39m 41s".into()));
        assert_eq!(label("✓ cargo build · 1m 12s"), (Dot::Done, "cargo build · 1m 12s".into()));
        assert_eq!(label("✗ make test · exit 3"), (Dot::Failed, "make test · exit 3".into()));
        assert_eq!(label("nvim"), (Dot::Quiet, "nvim".into()));
        assert_eq!(
            label("rush · Cross App"),
            (Dot::Quiet, "rush · Cross App".into()),
            "a program's own title"
        );
    }
}
