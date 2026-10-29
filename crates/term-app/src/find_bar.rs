//! The find bar (roadmap E.7, cells design §13.9): decline's one piece of
//! its own UI, by Alex's decision, made only of standard AppKit controls
//! so it looks and behaves like the system's. It sits over the top rows of
//! its pane, below the title bar (covering them rather than making the
//! grid shorter, so the program isn't resized):
//!
//! `[ search field (options on its magnifying glass) ] 3 of 120 [<][>] Done`
//!
//! The field searches as you type, after its own short delay; Return finds
//! the next match, Shift+Return the previous, Esc and Done close the bar.
//! The bar is a subview of its pane's view, so the menus' Find Next and
//! Find Previous reach the pane through the responder chain while the
//! field has the keys. The search itself is the pane's [`Search`].

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;

use block2::RcBlock;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, Bool, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSAutoresizingMaskOptions, NSBox, NSBoxType, NSButton, NSColor, NSControl, NSControlSize, NSControlTextEditingDelegate,
    NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSFont, NSImage, NSMenu, NSMenuItem, NSPasteboard, NSPasteboardNameFind,
    NSPasteboardTypeString, NSPasteboardTypeTextFinderOptions, NSResponder, NSSearchField, NSSearchFieldDelegate, NSSegmentSwitchTracking,
    NSSegmentedControl, NSTextAlignment, NSTextField, NSTextFieldDelegate, NSTextFinderCaseInsensitiveKey, NSTextFinderMatchingTypeKey,
    NSTextView, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectView,
};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use crate::pane_view::PaneView;
use crate::search::{Query, Search, Status, WordMatch};

/// The bar's height, points.
pub const HEIGHT: f64 = 32.0;
const MARGIN: f64 = 8.0;
const GAP: f64 = 6.0;
const COUNT_WIDTH: f64 = 92.0;
const DONE_WIDTH: f64 = 58.0;
const ARROWS_WIDTH: f64 = 54.0;

/// The Escape key's code.
const ESC: u16 = 53;

/// The options menu's tags.
const IGNORE_CASE: isize = 100;

/// A pane's find bar and its search, kept in the pane while it is open.
pub(crate) struct Find {
    pub(crate) bar: Retained<FindBar>,
    pub(crate) search: Box<dyn Search>,
    /// Search highlights in the last snapshot (for the test dumps).
    pub(crate) drawn: usize,
    /// The count as last shown.
    pub(crate) shown: Status,
    /// The terminal (its id, and whether it shows the alternate screen)
    /// the search was started on; None before the first search.
    pub(crate) seen: Option<(u64, bool)>,
}

impl Find {
    pub(crate) fn new(bar: Retained<FindBar>, search: Box<dyn Search>) -> Find {
        Find {
            bar,
            search,
            drawn: 0,
            shown: Status::default(),
            seen: None,
        }
    }
}

pub struct Ivars {
    pane: Weak<PaneView>,
    field: Retained<NSSearchField>,
    count: Retained<NSTextField>,
    arrows: Retained<NSSegmentedControl>,
    done: Retained<NSButton>,
    line: Retained<NSBox>,
    ignore_case: Cell<bool>,
    word: Cell<WordMatch>,
    /// Takes Esc while the field has the keys (see [`FindBar::watch_esc`]).
    monitor: RefCell<Option<Retained<AnyObject>>>,
}

