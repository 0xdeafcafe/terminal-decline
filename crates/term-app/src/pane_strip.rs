//! A pane's title strip (pane-arrange design §2): one line across the
//! top of each pane while the window shows two or more, or while macOS
//! shows no title bar of its own (full screen), saying the pane's name or
//! what it is doing. A band in the theme's colours (the background
//! tinted towards the foreground, a hairline in the divider colour under
//! it) with an `NSTextField` label, in a plain view, a subview of the
//! pane's view as the find bar is: the Metal path doesn't change. A click
//! focuses the pane; a double-click renames it.

use std::cell::Cell;

use objc2::rc::{Retained, Weak};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSBox, NSBoxType, NSColor, NSEvent, NSFont, NSLineBreakMode, NSResponder, NSTextField, NSTitlePosition, NSView};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use crate::pane_view::PaneView;

/// The strip's height, points.
pub const HEIGHT: f64 = 20.0;
/// The label's inset from the strip's left and right, points.
const INSET: f64 = 10.0;
/// How far the band is tinted from the theme's background towards its
/// foreground: the focused pane's, the others'.
const TINT_FOCUSED: f32 = 0.09;
const TINT: f32 = 0.04;
/// How far the others' text is from the background to the foreground.
const TEXT_FAINT: f32 = 0.5;

/// The colours a strip is drawn in, 0xRRGGBB, and the background's
/// opacity (0–255, as the theme's).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Look {
    pub foreground: u32,
    pub background: u32,
    pub divider: u32,
    pub opacity: u8,
}

pub struct Ivars {
    pane: Weak<PaneView>,
    label: Retained<NSTextField>,
    band: Retained<NSBox>,
    rule: Retained<NSBox>,
    /// What it was last drawn in, and whether focused.
    drawn: Cell<Option<(Look, bool)>>,
}

define_class!(
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeclinePaneStrip"]
    #[ivars = Ivars]
    pub struct PaneStrip;

    impl PaneStrip {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        /// The label is for reading only: clicks on it are the strip's.
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
        fn mouse_down(&self, event: &NSEvent) {
            let Some(pane) = self.ivars().pane.load() else { return };
            if event.clickCount() >= 2 {
                pane.rename();
            } else {
                pane.focus_from_strip();
            }
        }
    }

    unsafe impl NSObjectProtocol for PaneStrip {}
);

impl PaneStrip {
    /// A hidden strip on `pane`.
    pub fn new(pane: &PaneView) -> Retained<PaneStrip> {
        let mtm = MainThreadMarker::from(pane);
        let label = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        label.setFont(Some(&NSFont::systemFontOfSize(NSFont::smallSystemFontSize())));
        label.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        let band = plain_box(mtm);
        let rule = plain_box(mtm);
        let this = Self::alloc(mtm).set_ivars(Ivars {
            pane: Weak::new(pane),
            label: label.clone(),
            band: band.clone(),
            rule: rule.clone(),
            drawn: Cell::new(None),
        });
        let this: Retained<PaneStrip> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.addSubview(&band);
        this.addSubview(&rule);
        this.addSubview(&label);
        this.setHidden(true);
        pane.addSubview(&this);
        this
    }

    /// Show it at `top` points down the pane, `width` wide, saying `text`
    /// in `look`'s colours: the focused pane's band more tinted and its
    /// text in the foreground, the others' fainter.
    pub fn show(&self, width: f64, top: f64, text: &str, focused: bool, look: Look) {
        self.setFrame(NSRect::new(NSPoint::new(0.0, top), NSSize::new(width, HEIGHT)));
        let iv = self.ivars();
        iv.band
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, HEIGHT - 1.0)));
        iv.rule
            .setFrame(NSRect::new(NSPoint::new(0.0, HEIGHT - 1.0), NSSize::new(width, 1.0)));
        let label = &iv.label;
        let h = label.intrinsicContentSize().height.min(HEIGHT - 1.0);
        label.setFrame(NSRect::new(
            NSPoint::new(INSET, ((HEIGHT - 1.0 - h) / 2.0).round()),
            NSSize::new((width - 2.0 * INSET).max(0.0), h),
        ));
        if label.stringValue().to_string() != text {
            label.setStringValue(&NSString::from_str(text));
        }
        if iv.drawn.get() != Some((look, focused)) {
            iv.drawn.set(Some((look, focused)));
            let (band, rule, ink) = colours(look, focused);
            let alpha = look.opacity as f64 / 255.0;
            iv.band.setFillColor(&ns_colour(band, alpha));
            iv.rule.setFillColor(&ns_colour(rule, 1.0));
            label.setTextColor(Some(&ns_colour(ink, 1.0)));
        }
        if self.isHidden() {
            self.setHidden(false);
        }
    }

    pub fn hide(&self) {
        if !self.isHidden() {
            self.setHidden(true);
        }
    }

    /// What it says (for the test dumps).
    pub fn text(&self) -> Option<String> {
        (!self.isHidden()).then(|| self.ivars().label.stringValue().to_string())
    }
}

