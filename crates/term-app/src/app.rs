//! Application: delegate, menus, and opening windows.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSAutoresizingMaskOptions, NSBackingStoreType, NSColor,
    NSEventModifierFlags, NSMenu, NSMenuItem, NSScreen, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState,
    NSVisualEffectView, NSWindow, NSWindowOrderingMode, NSWindowStyleMask, NSWindowTabbingMode, NSWindowTitleVisibility,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSString};
use term_font::Fonts;
use term_pty::Command;

use crate::config::Config;
use crate::reload::SavedWindow;
use crate::secure_input::{self, SecureInput};
use crate::view::{Launch, TerminalView, content_size, delegate_of};

/// Every decline window shares this, so any two can be tabs of one window.
const TABBING_ID: &str = "red.forbes.decline.terminal";

/// Where a new window goes.
enum Place {
    /// Cascaded from the window in front, else centred.
    Cascade,
    /// At a saved frame (a restart into a new build).
    Frame(NSRect),
    /// A new tab of this window, after its current tab.
    TabOf(Retained<NSWindow>),
}

pub struct AppIvars {
    config: Rc<Config>,
    /// Windows to reopen after a dogfood build restarted into a newer one.
    restore: RefCell<Vec<crate::reload::SavedWindow>>,
    /// Config errors, shown at the top of each window.
    notices: RefCell<Rc<[String]>>,
    /// Whether the first window has checked the configured fonts are
    /// installed (adding to `notices` if not).
    fonts_checked: std::cell::Cell<bool>,
    windows: RefCell<Vec<(Retained<NSWindow>, Retained<TerminalView>)>>,
    /// Secure Keyboard Entry (decline menu).
    secure: RefCell<SecureInput>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeclineAppDelegate"]
    #[ivars = AppIvars]
    pub struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _n: &NSNotification) {
            let restore = std::mem::take(&mut *self.ivars().restore.borrow_mut());
            if restore.is_empty() {
                self.open_window(None, Place::Cascade, None);
            }
            self.restore(restore);
            self.watch_for_new_build();
            crate::latency::listen(|cmd| {
                let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
                if let Some(delegate) = delegate {
                    delegate.latency_command(cmd);
                }
            });
            let mtm = MainThreadMarker::from(self);
            #[allow(deprecated)]
            NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        }

        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(&self, _app: &NSApplication, has_visible: bool) -> bool {
            // Dock icon clicked with no windows open: open one.
            if !has_visible {
                self.open_window(None, Place::Cascade, None);
            }
            true
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _app: &NSApplication) -> bool {
            true
        }

        #[unsafe(method(applicationDidBecomeActive:))]
        fn did_become_active(&self, _n: &NSNotification) {
            secure_input::apply(self.ivars().secure.borrow_mut().set_active(true));
        }

        #[unsafe(method(applicationDidResignActive:))]
        fn did_resign_active(&self, _n: &NSNotification) {
            secure_input::apply(self.ivars().secure.borrow_mut().set_active(false));
        }

        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _n: &NSNotification) {
            secure_input::apply(self.ivars().secure.borrow_mut().quit());
        }
    }

    impl AppDelegate {
        #[unsafe(method(newWindow:))]
        fn new_window(&self, _sender: Option<&AnyObject>) {
            // Open in the same directory as the window in front.
            let cwd = self.key_view().and_then(|v| v.foreground_cwd());
            self.open_window(cwd, Place::Cascade, None);
        }

        /// Shell > New Tab: a tab of the window in front, in its directory.
        #[unsafe(method(newTab:))]
        fn new_tab(&self, _sender: Option<&AnyObject>) {
            self.new_tab_in_front();
        }

        /// decline > Secure Keyboard Entry: on or off, kept in user defaults.
        #[unsafe(method(toggleSecureKeyboardEntry:))]
        fn toggle_secure_keyboard_entry(&self, _sender: Option<&AnyObject>) {
            let (call, on) = {
                let mut secure = self.ivars().secure.borrow_mut();
                (secure.toggle(), secure.wanted())
            };
            secure_input::apply(call);
            objc2_foundation::NSUserDefaults::standardUserDefaults()
                .setBool_forKey(on, &NSString::from_str(secure_input::DEFAULTS_KEY));
        }

        /// Checkmarks: Secure Keyboard Entry shows whether it is on.
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            if item.action() == Some(sel!(toggleSecureKeyboardEntry:)) {
                let on = self.ivars().secure.borrow().wanted();
                item.setState(if on { objc2_app_kit::NSControlStateValueOn } else { objc2_app_kit::NSControlStateValueOff });
            }
            true
        }

        /// The tab bar's "+" button, which shows because this answers it.
        #[unsafe(method(newWindowForTab:))]
        fn new_window_for_tab(&self, sender: Option<&AnyObject>) {
            let _ = sender;
            self.new_tab_in_front();
        }

        /// Cmd+1…8 select that tab, Cmd+9 the last (the menu item's tag).
        #[unsafe(method(selectTab:))]
        fn select_tab(&self, sender: Option<&AnyObject>) {
            let Some(item) = sender.and_then(|s| s.downcast_ref::<NSMenuItem>()) else { return };
            let Some(window) = self.key_window() else { return };
            let Some(tabs) = window.tabbedWindows() else { return };
            if let Some(i) = tab_index(item.tag(), tabs.len()) {
                tabs.objectAtIndex(i).makeKeyAndOrderFront(None);
            }
        }

        #[unsafe(method(windowClosed:))]
        fn window_closed(&self, n: &NSNotification) {
            let closed = n.object();
            self.ivars().windows.borrow_mut().retain(|(w, _)| closed.as_deref().map(|o| !std::ptr::eq(o, w.as_ref() as &AnyObject)).unwrap_or(true));
        }
    }
);

