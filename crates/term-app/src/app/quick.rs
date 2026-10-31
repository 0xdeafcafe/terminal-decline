//! The quick terminal's window (see [`crate::quick`]): made on the first
//! press of the hotkey, then slid in and out. Hidden, it is ordered out:
//! its view counts it as not seen and gives back its drawables, as a
//! covered window or a background tab does, and its session keeps
//! running.

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSApplication, NSApplicationActivationOptions, NSBackingStoreType, NSEvent,
    NSRunningApplication, NSScreen, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use super::{AppDelegate, DELEGATE};
use crate::quick::{self, Rect};

/// The quick terminal's tabbing identifier: its own, so Merge All
/// Windows never takes it into another window's tabs.
pub(super) const TABBING_ID: &str = "red.forbes.terminal-decline.quick";

define_class!(
    /// The quick terminal's window: borderless, it says it can become key
    /// and main, and lets itself be placed past the screen's edge (where
    /// it slides from) without AppKit moving it back.
    #[unsafe(super(NSWindow, objc2_app_kit::NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeclineQuickWindow"]
    pub(super) struct QuickWindow;

    impl QuickWindow {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool {
            true
        }

        #[unsafe(method(constrainFrameRect:toScreen:))]
        fn constrain(&self, frame: NSRect, _screen: Option<&AnyObject>) -> NSRect {
            frame
        }
    }
);

impl QuickWindow {
    /// A new quick terminal window, not yet on screen.
    pub(super) fn create(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSWindow> {
        let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::Resizable;
        let window: Retained<QuickWindow> = unsafe {
            msg_send![QuickWindow::alloc(mtm), initWithContentRect: frame, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
        };
        let window = Retained::into_super(window);
        // Over other apps' windows and their full-screen Spaces, on the
        // Space that is active when it shows; not in Cmd+` cycling.
        window.setLevel(objc2_app_kit::NSFloatingWindowLevel);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::MoveToActiveSpace
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        window.setHidesOnDeactivate(false);
        window
    }
}

/// The quick terminal, once made.
pub(super) struct QuickTerminal {
    pub(super) window: Retained<NSWindow>,
    /// Shown, or sliding in.
    pub(super) shown: bool,
    /// The app that was in front when it was shown, which gets the focus
    /// back when it hides.
    previous: Option<Retained<NSRunningApplication>>,
}

thread_local! {
    /// Each show or hide's number: a slide's end does nothing once
    /// another has started.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

fn to_rect(r: Rect) -> NSRect {
    NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.w, r.h))
}

fn of_rect(r: NSRect) -> Rect {
    Rect {
        x: r.origin.x,
        y: r.origin.y,
        w: r.size.width,
        h: r.size.height,
    }
}

/// The usable area (`visibleFrame`) of the screen the mouse is on.
fn screen_under_mouse(mtm: MainThreadMarker) -> Rect {
    let screens = NSScreen::screens(mtm);
    let frames: Vec<Rect> = screens.iter().map(|s| of_rect(s.frame())).collect();
    let mouse = NSEvent::mouseLocation();
    let i = quick::screen_under(&frames, mouse.x, mouse.y);
    match screens.iter().nth(i) {
        Some(s) => of_rect(s.visibleFrame()),
        None => Rect {
            x: 0.0,
            y: 0.0,
            w: 1440.0,
            h: 900.0,
        },
    }
}

/// The screen the window is on (its usable area).
fn screen_of(window: &NSWindow, mtm: MainThreadMarker) -> Rect {
    window
        .screen()
        .map(|s| of_rect(s.visibleFrame()))
        .unwrap_or_else(|| screen_under_mouse(mtm))
}

impl AppDelegate {
    /// Register `quick_terminal_hotkey` (again, after a reload): not in a
    /// pin, nor under the test control, which must leave Alex's keys
    /// alone (it toggles with the test-only `quick` command).
    pub(super) fn register_hotkey(&self) {
        // The old one first: the same keys can't be registered twice.
        self.ivars().hotkey.borrow_mut().take();
        if self.in_pin() || crate::background::on() {
            return;
        }
        let text = self.ivars().config.borrow().quick_terminal_hotkey.clone();
        let Ok(Some(key)) = quick::Hotkey::parse(&text) else { return };
        match crate::hotkey::register(key, toggle_quick_now) {
            Ok(r) => *self.ivars().hotkey.borrow_mut() = Some(r),
            Err(e) => eprintln!("terminal-decline: quick_terminal_hotkey {text:?}: {e}"),
        }
    }

