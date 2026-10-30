//! Smoke tests in the background (AGENTS.md, "Smoke tests open real
//! windows"): under the test control ([`crate::control::ENV`]) the app
//! never activates itself or brings a window to the front, so a test run
//! doesn't take Alex's keyboard focus or flash windows on his screen.
//!
//! - The app is an accessory app: no Dock icon, not in the app switcher.
//! - Its windows are transparent and ordered behind every other window,
//!   and count as visible whatever covers them, so they draw as a window
//!   in front does.
//! - An inactive app has no key window, so the app keeps its own: the
//!   window it last made key ([`make_key_and_order_front`]). An
//!   `NSApplication` subclass answers it as the key and main window, so
//!   keys sent with the test-only `key` command, menu shortcuts and menu
//!   actions go where they go when the app is in front, and the window
//!   is told it became key as AppKit would tell it.
//!
//! `DECLINE_TEST_BACKGROUND=1` does this without the test control.
//! `DECLINE_TEST_FOREGROUND=1` turns this off, to watch a test run: the
//! app then activates and orders its windows front as usual.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::OnceLock;

use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSResponder, NSWindow,
    NSWindowDidBecomeKeyNotification, NSWindowDidResignKeyNotification, NSWindowTabGroup,
};
use objc2_foundation::{NSNotification, NSObject};

/// The env var that keeps the test-controlled app in the foreground.
pub const FOREGROUND_ENV: &str = "DECLINE_TEST_FOREGROUND";

/// The env var that puts the app in the background without the test
/// control, for the smoke test that checks the control is off without it.
pub const BACKGROUND_ENV: &str = "DECLINE_TEST_BACKGROUND";

/// Whether the app runs in the background: the test control is on (or
/// [`BACKGROUND_ENV`] is set) and [`FOREGROUND_ENV`] isn't set.
pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty() && v != "0");
        (set(crate::control::ENV) || set(BACKGROUND_ENV)) && !set(FOREGROUND_ENV)
    })
}

thread_local! {
    /// The window the app last made key, in the background.
    static KEY: RefCell<Option<Weak<NSWindow>>> = const { RefCell::new(None) };
}

/// The app's own key window, in the background.
fn key() -> Option<Retained<NSWindow>> {
    KEY.with(|k| k.borrow().as_ref().and_then(|w| w.load())).filter(|w| w.isVisible())
}

define_class!(
    /// The shared application in the background: its key and main window
    /// is the app's own key window while macOS gives it none.
    #[unsafe(super(NSApplication, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeclineBackgroundApplication"]
    struct BackgroundApplication;

    unsafe impl NSObjectProtocol for BackgroundApplication {}

    impl BackgroundApplication {
        #[unsafe(method_id(keyWindow))]
        fn key_window(&self) -> Option<Retained<NSWindow>> {
            let real: Option<Retained<NSWindow>> = unsafe { msg_send![super(self), keyWindow] };
            real.or_else(key)
        }

        #[unsafe(method_id(mainWindow))]
        fn main_window(&self) -> Option<Retained<NSWindow>> {
            let real: Option<Retained<NSWindow>> = unsafe { msg_send![super(self), mainWindow] };
            real.or_else(key)
        }
    }
);

/// Make the shared application, before anything else asks for it, and
/// set its activation policy: an accessory app in the background, else a
/// regular one.
pub fn shared_application(mtm: MainThreadMarker) -> Retained<NSApplication> {
    let (app, policy) = if on() {
        let app: Retained<NSApplication> = unsafe { msg_send![BackgroundApplication::class(), sharedApplication] };
        (app, NSApplicationActivationPolicy::Accessory)
    } else {
        (NSApplication::sharedApplication(mtm), NSApplicationActivationPolicy::Regular)
    };
    app.setActivationPolicy(policy);
    app
}

/// `NSApplication.activate`, except in the background.
pub fn activate(mtm: MainThreadMarker) {
    if !on() {
        NSApplication::sharedApplication(mtm).activate();
    }
}

/// `activateIgnoringOtherApps:`, except in the background.
pub fn activate_ignoring_other_apps(mtm: MainThreadMarker) {
    if !on() {
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
}

/// In the background, makes `window` (a sheet or an alert, say)
/// transparent, so it isn't seen over other apps' windows.
pub fn hide(window: &NSWindow) {
    if on() {
        window.setAlphaValue(0.0);
    }
}

/// The window's tab group when it has more than one tab, else `None`.
/// Asking a window for its `tabGroup` makes AppKit build the group's tab
/// bar, a tab bar item and a copy of the app's icon for it (1.4 MB of
/// footprint for a window on its own), even when the bar never shows;
/// `tabbedWindows` doesn't, and is `None` for a window on its own.
pub fn tab_group(window: &NSWindow) -> Option<Retained<NSWindowTabGroup>> {
    window.tabbedWindows().filter(|t| t.len() > 1)?;
    window.tabGroup()
}

/// `makeKeyAndOrderFront:`. In the background: the window, transparent,
/// goes behind every other window (a tab is selected in its group
/// instead) and becomes the app's own key window.
pub fn make_key_and_order_front(window: &NSWindow) {
    if !on() {
        window.makeKeyAndOrderFront(None);
        return;
    }
    hide(window);
    match tab_group(window) {
        Some(group) => group.setSelectedWindow(Some(window)),
        None if !window.isVisible() => window.orderBack(None),
        None => {}
    }
    make_key(window);
}

/// A local event monitor's handler.
pub type Handler = RcBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent>;

thread_local! {
    /// The local monitors of key downs, in the order added, in the
    /// background: [`send_key`] hands them its key downs.
    static MONITORS: RefCell<Vec<(Retained<AnyObject>, Handler)>> = const { RefCell::new(Vec::new()) };
}

/// `NSEvent.addLocalMonitorForEventsMatchingMask:handler:`. In the
/// background a monitor of key downs also hears [`send_key`]'s.
pub fn add_local_monitor(mask: NSEventMask, handler: Handler) -> Option<Retained<AnyObject>> {
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) }?;
    if on() && mask.contains(NSEventMask::KeyDown) {
        MONITORS.with(|m| m.borrow_mut().push((monitor.clone(), handler)));
    }
    Some(monitor)
}