impl AppDelegate {
    pub fn new(
        mtm: MainThreadMarker,
        config: Config,
        notices: Vec<String>,
        restore: Vec<crate::reload::SavedWindow>,
    ) -> Retained<AppDelegate> {
        let this = Self::alloc(mtm).set_ivars(AppIvars {
            config: Rc::new(config),
            restore: RefCell::new(restore),
            notices: RefCell::new(notices.into()),
            fonts_checked: std::cell::Cell::new(false),
            windows: RefCell::new(Vec::new()),
            secure: RefCell::new(SecureInput::new(
                objc2_foundation::NSUserDefaults::standardUserDefaults().boolForKey(&NSString::from_str(secure_input::DEFAULTS_KEY)),
            )),
        });
        let delegate: Retained<AppDelegate> = unsafe { msg_send![super(this), init] };
        DELEGATE.with(|d| *d.borrow_mut() = Some(objc2::rc::Weak::from_retained(&delegate)));
        delegate
    }

    /// Dogfood builds: restart into a newer build when one is installed.
    fn watch_for_new_build(&self) {
        let Some(bundle) = crate::reload::own_bundle() else { return };
        if !crate::reload::is_dogfood_bundle(&bundle) {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let _ = crate::reload::watch(exe, || {
            dispatch2::DispatchQueue::main().exec_async(|| {
                let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
                if let Some(delegate) = delegate {
                    delegate.restart_into_new_build();
                }
            });
        });
    }

    fn restart_into_new_build(&self) {
        let Some(bundle) = crate::reload::own_bundle() else { return };
        let saved = self.saved_windows();
        let restored = crate::reload::save(&saved).and_then(|file| crate::reload::relaunch(&bundle, &file));
        if let Err(e) = restored {
            eprintln!("decline: couldn't start the new build: {e}");
            return;
        }
        let mtm = MainThreadMarker::from(self);
        NSApplication::sharedApplication(mtm).terminate(None);
    }

    /// Each window to reopen after a restart, tabs in order with their
    /// group's index.
    fn saved_windows(&self) -> Vec<SavedWindow> {
        let windows = self.ivars().windows.borrow();
        let ours = |w: &NSWindow| {
            windows
                .iter()
                .find(|(o, _)| std::ptr::eq::<NSWindow>(&**o, w))
                .map(|(_, v)| v.clone())
        };
        let save = |w: &NSWindow, tab_group, selected| {
            let f = w.frame();
            SavedWindow {
                cwd: ours(w).and_then(|v| v.foreground_cwd()),
                frame: [f.origin.x, f.origin.y, f.size.width, f.size.height],
                tab_group,
                selected,
            }
        };
        let mut saved = Vec::new();
        let mut done: Vec<*const NSWindow> = Vec::new();
        let mut groups = 0;
        for (w, _) in windows.iter() {
            if done.contains(&Retained::as_ptr(w)) {
                continue;
            }
            match w.tabbedWindows().filter(|t| t.len() > 1) {
                Some(tabs) => {
                    let front = w.tabGroup().and_then(|g| g.selectedWindow());
                    for tab in tabs.iter().filter(|t| ours(t).is_some()) {
                        let selected = front.as_deref().is_some_and(|f| std::ptr::eq::<NSWindow>(f, &*tab));
                        saved.push(save(&tab, Some(groups), selected));
                        done.push(Retained::as_ptr(&tab));
                    }
                    groups += 1;
                }
                None => saved.push(save(w, None, false)),
            }
        }
        saved
    }

    /// Reopen saved windows, tabs as tabs, with the same tab in front.
    fn restore(&self, saved: Vec<SavedWindow>) {
        for group in crate::reload::tab_groups(saved) {
            let mut first: Option<Retained<NSWindow>> = None;
            let mut front = None;
            for w in group {
                let place = match &first {
                    Some(window) => Place::TabOf(window.clone()),
                    None => {
                        let [x, y, width, height] = w.frame;
                        Place::Frame(NSRect::new(NSPoint::new(x, y), objc2_foundation::NSSize::new(width, height)))
                    }
                };
                let opened = self.open_window(w.cwd, place, None);
                if w.selected {
                    front = opened.clone();
                }
                if first.is_none() {
                    first = opened;
                }
            }
            if let Some(front) = front {
                front.makeKeyAndOrderFront(None);
            }
        }
    }

    /// A tab of the window in front, else a window.
    fn new_tab_in_front(&self) {
        match self.key_window() {
            Some(window) => self.open_tab(&window),
            None => {
                self.open_window(None, Place::Cascade, None);
            }
        }
    }

    /// A new tab after `window`'s current one, in its directory.
    pub(crate) fn open_tab(&self, window: &NSWindow) {
        let cwd = self.view_of(window).and_then(|v| v.foreground_cwd());
        self.open_window(cwd, Place::TabOf(objc2::Message::retain(window)), None);
    }

    /// A command from the latency harness (see [`crate::latency`]).
    fn latency_command(&self, cmd: crate::latency::Command) {
        use crate::latency::Command as C;
        let mtm = MainThreadMarker::from(self);
        match cmd {
            C::Key { keycode } => {
                // Keys go to the key window, which only an active app has;
                // macOS may have refused activation at launch.
                let app = NSApplication::sharedApplication(mtm);
                if !app.isActive() {
                    app.activate();
                    if let Some((w, _)) = self.ivars().windows.borrow().first() {
                        w.makeKeyAndOrderFront(None);
                    }
                }
                crate::latency::post_key(keycode);
            }
            C::Type(bytes) => {
                let view = self.ivars().windows.borrow().first().map(|(_, v)| v.clone());
                if let Some(view) = view {
                    view.latency_type(bytes);
                }
            }
            C::Open(argv) => {
                // Behind the window being typed in, which stays key.
                let key = NSApplication::sharedApplication(mtm).keyWindow();
                self.open_window(None, Place::Cascade, Some(argv));
                if let Some(key) = key {
                    key.makeKeyAndOrderFront(None);
                }
            }
        }
    }

    fn key_view(&self) -> Option<Retained<TerminalView>> {
        self.view_of(&*self.key_window()?)
    }

    /// The key window, if it is one of ours.
    fn key_window(&self) -> Option<Retained<NSWindow>> {
        let mtm = MainThreadMarker::from(self);
        let key = NSApplication::sharedApplication(mtm).keyWindow()?;
        self.view_of(&key).map(|_| key)
    }

    fn view_of(&self, window: &NSWindow) -> Option<Retained<TerminalView>> {
        self.ivars()
            .windows
            .borrow()
            .iter()
            .find(|(w, _)| std::ptr::eq::<NSWindow>(&**w, window))
            .map(|(_, v)| v.clone())
    }

    /// Open a window in `cwd` (else the configured one) at `place`, running
    /// `argv` (else the configured program).
    fn open_window(&self, cwd: Option<String>, place: Place, argv: Option<Vec<String>>) -> Option<Retained<NSWindow>> {
        let mtm = MainThreadMarker::from(self);
        let config = self.ivars().config.clone();
        let scale = NSScreen::mainScreen(mtm).map(|s| s.backingScaleFactor()).unwrap_or(2.0);
        let fonts = Fonts::with_config(&config.font_config(config.font_size), scale);
        let metrics = fonts.metrics;
        if !self.ivars().fonts_checked.replace(true) {
            // Fonts the config names but that aren't installed: shown with
            // the other config notices, in this window and later ones.
            let missing = config.missing_font_notices(fonts.missing());
            for n in &missing {
                eprintln!("decline: {n}");
            }
            if !missing.is_empty() {
                let mut notices = self.ivars().notices.borrow_mut();
                *notices = notices.iter().cloned().chain(missing).collect();
            }
        }

        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable
            | NSWindowStyleMask::FullSizeContentView;
        let probe = NSRect::new(NSPoint::new(0.0, 0.0), objc2_foundation::NSSize::new(400.0, 300.0));
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), probe, style, NSBackingStoreType::Buffered, false)
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitlebarAppearsTransparent(true);
        window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        window.setTabbingIdentifier(&NSString::from_str(TABBING_ID));
        // A new window stays a window even when System Settings prefers
        // tabs (Cmd+T is for tabs); tabbing is allowed again once it is on
        // screen, for Merge All Windows and dragging tabs together.
        window.setTabbingMode(NSWindowTabbingMode::Disallowed);
        let title = config.title.clone().unwrap_or_else(|| "decline".into());
        window.setTitle(&NSString::from_str(&title));

