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
use std::ops::Range;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, Bool, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSAutoresizingMaskOptions, NSBox, NSBoxType, NSButton, NSColor, NSControl, NSControlSize, NSControlTextEditingDelegate,
    NSEvent, NSEventMask, NSEventModifierFlags, NSFont, NSImage, NSMenu, NSMenuItem, NSPasteboard, NSPasteboardNameFind,
    NSPasteboardTypeString, NSResponder, NSSearchField, NSSearchFieldDelegate, NSSegmentSwitchTracking, NSSegmentedControl,
    NSTextAlignment, NSTextField, NSTextFieldDelegate, NSTextView, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectView,
};
use objc2_foundation::{NSArray, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use term_core::HighlightKind;

use crate::pane_view::PaneView;
use crate::search::{Query, Search, Status, TextSearch, WordMatch};

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
    /// The hits in view, as last taken for the snapshot.
    pub(crate) marks: Vec<(u16, Range<u16>, HighlightKind)>,
}

impl Find {
    pub(crate) fn new(bar: Retained<FindBar>) -> Find {
        Find {
            bar,
            search: Box::new(TextSearch::new()),
            marks: Vec::new(),
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
        let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler) };
        *self.ivars().monitor.borrow_mut() = monitor;
    }

    /// The bar is closing: stop watching for Esc.
    pub(crate) fn stop(&self) {
        if let Some(m) = self.ivars().monitor.borrow_mut().take() {
            unsafe { NSEvent::removeMonitor(&m) };
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

/// The shared find text (the find pasteboard), as Safari, Xcode and the
/// rest use it. Not under the test control, so smoke tests don't read or
/// change the user's.
pub(crate) fn shared_text() -> Option<String> {
    if std::env::var_os(crate::control::ENV).is_some() {
        return None;
    }
    let pb = NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameFind });
    pb.stringForType(unsafe { NSPasteboardTypeString }).map(|s| s.to_string())
}

/// Make `text` the shared find text.
pub(crate) fn share_text(text: &str) {
    if std::env::var_os(crate::control::ENV).is_some() {
        return;
    }
    let pb = NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameFind });
    pb.clearContents();
    pb.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
}