/// An `NSBox` that only fills.
fn plain_box(mtm: MainThreadMarker) -> Retained<NSBox> {
    let b = NSBox::new(mtm);
    b.setBoxType(NSBoxType::Custom);
    b.setTitlePosition(NSTitlePosition::NoTitle);
    b.setBorderWidth(0.0);
    b
}

/// 0xRRGGBB as an sRGB colour.
fn ns_colour(rgb: u32, alpha: f64) -> Retained<NSColor> {
    let c = |s: u32| ((rgb >> s) & 0xFF) as f64 / 255.0;
    NSColor::colorWithSRGBRed_green_blue_alpha(c(16), c(8), c(0), alpha)
}

/// `a` (0xRRGGBB) moved `t` of the way to `b`.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xFF) as f32, ((b >> shift) & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << shift
    };
    ch(16) | ch(8) | ch(0)
}

/// The band's, the hairline's and the text's colours, 0xRRGGBB.
fn colours(look: Look, focused: bool) -> (u32, u32, u32) {
    let (fg, bg) = (look.foreground, look.background);
    let band = mix(bg, fg, if focused { TINT_FOCUSED } else { TINT });
    let ink = if focused { fg } else { mix(bg, fg, TEXT_FAINT) };
    (band, look.divider, ink)
}

/// What a pane's strip says: its name if it has one, else the window
/// title it would give with focus, without the app's name.
pub fn title_text(name: Option<&str>, title: &str) -> String {
    if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
        return n.trim().to_string();
    }
    let suffix = format!(" — {}", crate::PRODUCT_NAME);
    title.strip_suffix(&suffix).unwrap_or(title).to_string()
}

/// Whether panes show strips: several shown, or macOS shows no title
/// bar (`top_inset` 0, as in full screen).
pub fn shown(panes_shown: usize, top_inset: f64) -> bool {
    panes_shown > 1 || top_inset <= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_the_name_or_what_the_pane_does() {
        let app = crate::PRODUCT_NAME;
        assert_eq!(title_text(None, &format!("cargo test · 12s — {app}")), "cargo test · 12s");
        assert_eq!(title_text(None, "nvim"), "nvim");
        assert_eq!(title_text(Some(" logs "), "nvim"), "logs");
        assert_eq!(title_text(Some("  "), "nvim"), "nvim");
    }

    #[test]
    fn the_band_is_tinted_from_the_theme() {
        let look = Look {
            foreground: 0xFFFFFF,
            background: 0x000000,
            divider: 0x333333,
            opacity: 255,
        };
        let (band, rule, ink) = colours(look, true);
        assert_eq!(ink, 0xFFFFFF, "the focused pane's text is the foreground");
        assert_eq!(rule, 0x333333);
        assert!(band > 0 && band < 0x404040, "a little lighter than the background: {band:06x}");
        let (faint_band, _, faint_ink) = colours(look, false);
        assert!(faint_band < band, "the others' band is less tinted");
        assert_eq!(faint_ink, 0x808080, "the others' text is halfway");
    }

    #[test]
    fn strips_show_with_several_panes_or_no_title_bar() {
        assert!(!shown(1, 28.0), "one pane under the title bar: the title bar says it");
        assert!(shown(1, 0.0), "full screen");
        assert!(shown(2, 28.0));
    }
}