        let titlebar = window.frame().size.height - window.contentLayoutRect().size.height;
        let size = content_size(&config, metrics, scale, titlebar.max(0.0));
        window.setContentSize(size);
        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), size);

        let view = TerminalView::new(mtm, bounds);
        view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
        if config.opacity < 1.0 {
            window.setOpaque(false);
            window.setBackgroundColor(Some(&NSColor::clearColor()));
        }
        if config.opacity < 1.0 && config.blur {
            let blur = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), bounds);
            blur.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
            blur.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            blur.setState(NSVisualEffectState::Active);
            blur.addSubview(&view);
            window.setContentView(Some(&blur));
        } else {
            window.setContentView(Some(&view));
        }
        window.setDelegate(Some(delegate_of(&view)));
        window.makeFirstResponder(Some(&view));
        window.setAcceptsMouseMovedEvents(true);

        let mut command = Command {
            argv: argv.unwrap_or_else(|| config.command.clone()),
            cwd: cwd.or_else(|| config.cwd.clone()),
            env: config.env.clone().into_iter().collect(),
            shell: None,
        };
        if let Some(dir) = &command.cwd {
            command.cwd = Some(expand_home(dir));
        }
        // Restored frame, else cascade from the window in front, else centre;
        // a tab takes its window's frame.
        match &place {
            Place::Frame(frame) => window.setFrame_display(*frame, false),
            Place::Cascade => match self.ivars().windows.borrow().last() {
                Some((prev, _)) => {
                    let f = prev.frame();
                    window.cascadeTopLeftFromPoint(NSPoint::new(f.origin.x, f.origin.y + f.size.height));
                    let _ = window.cascadeTopLeftFromPoint(NSPoint::new(f.origin.x + 22.0, f.origin.y + f.size.height - 22.0));
                }
                None => window.center(),
            },
            Place::TabOf(parent) => {
                window.setTabbingMode(NSWindowTabbingMode::Automatic);
                parent.addTabbedWindow_ordered(&window, NSWindowOrderingMode::Above);
            }
        }
        window.makeKeyAndOrderFront(None);
        // A new tab puts the one in front behind it, without key
        // notifications while the app is inactive.
        crate::view::sync_visibility_soon();
        window.setTabbingMode(NSWindowTabbingMode::Automatic);
        if std::env::var_os("DECLINE_DEBUG").is_some() {
            // Test tooling uses this to screenshot the window.
            eprintln!("decline: window {}", window.windowNumber());
        }

        if let Err(e) = view.start(
            config,
            Launch {
                command,
                fonts,
                scale,
                notices: self.ivars().notices.borrow().clone(),
            },
        ) {
            eprintln!("decline: could not open a window: {e}");
            window.close();
            return None;
        }
        let center = objc2_foundation::NSNotificationCenter::defaultCenter();
        unsafe {
            center.addObserver_selector_name_object(
                self,
                sel!(windowClosed:),
                Some(objc2_app_kit::NSWindowWillCloseNotification),
                Some(&window),
            )
        };
        self.ivars().windows.borrow_mut().push((window.clone(), view));
        Some(window)
    }
}

