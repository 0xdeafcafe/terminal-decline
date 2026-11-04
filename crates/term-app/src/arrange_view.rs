//! Arranging panes with the keyboard (pane-arrange design §3): Cmd+Shift+M
//! lifts the focused pane, Cmd+Shift+N starts a new one, the arrows step it
//! through the places it could go ([`Panes::places`]), and this view, over
//! the whole window, shows the layout each would make: the panes dimmed,
//! each pane of the new layout outlined with its number, name and grid,
//! the arranged one filled with the accent colour, and a hint bar at the
//! bottom. Return places it, Esc (or a click, or the window losing key)
//! cancels. No program is resized until Return. Plain AppKit views, as
//! the find bar is: the Metal path doesn't change.

use objc2::rc::{Retained, Weak};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBox, NSBoxType, NSColor, NSEvent, NSEventModifierFlags, NSFont, NSLineBreakMode, NSResponder, NSTextAlignment, NSTextField,
    NSTitlePosition, NSView,
};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use crate::control::ArrangeKey;
use crate::panes::{Direction, Moving, Node, PaneId, Panes, Place, Side, Spot};
use crate::view::WindowView;

/// The hint bar's height, points.
const BAR: f64 = 30.0;

/// An arranging under way: what moves, where it could go, and where it is.
pub(crate) struct Arrange {
    pub(crate) moving: Moving,
    pub(crate) places: Vec<Place>,
    pub(crate) at: usize,
    pub(crate) view: Retained<ArrangeView>,
}

impl Arrange {
    /// Do `key`: a new place (true), or nothing to do (false: beep).
    /// Return and Esc are the window's ([`WindowView::end_arrange`]).
    pub(crate) fn key(&mut self, key: ArrangeKey, order: &[PaneId]) -> bool {
        let next = match key {
            ArrangeKey::Towards(d) => Panes::step(&self.places, self.at, d),
            ArrangeKey::Next => Some((self.at + 1) % self.places.len()),
            ArrangeKey::Previous => Some((self.at + self.places.len() - 1) % self.places.len()),
            ArrangeKey::Swap => {
                let cur = &self.places[self.at];
                match (&cur.spot, cur.beside) {
                    // Back beside it: its first place there.
                    (Spot::Swap(x), _) => self
                        .places
                        .iter()
                        .position(|p| p.beside == Some(*x) && !matches!(p.spot, Spot::Swap(_))),
                    (_, Some(x)) => self.places.iter().position(|p| p.spot == Spot::Swap(x)),
                    _ => None,
                }
            }
            ArrangeKey::Pane(n) => {
                let id = n.checked_sub(1).and_then(|i| order.get(i)).copied();
                match id {
                    Some(id) if id == self.moving.id() => self.places.iter().position(|p| p.spot == Spot::Home),
                    Some(id) => self.places.iter().position(|p| p.spot == Spot::Swap(id)),
                    None => None,
                }
            }
            ArrangeKey::Place | ArrangeKey::Cancel => None,
        };
        match next {
            Some(i) if i != self.at => {
                self.at = i;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn place(&self) -> &Place {
        &self.places[self.at]
    }
}

/// The panes under the node at `path` of `root`, in tree order.
fn leaves_at(root: &Node, path: &[bool]) -> Vec<PaneId> {
    let mut node = root;
    for &second in path {
        match node {
            Node::Split { a, b, .. } => node = if second { b } else { a },
            Node::Leaf(_) => return Vec::new(),
        }
    }
    let mut out = Vec::new();
    fn walk(n: &Node, out: &mut Vec<PaneId>) {
        match n {
            Node::Leaf(id) => out.push(*id),
            Node::Split { a, b, .. } => {
                walk(a, out);
                walk(b, out);
            }
        }
    }
    walk(node, &mut out);
    out
}

fn side_word(side: Side) -> &'static str {
    match side {
        Side::Left => "left of",
        Side::Right => "right of",
        Side::Top => "above",
        Side::Bottom => "below",
    }
}

fn edge_word(side: Side) -> &'static str {
    match side {
        Side::Left => "the window's left edge",
        Side::Right => "the window's right edge",
        Side::Top => "the window's top edge",
        Side::Bottom => "the window's bottom edge",
    }
}

/// What the hint bar says of `place` for `moving`, among `tree` (the
/// panes as they are): `MOVE nvim → right of cargo`. `name` names a pane.
pub(crate) fn describe(moving: Moving, place: &Place, tree: &Panes, name: &dyn Fn(PaneId) -> String) -> String {
    let what = match moving {
        Moving::Pane(id) => format!("MOVE {}", name(id)),
        Moving::New(_) => "NEW PANE".to_string(),
    };
    let to = match &place.spot {
        Spot::Home => "where it is".to_string(),
        Spot::Swap(x) => format!("swap with {}", name(*x)),
        Spot::Beside { node, side } if node.0.is_empty() => edge_word(*side).to_string(),
        Spot::Beside { node, side } => {
            // The node is a path in the tree without the moving pane.
            let mut rest = tree.clone();
            if let Moving::Pane(id) = moving {
                rest.close(id);
            }
            let under: Vec<String> = leaves_at(rest.root(), &node.0).into_iter().map(name).collect();
            format!("{} {}", side_word(*side), under.join(" + "))
        }
    };
    format!("{what} \u{2192} {to}")
}

/// The keys the hint bar lists.
pub(crate) const KEYS: &str =
    "\u{2190}\u{2191}\u{2193}\u{2192} move   \u{21e5} next   \u{2423} swap   1\u{2013}9 pane   \u{23ce} place   esc cancel";

/// One pane of the preview: where (window points, y down), what it says,
/// and whether it is the arranged one (`New`: a new pane).
pub(crate) struct Outline {
    pub(crate) rect: NSRect,
    pub(crate) text: String,
    pub(crate) arranged: Option<bool>,
}

pub struct Ivars {
    window: Weak<WindowView>,
}

define_class!(
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeclineArrangeView"]
    #[ivars = Ivars]
    pub struct ArrangeView;

    impl ArrangeView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        /// While it is up, the menus' shortcuts do nothing: the keys are
        /// the mode's. Keys without Cmd or Ctrl go on to `keyDown:`.
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            let flags = event.modifierFlags();
            flags.contains(NSEventModifierFlags::Command) || flags.contains(NSEventModifierFlags::Control)
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let shift = event.modifierFlags().contains(NSEventModifierFlags::Shift);
            let chars = event.charactersIgnoringModifiers().map(|c| c.to_string()).unwrap_or_default();
            let Some(key) = key_of(event.keyCode(), shift, &chars) else {
                objc2_app_kit::NSBeep();
                return;
            };
            if let Some(w) = self.ivars().window.load() {
                w.arrange_key(key);
            }
        }