    /// The hotkey: show the quick terminal; focus it if it shows but
    /// another window has the keys; else hide it.
    pub(super) fn toggle_quick(&self) {
        if self.in_pin() {
            return;
        }
        let mtm = MainThreadMarker::from(self);
        let state = self.ivars().quick.borrow().as_ref().map(|q| (q.window.clone(), q.shown));
        match state {
            Some((window, true)) => {
                let active = NSApplication::sharedApplication(mtm).isActive() || crate::background::on();
                if active && crate::background::is_key(&window) {
                    self.hide_quick();
                } else {
                    crate::background::activate(mtm);
                    crate::background::make_key_and_order_front(&window);
                }
            }
            _ => self.show_quick(),
        }
    }

    fn show_quick(&self) {
        let mtm = MainThreadMarker::from(self);
        let config = self.ivars().config.borrow().clone();
        let screen = screen_under_mouse(mtm);
        let position = config.quick_terminal_position;
        let shown = quick::shown_frame(position, screen, config.quick_terminal_size);
        let from = quick::hidden_frame(position, screen, shown);
        let app = NSApplication::sharedApplication(mtm);
        let previous = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .filter(|a| !app.isActive() && a.processIdentifier() != std::process::id() as i32);
        let existing = self.ivars().quick.borrow().as_ref().map(|q| q.window.clone());
        let window = match existing {
            Some(window) => {
                window.setFrame_display(to_rect(from), false);
                crate::background::activate(mtm);
                crate::background::make_key_and_order_front(&window);
                window
            }
            None => {
                // Opened where it slides from, made key and ordered front.
                crate::background::activate(mtm);
                let Some(window) = self.open_window_as(None, super::Place::Quick(to_rect(from)), None, false, None, None, None, None)
                else {
                    return;
                };
                window
            }
        };
        *self.ivars().quick.borrow_mut() = Some(QuickTerminal {
            window: window.clone(),
            shown: true,
            previous,
        });
        let fade = position == quick::Position::Center && !crate::background::on();
        if fade {
            window.setAlphaValue(0.0);
        }
        // A hide still sliding away doesn't order it out now.
        GENERATION.with(|g| g.set(g.get() + 1));
        animate(&window, to_rect(shown), fade.then_some(1.0), self.slide_seconds(), || {});
        if let Some(view) = self.view_of(&window) {
            view.sync_visibility();
        }
    }

    /// Slide the quick terminal away, then order it out, and give the
    /// focus back to the app that had it.
    pub(super) fn hide_quick(&self) {
        let mtm = MainThreadMarker::from(self);
        let (window, previous) = {
            let mut quick = self.ivars().quick.borrow_mut();
            let Some(q) = quick.as_mut().filter(|q| q.shown) else { return };
            q.shown = false;
            (q.window.clone(), q.previous.take())
        };
        let position = self.ivars().config.borrow().quick_terminal_position;
        let to = quick::hidden_frame(position, screen_of(&window, mtm), of_rect(window.frame()));
        let generation = GENERATION.with(|g| g.replace(g.get() + 1) + 1);
        let fade = (position == quick::Position::Center && !crate::background::on()).then_some(0.0);
        let w = window.clone();
        animate(&window, to_rect(to), fade, self.slide_seconds(), move || {
            if GENERATION.with(|g| g.get()) != generation {
                return;
            }
            w.orderOut(None);
            let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
            if let Some(view) = delegate.as_ref().and_then(|d| d.view_of(&w)) {
                view.sync_visibility();
            }
        });
        // The app that was in front before it showed gets the focus
        // back, unless the user went somewhere else meanwhile.
        if let Some(previous) = previous
            && NSApplication::sharedApplication(mtm).isActive()
            && !crate::background::on()
        {
            previous.activateWithOptions(NSApplicationActivationOptions::empty());
        }
    }