/// `NSEvent.removeMonitor:`, for a monitor from [`add_local_monitor`].
pub fn remove_monitor(monitor: &AnyObject) {
    unsafe { NSEvent::removeMonitor(monitor) };
    MONITORS.with(|m| m.borrow_mut().retain(|(o, _)| !std::ptr::eq::<AnyObject>(&**o, monitor)));
}

/// `NSApplication.sendEvent` of a key event for `window`, the key window
/// (the test-only `key` command). An inactive app has no key window for
/// AppKit to hand a key to, so in the background this does what AppKit
/// does: a key down goes to the local monitors (in the order added),
/// then, with Command or Control, to the window's and the main menu's key
/// equivalents, and what none of them took, and a key up, to the window.
pub fn send_key(app: &NSApplication, window: &NSWindow, event: &NSEvent) {
    if !on() || app.isActive() {
        app.sendEvent(event);
        return;
    }
    if event.r#type() == NSEventType::KeyDown {
        let handlers: Vec<Handler> = MONITORS.with(|m| m.borrow().iter().map(|(_, h)| h.clone()).collect());
        if handlers.iter().any(|h| h.call((NonNull::from(event),)).is_null()) {
            return;
        }
        // A plain key goes to the first responder, as AppKit hands it (a
        // hidden menu item may match it otherwise).
        let chord = event
            .modifierFlags()
            .intersects(NSEventModifierFlags::Command | NSEventModifierFlags::Control);
        if chord && (window.performKeyEquivalent(event) || app.mainMenu().is_some_and(|m| m.performKeyEquivalent(event))) {
            return;
        }
    }
    window.sendEvent(event);
}

/// `isKeyWindow`, or in the background whether it is the app's own key
/// window.
pub fn is_key(window: &NSWindow) -> bool {
    match on() {
        true => key().is_some_and(|k| std::ptr::eq::<NSWindow>(&*k, window)),
        false => window.isKeyWindow(),
    }
}

/// After `window` closed, in the background: another window becomes key,
/// as AppKit would pick one, on the next turn of the main queue (once a
/// closed tab's group has selected another): the selected tab of a group,
/// else the last of `windows` that is still open.
pub fn closed(window: &NSWindow, windows: Vec<Retained<NSWindow>>) {
    if !on() || key().is_some_and(|k| !std::ptr::eq::<NSWindow>(&*k, window)) {
        return;
    }
    KEY.with(|k| *k.borrow_mut() = None);
    let windows = dispatch2::MainThreadBound::new(windows, MainThreadMarker::from(window));
    dispatch2::DispatchQueue::main().exec_async(move || {
        let windows = windows.into_inner(MainThreadMarker::new().expect("on the main thread"));
        let open: Vec<_> = windows.into_iter().filter(|w| w.isVisible()).collect();
        let selected = open.iter().find(|w| {
            tab_group(w)
                .and_then(|g| g.selectedWindow())
                .is_none_or(|s| std::ptr::eq::<NSWindow>(&*s, &***w))
        });
        if let Some(next) = selected.or(open.last()) {
            make_key(next);
        }
    });
}

/// `window` becomes the app's own key window; it and the window that was
/// key are told, as AppKit tells them.
fn make_key(window: &NSWindow) {
    let old = key();
    if old.as_deref().is_some_and(|o| std::ptr::eq::<NSWindow>(o, window)) {
        return;
    }
    KEY.with(|k| *k.borrow_mut() = Some(Weak::from(window)));
    if let Some(old) = old {
        tell(&old, false);
    }
    tell(window, true);
}

/// Send `window`'s delegate `windowDidBecomeKey:` (`became`) or
/// `windowDidResignKey:`, if it answers it.
fn tell(window: &NSWindow, became: bool) {
    let Some(delegate) = window.delegate() else { return };
    let delegate: &AnyObject = delegate.as_ref();
    let (name, selector) = match became {
        true => (unsafe { NSWindowDidBecomeKeyNotification }, sel!(windowDidBecomeKey:)),
        false => (unsafe { NSWindowDidResignKeyNotification }, sel!(windowDidResignKey:)),
    };
    let answers: bool = unsafe { msg_send![delegate, respondsToSelector: selector] };
    if !answers {
        return;
    }
    let n = unsafe { NSNotification::notificationWithName_object(name, Some(window)) };
    match became {
        true => unsafe { msg_send![delegate, windowDidBecomeKey: &*n] },
        false => unsafe { msg_send![delegate, windowDidResignKey: &*n] },
    }
}