        /// A click anywhere cancels.
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            if let Some(w) = self.ivars().window.load() {
                w.arrange_key(ArrangeKey::Cancel);
            }
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, _event: &NSEvent) {
            if let Some(w) = self.ivars().window.load() {
                w.arrange_key(ArrangeKey::Cancel);
            }
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, _event: &NSEvent) {}
    }

    unsafe impl NSObjectProtocol for ArrangeView {}
);

/// The mode's key for a key event: its code, Shift, and its characters
/// without modifiers.
pub(crate) fn key_of(code: u16, shift: bool, chars: &str) -> Option<ArrangeKey> {
    Some(match code {
        123 => ArrangeKey::Towards(Direction::Left),
        124 => ArrangeKey::Towards(Direction::Right),
        125 => ArrangeKey::Towards(Direction::Down),
        126 => ArrangeKey::Towards(Direction::Up),
        48 if shift => ArrangeKey::Previous,
        48 => ArrangeKey::Next,
        49 => ArrangeKey::Swap,
        36 | 76 => ArrangeKey::Place,
        53 => ArrangeKey::Cancel,
        _ => match chars.chars().next()? {
            c @ '1'..='9' => ArrangeKey::Pane(c as usize - '0' as usize),
            _ => return None,
        },
    })
}

fn label(mtm: MainThreadMarker, text: &str, size: f64, bold: bool) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    let font = if bold {
        NSFont::boldSystemFontOfSize(size)
    } else {
        NSFont::systemFontOfSize(size)
    };
    l.setFont(Some(&font));
    l.setTextColor(Some(&NSColor::whiteColor()));
    l.setAlignment(NSTextAlignment::Center);
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
    l
}

fn filled(mtm: MainThreadMarker, fill: &NSColor, border: Option<&NSColor>, radius: f64) -> Retained<NSBox> {
    let b = NSBox::new(mtm);
    b.setBoxType(NSBoxType::Custom);
    b.setTitlePosition(NSTitlePosition::NoTitle);
    b.setFillColor(fill);
    match border {
        Some(c) => {
            b.setBorderColor(c);
            b.setBorderWidth(1.0);
        }
        None => b.setBorderWidth(0.0),
    }
    b.setCornerRadius(radius);
    b.setContentViewMargins(NSSize::new(0.0, 0.0));
    b
}