define_class!(
    #[unsafe(super(NSVisualEffectView, NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeclineFindBar"]
    #[ivars = Ivars]
    pub struct FindBar;

    impl FindBar {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.lay_out();
        }

        /// The field's action: as you type (after its delay), and Return.
        #[unsafe(method(search:))]
        fn search(&self, _sender: Option<&AnyObject>) {
            self.query_changed();
        }

        /// The previous/next arrows.
        #[unsafe(method(step:))]
        fn step(&self, sender: Option<&AnyObject>) {
            let forward = sender
                .and_then(|s| s.downcast_ref::<NSSegmentedControl>())
                .is_none_or(|s| s.selectedSegment() == 1);
            if let Some(p) = self.ivars().pane.load() {
                p.find_step(forward);
            }
        }

        #[unsafe(method(done:))]
        fn done(&self, _sender: Option<&AnyObject>) {
            if let Some(p) = self.ivars().pane.load() {
                p.close_find();
            }
        }

        #[unsafe(method(toggleIgnoreCase:))]
        fn toggle_ignore_case(&self, _sender: Option<&AnyObject>) {
            let i = &self.ivars().ignore_case;
            i.set(!i.get());
            self.query_changed();
        }

        #[unsafe(method(setWordMatch:))]
        fn set_word_match(&self, sender: Option<&NSMenuItem>) {
            let word = match sender.map_or(0, |s| s.tag()) {
                1 => WordMatch::StartsWith,
                2 => WordMatch::FullWord,
                _ => WordMatch::Contains,
            };
            self.ivars().word.set(word);
            self.query_changed();
        }

        /// The options menu's checkmarks.
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            let on = match item.tag() {
                IGNORE_CASE => self.ivars().ignore_case.get(),
                t => word_tag(self.ivars().word.get()) == t,
            };
            item.setState(if on {
                objc2_app_kit::NSControlStateValueOn
            } else {
                objc2_app_kit::NSControlStateValueOff
            });
            true
        }
    }

    unsafe impl NSObjectProtocol for FindBar {}

    unsafe impl NSControlTextEditingDelegate for FindBar {
        /// Return and Shift+Return step through the matches; Esc closes
        /// the bar (the field would only clear itself).
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, _control: &NSControl, _text_view: &NSTextView, command: Sel) -> Bool {
            Bool::new(self.text_command(command))
        }
    }

    unsafe impl NSTextFieldDelegate for FindBar {}
    unsafe impl NSSearchFieldDelegate for FindBar {}
);

impl FindBar {
    /// A command from the field's editor: true if the bar did it.
    fn text_command(&self, command: Sel) -> bool {
        {
            let Some(p) = self.ivars().pane.load() else { return false };
            if command == sel!(insertNewline:) {
                let mtm = MainThreadMarker::from(self);
                let shift = NSApplication::sharedApplication(mtm)
                    .currentEvent()
                    .is_some_and(|e| e.modifierFlags().contains(NSEventModifierFlags::Shift));
                // Return searches now if the delay hasn't run out yet.
                if p.find_query_differs(&self.query()) {
                    self.query_changed();
                } else {
                    p.find_step(!shift);
                }
                return true;
            }
            if command == sel!(cancelOperation:) {
                p.close_find();
                return true;
            }
            false
        }
    }
}

fn word_tag(word: WordMatch) -> isize {
    match word {
        WordMatch::Contains => 0,
        WordMatch::StartsWith => 1,
        WordMatch::FullWord => 2,
    }
}