    /// `quick_terminal_animation_ms`, as the slide's length in seconds.
    fn slide_seconds(&self) -> f64 {
        let ms = self.ivars().config.borrow().quick_terminal_animation_ms;
        let reduce = NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        quick::animation_seconds(ms, reduce, crate::background::on())
    }

    /// Pins have no quick terminal: the app's own process has it.
    fn in_pin(&self) -> bool {
        matches!(self.ivars().role, crate::entry::Role::Pin(_))
    }

    pub(super) fn is_quick(&self, window: &NSWindow) -> bool {
        self.ivars()
            .quick
            .borrow()
            .as_ref()
            .is_some_and(|q| std::ptr::eq::<NSWindow>(&*q.window, window))
    }
}

/// Move `window` to `frame` (and its alpha to `alpha`) over `seconds`,
/// then call `done`; at once with 0 seconds. AppKit runs the animation
/// on the main thread's display link while it lasts, and nothing after.
fn animate(window: &NSWindow, frame: NSRect, alpha: Option<f64>, seconds: f64, done: impl Fn() + 'static) {
    if seconds <= 0.0 {
        window.setFrame_display(frame, true);
        if let Some(a) = alpha {
            window.setAlphaValue(a);
        }
        done();
        return;
    }
    let target = objc2::Message::retain(window);
    let changes = block2::RcBlock::new(move |context: std::ptr::NonNull<NSAnimationContext>| {
        let context = unsafe { context.as_ref() };
        context.setDuration(seconds);
        let animator = target.animator();
        animator.setFrame_display(frame, true);
        if let Some(a) = alpha {
            animator.setAlphaValue(a);
        }
    });
    let done = block2::RcBlock::new(done);
    NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&done));
}

/// The hotkey's handler, and the test-only `quick` command.
pub(crate) fn toggle_quick_now() {
    let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
    if let Some(delegate) = delegate {
        delegate.toggle_quick();
    }
}

/// `window` stopped being key: with `quick_terminal_autohide`, the quick
/// terminal slides away (not for its own sheet, the close question).
pub(crate) fn resigned_key(window: &NSWindow) {
    let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
    let Some(delegate) = delegate else { return };
    if !delegate.is_quick(window) || window.attachedSheet().is_some() || !delegate.ivars().config.borrow().quick_terminal_autohide {
        return;
    }
    // On the next turn: a sheet being attached resigns key first.
    let window = dispatch2::MainThreadBound::new(objc2::Message::retain(window), MainThreadMarker::from(&*delegate));
    dispatch2::DispatchQueue::main().exec_async(move || {
        let mtm = MainThreadMarker::new().expect("on the main thread");
        let window = window.into_inner(mtm);
        let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
        let Some(delegate) = delegate else { return };
        if window.attachedSheet().is_none() && !crate::background::is_key(&window) {
            delegate.hide_quick();
        }
    });
}

/// For a dump: whether this is the quick terminal and it shows, and its
/// frame and its screen's usable area (x, y, width, height).
pub(crate) fn status(window: &NSWindow) -> Option<crate::control::QuickDump> {
    let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()))?;
    let shown = delegate
        .ivars()
        .quick
        .borrow()
        .as_ref()
        .filter(|q| std::ptr::eq::<NSWindow>(&*q.window, window))?
        .shown;
    let mtm = MainThreadMarker::from(&*delegate);
    let f = of_rect(window.frame());
    let s = screen_of(window, mtm);
    Some(crate::control::QuickDump {
        shown,
        on_screen: window.isVisible(),
        frame: [f.x, f.y, f.w, f.h],
        screen: [s.x, s.y, s.w, s.h],
        level: window.level() as i64,
    })
}