impl ArrangeView {
    pub(crate) fn new(window: &WindowView) -> Retained<ArrangeView> {
        let mtm = MainThreadMarker::from(window);
        let this = Self::alloc(mtm).set_ivars(Ivars { window: Weak::new(window) });
        let this: Retained<ArrangeView> = unsafe { msg_send![super(this), initWithFrame: window.bounds()] };
        window.addSubview(&this);
        this
    }

    /// Show `outlines` over the dimmed window, `hint` in the bar at the
    /// bottom; `bounds` is the window view's.
    pub(crate) fn show(&self, bounds: NSRect, outlines: &[Outline], hint: &str) {
        let mtm = MainThreadMarker::from(self);
        self.setFrame(bounds);
        for v in self.subviews().iter() {
            v.removeFromSuperview();
        }
        let dim = filled(mtm, &NSColor::colorWithWhite_alpha(0.0, 0.45), None, 0.0);
        dim.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), bounds.size));
        self.addSubview(&dim);
        let accent = NSColor::controlAccentColor();
        let line = NSColor::colorWithWhite_alpha(1.0, 0.7);
        for o in outlines {
            let r = NSRect::new(
                NSPoint::new(o.rect.origin.x + 3.0, o.rect.origin.y + 3.0),
                NSSize::new((o.rect.size.width - 6.0).max(1.0), (o.rect.size.height - 6.0).max(1.0)),
            );
            let fill = match o.arranged {
                Some(true) => accent.colorWithAlphaComponent(0.35),
                Some(false) => accent.colorWithAlphaComponent(0.2),
                None => NSColor::colorWithWhite_alpha(0.0, 0.25),
            };
            let border = if o.arranged.is_some() { accent.clone() } else { line.clone() };
            let b = filled(mtm, &fill, Some(&*border), 6.0);
            b.setFrame(r);
            self.addSubview(&b);
            let l = label(mtm, &o.text, 13.0, o.arranged.is_some());
            let h = l.intrinsicContentSize().height;
            l.setFrame(NSRect::new(
                NSPoint::new(r.origin.x + 6.0, r.origin.y + ((r.size.height - h) / 2.0).max(0.0)),
                NSSize::new((r.size.width - 12.0).max(0.0), h),
            ));
            self.addSubview(&l);
        }
        let bar = filled(mtm, &NSColor::colorWithWhite_alpha(0.08, 0.9), None, 0.0);
        bar.setFrame(NSRect::new(
            NSPoint::new(0.0, (bounds.size.height - BAR).max(0.0)),
            NSSize::new(bounds.size.width, BAR),
        ));
        self.addSubview(&bar);
        let text = label(mtm, &format!("{hint}      {KEYS}"), 12.0, false);
        let h = text.intrinsicContentSize().height;
        text.setFrame(NSRect::new(
            NSPoint::new(10.0, (bounds.size.height - BAR + (BAR - h) / 2.0).max(0.0)),
            NSSize::new((bounds.size.width - 20.0).max(0.0), h),
        ));
        self.addSubview(&text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(key_of(124, false, ""), Some(ArrangeKey::Towards(Direction::Right)));
        assert_eq!(key_of(48, true, "\t"), Some(ArrangeKey::Previous));
        assert_eq!(key_of(48, false, "\t"), Some(ArrangeKey::Next));
        assert_eq!(key_of(36, false, "\r"), Some(ArrangeKey::Place));
        assert_eq!(key_of(53, false, "\u{1b}"), Some(ArrangeKey::Cancel));
        assert_eq!(key_of(18, false, "1"), Some(ArrangeKey::Pane(1)));
        assert_eq!(key_of(0, false, "a"), None);
    }

    #[test]
    fn leaves_under_a_node() {
        let root = Node::Split {
            dir: crate::panes::Dir::Right,
            ratio: 0.5,
            a: Box::new(Node::Leaf(1)),
            b: Box::new(Node::Split {
                dir: crate::panes::Dir::Down,
                ratio: 0.5,
                a: Box::new(Node::Leaf(2)),
                b: Box::new(Node::Leaf(3)),
            }),
        };
        assert_eq!(leaves_at(&root, &[]), [1, 2, 3]);
        assert_eq!(leaves_at(&root, &[true]), [2, 3]);
        assert_eq!(leaves_at(&root, &[true, false]), [2]);
        assert_eq!(leaves_at(&root, &[false, true]), Vec::<PaneId>::new());
    }
}