impl FindBar {
    /// A bar for `pane`, showing `query`.
    pub(crate) fn new(mtm: MainThreadMarker, pane: &PaneView, query: &Query) -> Retained<FindBar> {
        let field = NSSearchField::new(mtm);
        field.setPlaceholderString(Some(&NSString::from_str("Find")));
        field.setStringValue(&NSString::from_str(&query.text));
        // Search as you type, after the field's short delay.
        field.setSendsWholeSearchString(false);
        field.setSendsSearchStringImmediately(false);
        let count = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        count.setTextColor(Some(&NSColor::secondaryLabelColor()));
        count.setFont(Some(&NSFont::systemFontOfSize(NSFont::smallSystemFontSize())));
        count.setAlignment(NSTextAlignment::Right);
        let image = |name: &str, label: &str| {
            NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), Some(&NSString::from_str(label)))
                .unwrap_or_default()
        };
        let images = NSArray::from_retained_slice(&[image("chevron.left", "Previous"), image("chevron.right", "Next")]);
        let arrows = unsafe {
            NSSegmentedControl::segmentedControlWithImages_trackingMode_target_action(
                &images,
                NSSegmentSwitchTracking::Momentary,
                None,
                None,
                mtm,
            )
        };
        arrows.setControlSize(NSControlSize::Small);
        let done = unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str("Done"), None, None, mtm) };
        done.setControlSize(NSControlSize::Small);
        let line = NSBox::new(mtm);
        line.setBoxType(NSBoxType::Separator);
        let this = Self::alloc(mtm).set_ivars(Ivars {
            pane: Weak::new(pane),
            field,
            count,
            arrows,
            done,
            line,
            ignore_case: Cell::new(query.ignore_case),
            word: Cell::new(query.word),
            monitor: RefCell::new(None),
        });
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(pane.frame().size.width, HEIGHT));
        let bar: Retained<FindBar> = unsafe { msg_send![super(this), initWithFrame: frame] };
        bar.setMaterial(NSVisualEffectMaterial::HeaderView);
        bar.setBlendingMode(NSVisualEffectBlendingMode::WithinWindow);
        bar.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        let iv = bar.ivars();
        let target: &AnyObject = &bar;
        unsafe {
            iv.field.setTarget(Some(target));
            iv.field.setAction(Some(sel!(search:)));
            iv.field.setDelegate(Some(ProtocolObject::from_ref(&*bar)));
            iv.arrows.setTarget(Some(target));
            iv.arrows.setAction(Some(sel!(step:)));
            iv.done.setTarget(Some(target));
            iv.done.setAction(Some(sel!(done:)));
        }
        iv.field.setSearchMenuTemplate(Some(&bar.options_menu(mtm)));
        for v in [
            &**iv.field as &NSView,
            &**iv.count as &NSView,
            &**iv.arrows as &NSView,
            &**iv.done as &NSView,
            &**iv.line as &NSView,
        ] {
            bar.addSubview(v);
        }
        bar.lay_out();
        bar.watch_esc();
        bar
    }

    /// NSSearchField takes Esc itself, before its delegate hears of it,
    /// and clears its text: while the field has the keys, plain Esc closes
    /// the bar instead, as in Safari and Xcode.
    fn watch_esc(&self) {
        let bar = Weak::new(self);
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            let e = unsafe { event.as_ref() };
            if e.r#type() == NSEventType::LeftMouseDown {
                // A click in the bar focuses its pane (the keys stay in
                // the field).
                if let Some(bar) = bar.load()
                    && bar.contains_click(e)
                    && let Some(p) = bar.ivars().pane.load()
                {
                    p.bar_clicked();
                }
                return event.as_ptr();
            }
            let plain = !e.modifierFlags().intersects(
                NSEventModifierFlags::Command | NSEventModifierFlags::Control | NSEventModifierFlags::Option | NSEventModifierFlags::Shift,
            );
            if e.keyCode() == ESC
                && plain
                && let Some(bar) = bar.load()
                && bar.has_focus()
                && let Some(p) = bar.ivars().pane.load()
            {
                p.close_find();
                return std::ptr::null_mut();
            }
            event.as_ptr()
        });
        let monitor = crate::background::add_local_monitor(NSEventMask::KeyDown | NSEventMask::LeftMouseDown, handler);
        *self.ivars().monitor.borrow_mut() = monitor;
    }

    /// Whether the mouse-down `e` is on the bar.
    fn contains_click(&self, e: &NSEvent) -> bool {
        if e.window(MainThreadMarker::from(self))
            .is_none_or(|w| self.window().is_none_or(|mine| !std::ptr::eq(&*w, &*mine)))
        {
            return false;
        }
        let at = self.convertPoint_fromView(e.locationInWindow(), None);
        let size = self.bounds().size;
        at.x >= 0.0 && at.y >= 0.0 && at.x < size.width && at.y < size.height
    }

    /// The test control's click: a left click on the search field, put on
    /// the event queue as a real one arrives (the down first, so the
    /// field's own tracking finds the up waiting).
    pub(crate) fn post_click(&self) {
        let Some(window) = self.window() else { return };
        let f = self.ivars().field.frame();
        let at = self.convertPoint_toView(
            NSPoint::new(f.origin.x + f.size.width / 2.0, f.origin.y + f.size.height / 2.0),
            None,
        );
        let app = NSApplication::sharedApplication(MainThreadMarker::from(self));
        for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
            let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                kind,
                at,
                NSEventModifierFlags::empty(),
                0.0,
                window.windowNumber(),
                None,
                0,
                1,
                1.0,
            );
            if let Some(event) = event {
                app.postEvent_atStart(&event, false);
            }
        }
    }

    /// The bar is closing: stop watching for Esc.
    pub(crate) fn stop(&self) {
        if let Some(m) = self.ivars().monitor.borrow_mut().take() {
            crate::background::remove_monitor(&m);
        }
    }

    /// Ignore Case, then Contains / Starts With / Full Word.
    fn options_menu(&self, mtm: MainThreadMarker) -> Retained<NSMenu> {
        let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Options"));
        let add = |title: &str, action: Sel, tag: isize| {
            let it = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    Some(action),
                    &NSString::from_str(""),
                )
            };
            it.setTag(tag);
            unsafe { it.setTarget(Some(self)) };
            menu.addItem(&it);
        };
        add("Ignore Case", sel!(toggleIgnoreCase:), IGNORE_CASE);
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        add("Contains", sel!(setWordMatch:), 0);
        add("Starts With", sel!(setWordMatch:), 1);
        add("Full Word", sel!(setWordMatch:), 2);
        menu
    }

    fn lay_out(&self) {
        let iv = self.ivars();
        let w = self.frame().size.width;
        let mid = |h: f64| ((HEIGHT - h) / 2.0).round();
        let right = w - MARGIN;
        let done_x = right - DONE_WIDTH;
        let arrows_x = done_x - GAP - ARROWS_WIDTH;
        let count_x = arrows_x - GAP - COUNT_WIDTH;
        let field_w = (count_x - GAP - MARGIN).max(60.0);
        iv.field
            .setFrame(NSRect::new(NSPoint::new(MARGIN, mid(22.0)), NSSize::new(field_w, 22.0)));
        iv.count
            .setFrame(NSRect::new(NSPoint::new(count_x, mid(16.0)), NSSize::new(COUNT_WIDTH, 16.0)));
        iv.arrows
            .setFrame(NSRect::new(NSPoint::new(arrows_x, mid(20.0)), NSSize::new(ARROWS_WIDTH, 20.0)));
        iv.done
            .setFrame(NSRect::new(NSPoint::new(done_x, mid(22.0)), NSSize::new(DONE_WIDTH, 22.0)));
        iv.line.setFrame(NSRect::new(NSPoint::new(0.0, HEIGHT - 1.0), NSSize::new(w, 1.0)));
    }

    /// Put the bar at `top` (points down from its pane view's top), as
    /// wide as the pane.
    pub(crate) fn place(&self, width: f64, top: f64) {
        self.setFrame(NSRect::new(NSPoint::new(0.0, top), NSSize::new(width, HEIGHT)));
    }

    /// What the bar asks for now.
    pub(crate) fn query(&self) -> Query {
        let iv = self.ivars();
        Query {
            text: iv.field.stringValue().to_string(),
            ignore_case: iv.ignore_case.get(),
            word: iv.word.get(),
        }
    }

    /// Give the field the keys, its text selected.
    pub(crate) fn focus(&self) {
        if let Some(w) = self.window() {
            let field: &NSResponder = &self.ivars().field;
            w.makeFirstResponder(Some(field));
            unsafe { self.ivars().field.selectText(None) };
        }
    }

    /// Whether the field is being edited (has the keys).
    pub(crate) fn has_focus(&self) -> bool {
        self.ivars().field.currentEditor().is_some()
    }

    /// Set the field's text and options (Use Selection for Find, the
    /// find pasteboard, the test control).
    pub(crate) fn set_query(&self, q: &Query) {
        let iv = self.ivars();
        iv.field.setStringValue(&NSString::from_str(&q.text));
        iv.ignore_case.set(q.ignore_case);
        iv.word.set(q.word);
    }

    /// Show `status`: "3 of 120".
    pub(crate) fn show_status(&self, status: Status) {
        let searched = !self.ivars().field.stringValue().is_empty();
        self.ivars().count.setStringValue(&NSString::from_str(&status.text(searched)));
    }

    /// The count as shown (for the test dumps).
    pub(crate) fn count_text(&self) -> String {
        self.ivars().count.stringValue().to_string()
    }

    fn query_changed(&self) {
        if let Some(p) = self.ivars().pane.load() {
            p.find_query_changed(self.query());
        }
    }
}

