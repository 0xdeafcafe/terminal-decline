//! A pane's title strip (pane-arrange design §2): one line across the
//! top of each pane while the window shows two or more, or while macOS
//! shows no title bar of its own (full screen), saying the pane's name or
//! what it is doing. A standard `NSTextField` label in a plain view, a
//! subview of the pane's view as the find bar is: the Metal path doesn't
//! change. A click focuses the pane; a double-click renames it.

use objc2::rc::{Retained, Weak};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSColor, NSEvent, NSFont, NSLineBreakMode, NSResponder, NSTextField, NSView};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use crate::pane_view::PaneView;

/// The strip's height, points.
pub const HEIGHT: f64 = 20.0;
/// The label's inset from the strip's left and right, points.
const INSET: f64 = 8.0;

pub struct Ivars {
    pane: Weak<PaneView>,
    label: Retained<NSTextField>,
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
        label.setTextColor(Some(&NSColor::labelColor()));
        let this = Self::alloc(mtm).set_ivars(Ivars {
            pane: Weak::new(pane),
            label: label.clone(),
        });
        let this: Retained<PaneStrip> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.addSubview(&label);
        this.setHidden(true);
        pane.addSubview(&this);
        this
    }

    /// Show it at `top` points down the pane, `width` wide, saying `text`;
    /// the focused pane's in the label colour, the others' fainter.
    pub fn show(&self, width: f64, top: f64, text: &str, focused: bool) {
        self.setFrame(NSRect::new(NSPoint::new(0.0, top), NSSize::new(width, HEIGHT)));
        let label = &self.ivars().label;
        let h = label.intrinsicContentSize().height.min(HEIGHT);
        label.setFrame(NSRect::new(
            NSPoint::new(INSET, ((HEIGHT - h) / 2.0).floor()),
            NSSize::new((width - 2.0 * INSET).max(0.0), h),
        ));
        if label.stringValue().to_string() != text {
            label.setStringValue(&NSString::from_str(text));
        }
        label.setAlphaValue(if focused { 1.0 } else { 0.55 });
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
    fn strips_show_with_several_panes_or_no_title_bar() {
        assert!(!shown(1, 28.0), "one pane under the title bar: the title bar says it");
        assert!(shown(1, 0.0), "full screen");
        assert!(shown(2, 28.0));
    }
}
