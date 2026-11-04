//! What says a pane's text size (pane-arrange design §1): a badge at its
//! bottom right for a moment after Cmd+= / Cmd+- (`16 pt  (+2)`), and,
//! while its size isn't the config's, a faint `+2` in its bottom-right
//! corner. Standard AppKit views over the pane, as the find bar is: the
//! Metal path doesn't change. Labels pass clicks on to the pane.

use std::cell::Cell;
use std::time::Duration;

use dispatch2::{DispatchQueue, DispatchTime};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSBox, NSBoxType, NSColor, NSFont, NSTextAlignment, NSTextField, NSTitlePosition,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::pane_view::PaneView;
use crate::panes::PaneId;

/// How long the badge shows before it fades.
const SHOWN: Duration = Duration::from_millis(1000);
/// How long it takes to fade.
const FADE: f64 = 0.25;
/// From the pane's edges, points.
const MARGIN: f64 = 6.0;
const BADGE: NSSize = NSSize::new(104.0, 24.0);
const CORNER: NSSize = NSSize::new(44.0, 14.0);

/// A pane's two size labels.
pub(crate) struct SizeLabels {
    badge: Retained<NSBox>,
    badge_text: Retained<NSTextField>,
    corner: Retained<NSTextField>,
    /// Bumped each time the badge shows, so only the last fade runs.
    shown: Cell<u64>,
}

/// `+2`, `−1.5`: how far `d` points is from the default; empty at 0.
pub(crate) fn difference_text(d: f64) -> String {
    let d = (d * 10.0).round() / 10.0;
    if d == 0.0 {
        return String::new();
    }
    let sign = if d > 0.0 { "+" } else { "\u{2212}" };
    let n = d.abs();
    if n.fract() == 0.0 {
        format!("{sign}{n:.0}")
    } else {
        format!("{sign}{n:.1}")
    }
}

/// `16 pt  (+2)`, or `13 pt` at the default.
pub(crate) fn badge_text(size: f64, d: f64) -> String {
    let size = (size * 10.0).round() / 10.0;
    let size = if size.fract() == 0.0 {
        format!("{size:.0}")
    } else {
        format!("{size:.1}")
    };
    match difference_text(d) {
        t if t.is_empty() => format!("{size} pt"),
        t => format!("{size} pt  ({t})"),
    }
}

impl SizeLabels {
    /// Labels for `pane`, whose size is `difference` points from the
    /// default; the badge hidden.
    pub(crate) fn new(pane: &PaneView, difference: f64) -> SizeLabels {
        let mtm = MainThreadMarker::from(pane);
        let badge_text = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        badge_text.setFont(Some(&NSFont::boldSystemFontOfSize(12.0)));
        badge_text.setTextColor(Some(&NSColor::whiteColor()));
        badge_text.setAlignment(NSTextAlignment::Center);
        let badge = NSBox::new(mtm);
        badge.setBoxType(NSBoxType::Custom);
        badge.setTitlePosition(NSTitlePosition::NoTitle);
        badge.setBorderWidth(0.0);
        badge.setCornerRadius(7.0);
        badge.setFillColor(&NSColor::colorWithWhite_alpha(0.1, 0.72));
        badge.setContentViewMargins(NSSize::new(0.0, 4.0));
        badge.setContentView(Some(&badge_text));
        badge.setHidden(true);
        let corner = NSTextField::labelWithString(&NSString::from_str(&difference_text(difference)), mtm);
        corner.setFont(Some(&NSFont::systemFontOfSize(10.0)));
        corner.setTextColor(Some(&NSColor::labelColor()));
        corner.setAlignment(NSTextAlignment::Right);
        corner.setAlphaValue(0.35);
        corner.setHidden(difference_text(difference).is_empty());
        pane.addSubview(&corner);
        pane.addSubview(&badge);
        SizeLabels {
            badge,
            badge_text,
            corner,
            shown: Cell::new(0),
        }
    }

    /// Put them at the bottom right of a pane `size` points big (its view
    /// is flipped: y down).
    pub(crate) fn place(&self, size: NSSize) {
        let at = |s: NSSize| {
            NSRect::new(
                NSPoint::new((size.width - s.width - MARGIN).max(0.0), (size.height - s.height - MARGIN).max(0.0)),
                s,
            )
        };
        self.badge.setFrame(at(BADGE));
        let corner = NSRect::new(
            NSPoint::new(
                (size.width - CORNER.width - 3.0).max(0.0),
                (size.height - CORNER.height - 2.0).max(0.0),
            ),
            CORNER,
        );
        self.corner.setFrame(corner);
    }

    /// The faint label for a size `difference` points from the default.
    pub(crate) fn set_difference(&self, difference: f64) {
        let text = difference_text(difference);
        self.corner.setStringValue(&NSString::from_str(&text));
        self.corner.setHidden(text.is_empty());
    }

    /// The size became `size`, `difference` from the default: the badge
    /// shows it for a moment, then fades (pane `pane` of window `window`
    /// is asked to, by id, from the main queue).
    pub(crate) fn changed(&self, size: f64, difference: f64, window: usize, pane: PaneId) {
        self.set_difference(difference);
        self.badge_text.setStringValue(&NSString::from_str(&badge_text(size, difference)));
        self.badge.setAlphaValue(1.0);
        self.badge.setHidden(false);
        let n = self.shown.get() + 1;
        self.shown.set(n);
        let _ = DispatchQueue::main().after(DispatchTime::NOW.time(SHOWN.as_nanos() as i64), move || {
            crate::view::with_view(window, |v| v.fade_size_badge(pane, n));
        });
    }

    /// Fade the badge, unless it was shown again since showing `n`th.
    pub(crate) fn fade(&self, n: u64) {
        if self.shown.get() != n {
            return; // shown again since: its own fade comes later
        }
        let fading = self.badge.clone();
        let changes = block2::RcBlock::new(move |context: std::ptr::NonNull<NSAnimationContext>| {
            unsafe { context.as_ref() }.setDuration(FADE);
            fading.animator().setAlphaValue(0.0);
        });
        let badge = self.badge.clone();
        let done = block2::RcBlock::new(move || {
            // Shown again while fading: it stays.
            if badge.alphaValue() == 0.0 {
                badge.setHidden(true);
            }
        });
        NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&done));
    }

    /// Whether the badge shows (for the test dumps).
    #[allow(dead_code)]
    pub(crate) fn badge_shown(&self) -> bool {
        !self.badge.isHidden()
    }
}

impl Drop for SizeLabels {
    fn drop(&mut self) {
        self.badge.removeFromSuperview();
        self.corner.removeFromSuperview();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts() {
        assert_eq!(difference_text(0.0), "");
        assert_eq!(difference_text(2.0), "+2");
        assert_eq!(difference_text(-1.0), "\u{2212}1");
        assert_eq!(difference_text(1.5), "+1.5");
        assert_eq!(difference_text(1e-9), "");
        assert_eq!(badge_text(16.0, 2.0), "16 pt  (+2)");
        assert_eq!(badge_text(13.0, 0.0), "13 pt");
        assert_eq!(badge_text(12.5, -0.5), "12.5 pt  (\u{2212}0.5)");
    }
}