/// The shared find text and options (the find pasteboard), as Safari,
/// Xcode and the rest use them. Not under the test control, so smoke
/// tests don't read or change the user's.
pub(crate) fn shared_query() -> Option<Query> {
    if std::env::var_os(crate::control::ENV).is_some() {
        return None;
    }
    read_query(&NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameFind }))
}

/// Make `q` the shared find text and options.
pub(crate) fn share_query(q: &Query) {
    if std::env::var_os(crate::control::ENV).is_some() {
        return;
    }
    let pb = NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameFind });
    // Leave it alone if it already says this (its change count is how
    // other apps know it changed).
    if read_query(&pb).as_ref() != Some(q) {
        write_query(&pb, q);
    }
}

/// NSTextFinderMatchingType's values, as apps write them.
fn matching_type(word: WordMatch) -> isize {
    match word {
        WordMatch::Contains => 0,
        WordMatch::StartsWith => 1,
        WordMatch::FullWord => 2,
    }
}

/// `pb`'s find text, with the options of its text finder dictionary when
/// it has one (Ends With, which the bar doesn't have, reads as Contains).
fn read_query(pb: &NSPasteboard) -> Option<Query> {
    let text = pb.stringForType(unsafe { NSPasteboardTypeString })?.to_string();
    let mut q = Query { text, ..Query::default() };
    let options = pb.propertyListForType(unsafe { NSPasteboardTypeTextFinderOptions });
    if let Some(dict) = options.as_deref().and_then(|o| o.downcast_ref::<NSDictionary>()) {
        let number = |key: &NSString| {
            dict.objectForKey(key)
                .and_then(|v| v.downcast_ref::<NSNumber>().map(|n| n.as_isize()))
        };
        if let Some(i) = number(unsafe { NSTextFinderCaseInsensitiveKey }) {
            q.ignore_case = i != 0;
        }
        q.word = match number(unsafe { NSTextFinderMatchingTypeKey }) {
            Some(1) => WordMatch::StartsWith,
            Some(2) => WordMatch::FullWord,
            _ => WordMatch::Contains,
        };
    }
    Some(q)
}