/// The tab a Cmd+number item selects: `tag` 1…8 is that tab, 9 the last.
fn tab_index(tag: isize, tabs: usize) -> Option<usize> {
    match tag {
        9 if tabs > 0 => Some(tabs - 1),
        1..=8 if (tag as usize) <= tabs => Some(tag as usize - 1),
        _ => None,
    }
}

/// Whether the app holds secure keyboard input (for the test dumps).
pub(crate) fn secure_input_held() -> bool {
    let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
    delegate.is_some_and(|d| d.ivars().secure.borrow().held())
}

/// A new tab beside `window` (the test-only `tab` control command).
pub(crate) fn open_tab_beside(window: &NSWindow) {
    let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
    if let Some(delegate) = delegate {
        delegate.open_tab(window);
    }
}

fn expand_home(p: &str) -> String {
    match (p.strip_prefix("~"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}{rest}"),
        _ => p.to_string(),
    }
}

fn item(mtm: MainThreadMarker, title: &str, action: Option<Sel>, key: &str) -> Retained<NSMenuItem> {
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(title), action, &NSString::from_str(key))
    }
}

fn submenu(mtm: MainThreadMarker, bar: &NSMenu, title: &str, items: &[Retained<NSMenuItem>]) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
    for i in items {
        menu.addItem(i);
    }
    let holder = item(mtm, title, None, "");
    holder.setSubmenu(Some(&menu));
    bar.addItem(&holder);
    menu
}