/// Put `q`'s text and options on `pb`.
fn write_query(pb: &NSPasteboard, q: &Query) {
    pb.clearContents();
    pb.setString_forType(&NSString::from_str(&q.text), unsafe { NSPasteboardTypeString });
    let ignore_case = NSNumber::new_bool(q.ignore_case);
    let matching = NSNumber::new_isize(matching_type(q.word));
    let options: Retained<NSDictionary<NSString, AnyObject>> = unsafe {
        NSDictionary::from_slices(
            &[NSTextFinderCaseInsensitiveKey, NSTextFinderMatchingTypeKey],
            &[&*ignore_case as &AnyObject, &*matching as &AnyObject],
        )
    };
    unsafe { pb.setPropertyList_forType(&options, NSPasteboardTypeTextFinderOptions) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_roundtrip_through_the_pasteboard() {
        let pb = NSPasteboard::pasteboardWithUniqueName();
        for (ignore_case, word) in [
            (true, WordMatch::FullWord),
            (false, WordMatch::StartsWith),
            (true, WordMatch::Contains),
        ] {
            let q = Query {
                text: "needle".into(),
                ignore_case,
                word,
            };
            write_query(&pb, &q);
            assert_eq!(read_query(&pb), Some(q));
        }
        // Another app's text with no options reads with the defaults.
        pb.clearContents();
        pb.setString_forType(&NSString::from_str("plain"), unsafe { NSPasteboardTypeString });
        assert_eq!(
            read_query(&pb),
            Some(Query {
                text: "plain".into(),
                ..Query::default()
            })
        );
    }
}