/// The menu bar. macOS needs one; these are the standard items plus the
/// terminal's few shortcuts.
pub fn build_menus(mtm: MainThreadMarker, app_name: &str) -> Retained<NSMenu> {
    let bar = NSMenu::new(mtm);
    let full_screen = item(mtm, "Enter Full Screen", Some(sel!(toggleFullScreen:)), "f");
    full_screen.setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Control);
    let hide_others = item(mtm, "Hide Others", Some(sel!(hideOtherApplications:)), "h");
    hide_others.setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Option);
    submenu(
        mtm,
        &bar,
        app_name,
        &[
            item(mtm, "Secure Keyboard Entry", Some(sel!(toggleSecureKeyboardEntry:)), ""),
            NSMenuItem::separatorItem(mtm),
            item(mtm, &format!("Hide {app_name}"), Some(sel!(hide:)), "h"),
            hide_others,
            NSMenuItem::separatorItem(mtm),
            item(mtm, &format!("Quit {app_name}"), Some(sel!(terminate:)), "q"),
        ],
    );
    submenu(
        mtm,
        &bar,
        "Shell",
        &[
            item(mtm, "New Window", Some(sel!(newWindow:)), "n"),
            item(mtm, "New Tab", Some(sel!(newTab:)), "t"),
            NSMenuItem::separatorItem(mtm),
            item(mtm, "Close", Some(sel!(performClose:)), "w"),
        ],
    );
    submenu(
        mtm,
        &bar,
        "Edit",
        &[
            item(mtm, "Copy", Some(sel!(copy:)), "c"),
            item(mtm, "Paste", Some(sel!(paste:)), "v"),
            item(mtm, "Select All", Some(sel!(selectAll:)), "a"),
            NSMenuItem::separatorItem(mtm),
            item(mtm, "Clear Scrollback", Some(sel!(clearBuffer:)), "k"),
        ],
    );
    submenu(
        mtm,
        &bar,
        "View",
        &[
            item(mtm, "Bigger", Some(sel!(increaseFontSize:)), "="),
            item(mtm, "Smaller", Some(sel!(decreaseFontSize:)), "-"),
            item(mtm, "Actual Size", Some(sel!(resetFontSize:)), "0"),
            NSMenuItem::separatorItem(mtm),
            full_screen,
        ],
    );
    // AppKit's own tab actions; with these in the menu it adds no copies.
    let mut window_items = vec![
        item(mtm, "Minimize", Some(sel!(performMiniaturize:)), "m"),
        item(mtm, "Zoom", Some(sel!(performZoom:)), ""),
        NSMenuItem::separatorItem(mtm),
        item(mtm, "Show Previous Tab", Some(sel!(selectPreviousTab:)), "{"),
        item(mtm, "Show Next Tab", Some(sel!(selectNextTab:)), "}"),
        item(mtm, "Move Tab to New Window", Some(sel!(moveTabToNewWindow:)), ""),
        item(mtm, "Merge All Windows", Some(sel!(mergeAllWindows:)), ""),
    ];
    // Cmd+1…9: hidden, as in Safari, but their keys still work.
    for n in 1..=9 {
        let select = item(mtm, &format!("Show Tab {n}"), Some(sel!(selectTab:)), &n.to_string());
        select.setTag(n);
        select.setHidden(true);
        select.setAllowsKeyEquivalentWhenHidden(true);
        window_items.push(select);
    }
    let window_menu = submenu(mtm, &bar, "Window", &window_items);
    let app = NSApplication::sharedApplication(mtm);
    app.setWindowsMenu(Some(&window_menu));
    bar
}

thread_local! {
    static DELEGATE: RefCell<Option<objc2::rc::Weak<AppDelegate>>> = const { RefCell::new(None) };
}

pub fn run(config: Config, notices: Vec<String>, app_name: &str, restore: Vec<crate::reload::SavedWindow>) {
    let mtm = MainThreadMarker::new().expect("must run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    crate::notify::on_click(crate::view::bring_to_front);
    let delegate = AppDelegate::new(mtm, config, notices, restore);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.setMainMenu(Some(&build_menus(mtm, app_name)));
    app.run();
}

#[cfg(test)]
mod tests {
    use super::tab_index;

    #[test]
    fn cmd_number_picks_a_tab() {
        for (tag, tabs, want) in [
            (1, 3, Some(0)),
            (3, 3, Some(2)),
            (4, 3, None),
            (8, 8, Some(7)),
            (9, 3, Some(2)),
            (9, 12, Some(11)),
            (9, 0, None),
            (1, 0, None),
            (0, 3, None),
            (10, 20, None),
        ] {
            assert_eq!(tab_index(tag, tabs), want, "tag={tag} tabs={tabs}");
        }
    }
}
