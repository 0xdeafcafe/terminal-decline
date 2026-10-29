//! Application: delegate, menus, and opening windows.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSApplicationTerminateReply,
    NSAutoresizingMaskOptions, NSBackingStoreType, NSColor, NSEventModifierFlags, NSMenu, NSMenuItem, NSPasteboard, NSScreen,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindow, NSWindowOrderingMode,
    NSWindowStyleMask, NSWindowTabbingMode, NSWindowTitleVisibility, NSWindowUserTabbingPreference,
};
use objc2_foundation::{NSArray, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSString, NSURL};
use term_font::Fonts;
use term_pty::Command;
use term_render::Renderer;

use crate::config::Config;
use crate::desktop::{self, FrontWindow, Open, TabPreference};
use crate::keybind::{Keybinds, MenuAction};
use crate::reload::SavedWindow;
use crate::secure_input::{self, SecureInput};
use crate::view::{Launch, WindowView, content_size, delegate_of};

/// Every Terminal Decline window shares this, so any two can be tabs of one
/// window.
const TABBING_ID: &str = "red.forbes.terminal-decline.terminal";

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
    /// The config in effect; replaced when a config file changes.
    config: RefCell<Rc<Config>>,
    /// Where the config was read from, to read it again.
    origin: crate::config::Source,
    /// Watches the config files (see [`crate::watch`]).
    watcher: RefCell<Option<crate::watch::Watcher>>,
    /// The app's name, for the menus rebuilt after a reload.
    app_name: String,
    /// Windows to reopen after a dogfood build restarted into a newer one.
    restore: RefCell<Vec<crate::reload::SavedWindow>>,
    /// Config errors, shown at the top of each window.
    notices: RefCell<Rc<[String]>>,
    /// Whether the first window has checked the configured fonts are
    /// installed (adding to `notices` if not).
    fonts_checked: std::cell::Cell<bool>,
    windows: RefCell<Vec<(Retained<NSWindow>, Retained<WindowView>)>>,
    /// Secure Keyboard Entry (Terminal Decline menu).
    secure: RefCell<SecureInput>,
    /// Whether `applicationDidFinishLaunching:` has run.
    launched: Cell<bool>,
    /// Folders opened with the app before it finished launching (a folder
    /// dropped on the Dock icon of an app not running): their windows open
    /// instead of the first window.
    opened_at_launch: RefCell<Vec<String>>,
    /// Quit without asking, even with `confirm_close` (a restart into a
    /// new build, or the test-only `quit`).
    quit_without_asking: Cell<bool>,
    /// The Settings tab, while it is open: Cmd+, brings it to the front.
    settings: RefCell<Option<Retained<NSWindow>>>,
    /// The errors of the config in effect, so a reload writes only new
    /// ones into the windows.
    errors: RefCell<Vec<String>>,
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
            let folders = std::mem::take(&mut *self.ivars().opened_at_launch.borrow_mut());
            self.ivars().launched.set(true);
            if restore.is_empty() && folders.is_empty() {
                self.open_window(None, Place::Cascade, None);
            }
            self.restore(restore);
            for dir in folders {
                self.open_folder(dir, None);
            }
            let mtm = MainThreadMarker::from(self);
            unsafe { NSApplication::sharedApplication(mtm).setServicesProvider(Some(self)) };
            self.watch_for_new_build();
            self.watch_config();
            crate::latency::listen(|cmd| {
                let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
                if let Some(delegate) = delegate {
                    delegate.latency_command(cmd);
                }
            });
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

        /// Folders (or files, which open in their folder) dropped on the
        /// Dock icon or opened with `open -a`: a window there, or a tab when
        /// macOS prefers tabs.
        #[unsafe(method(application:openURLs:))]
        fn open_urls(&self, _app: &NSApplication, urls: &NSArray<NSURL>) {
            for dir in folders_of(urls.iter().filter_map(|u| u.path()).map(|p| p.to_string())) {
                if self.ivars().launched.get() {
                    self.open_folder(dir, None);
                } else {
                    self.ivars().opened_at_launch.borrow_mut().push(dir);
                }
            }
        }

        /// With `confirm_close`, ask before quitting while programs run.
        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _app: &NSApplication) -> NSApplicationTerminateReply {
            if self.ivars().quit_without_asking.get() || !self.ivars().config.borrow().confirm_close {
                return NSApplicationTerminateReply::TerminateNow;
            }
            let views: Vec<_> = self.ivars().windows.borrow().iter().map(|(_, v)| v.clone()).collect();
            let programs: Vec<String> = views.iter().flat_map(|v| v.running_programs()).collect();
            if programs.is_empty() {
                return NSApplicationTerminateReply::TerminateNow;
            }
            let mtm = MainThreadMarker::from(self);
            let (title, text) = crate::running::quit_question(&self.ivars().app_name, &programs);
            let alert = NSAlert::new(mtm);
            alert.setMessageText(&NSString::from_str(&title));
            alert.setInformativeText(&NSString::from_str(&text));
            alert.addButtonWithTitle(&NSString::from_str("Quit"));
            alert.addButtonWithTitle(&NSString::from_str("Cancel"));
            if alert.runModal() == NSAlertFirstButtonReturn {
                NSApplicationTerminateReply::TerminateNow
            } else {
                NSApplicationTerminateReply::TerminateCancel
            }
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

        /// Services > New Terminal Decline Window at Folder (NSServices in
        /// the bundle's Info.plist).
        #[unsafe(method(newWindowAtFolder:userData:error:))]
        fn new_window_at_folder(&self, pb: &NSPasteboard, _data: Option<&NSString>, _error: *mut *mut NSString) {
            for dir in folders_of(crate::pane_view::file_paths(pb)) {
                self.open_folder(dir, Some(Open::Window));
            }
        }

        /// Services > New Terminal Decline Tab at Folder.
        #[unsafe(method(newTabAtFolder:userData:error:))]
        fn new_tab_at_folder(&self, pb: &NSPasteboard, _data: Option<&NSString>, _error: *mut *mut NSString) {
            for dir in folders_of(crate::pane_view::file_paths(pb)) {
                self.open_folder(dir, Some(Open::Tab));
            }
        }

        /// Shell > New Tab: a tab of the window in front, in its directory.
        #[unsafe(method(newTab:))]
        fn new_tab(&self, _sender: Option<&AnyObject>) {
            self.new_tab_in_front();
        }

        /// Terminal Decline > Settings… (Cmd+,).
        #[unsafe(method(showSettings:))]
        fn show_settings_item(&self, _sender: Option<&AnyObject>) {
            self.show_settings();
        }

        /// Terminal Decline > Edit config.toml: the file in the default
        /// text editor, for anyone who prefers it to Settings….
        #[unsafe(method(editConfig:))]
        fn edit_config(&self, _sender: Option<&AnyObject>) {
            let Some(file) = self.ivars().origin.edited().map(std::path::Path::to_path_buf) else { return };
            if !file.exists() {
                // `open` needs a file to open.
                let made = file
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::OpenOptions::new().create_new(true).write(true).open(&file).map(drop));
                if let Err(e) = made {
                    eprintln!("terminal-decline: {}: {e}", file.display());
                }
            }
            if let Err(e) = std::process::Command::new("/usr/bin/open").arg("-t").arg(&file).spawn() {
                eprintln!("terminal-decline: open -t {}: {e}", file.display());
            }
        }

        /// Terminal Decline > Secure Keyboard Entry: on or off, kept in user
        /// defaults.
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
            let is_closed = |w: &NSWindow| closed.as_deref().is_some_and(|o| std::ptr::eq(o, w as &AnyObject));
            self.ivars().windows.borrow_mut().retain(|(w, _)| !is_closed(w));
            let mut settings = self.ivars().settings.borrow_mut();
            if settings.as_deref().is_some_and(is_closed) {
                *settings = None;
            }
        }
    }
);

impl AppDelegate {
    pub fn new(
        mtm: MainThreadMarker,
        config: Config,
        notices: Vec<String>,
        restore: Vec<crate::reload::SavedWindow>,
        origin: crate::config::Source,
        app_name: &str,
    ) -> Retained<AppDelegate> {
        // A choice made before the rename, under the old bundle id. The
        // main bundle is this process's: the app's, or in a pin the pin's
        // own (its Secure Keyboard Entry choice is its own).
        if let Some(id) = objc2_foundation::NSBundle::mainBundle().bundleIdentifier() {
            secure_input::carry_over(&id.to_string());
        }
        let this = Self::alloc(mtm).set_ivars(AppIvars {
            config: RefCell::new(Rc::new(config)),
            origin,
            watcher: RefCell::new(None),
            app_name: app_name.into(),
            restore: RefCell::new(restore),
            launched: Cell::new(false),
            opened_at_launch: RefCell::new(Vec::new()),
            quit_without_asking: Cell::new(false),
            settings: RefCell::new(None),
            errors: RefCell::new(notices.clone()),
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

    /// Reload the config whenever one of its files changes.
    fn watch_config(&self) {
        let config = self.ivars().config.borrow().clone();
        let files = self.ivars().origin.watched(&config);
        let watcher = crate::watch::Watcher::start(files, || {
            dispatch2::DispatchQueue::main().exec_async(|| {
                let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
                if let Some(delegate) = delegate {
                    delegate.reload_config();
                }
            });
        });
        match watcher {
            Ok(w) => *self.ivars().watcher.borrow_mut() = Some(w),
            Err(e) => eprintln!("terminal-decline: can't watch the config for changes: {e}"),
        }
    }

    /// Read the config again and apply it: to every window what can change
    /// while it runs, the rest to new windows. Errors new since the last
    /// load are written into the windows, except while the Settings tab is
    /// open (it shows the same messages) and into full-screen programs;
    /// that a setting waits for new windows only goes to stderr (the
    /// Settings tab marks those settings).
    fn reload_config(&self) {
        let (config, errors) = self.ivars().origin.load();
        let config = Rc::new(config);
        let old = self.ivars().config.replace(config.clone());
        let mut shown = errors.clone();
        if old.font_config(old.font_size) != config.font_config(config.font_size) && config.names_fonts() {
            let mtm = MainThreadMarker::from(self);
            let scale = NSScreen::mainScreen(mtm).map(|s| s.backingScaleFactor()).unwrap_or(2.0);
            let fonts = Fonts::with_config(&config.font_config(config.font_size), scale);
            shown.extend(config.missing_font_notices(fonts.missing()));
        }
        // New windows show the errors, as they would at startup.
        *self.ivars().notices.borrow_mut() = shown.iter().cloned().collect();
        let before = self.ivars().errors.replace(shown.clone());
        let later = crate::config::new_windows_notice(&crate::config::new_windows_only(&old, &config));
        for n in shown.iter().chain(&later) {
            eprintln!("terminal-decline: {n}");
        }
        let settings_open = self.ivars().settings.borrow().is_some();
        shown.retain(|n| !settings_open && !before.contains(n));
        if old.keybind != config.keybind {
            let mtm = MainThreadMarker::from(self);
            let menus = build_menus(mtm, &self.ivars().app_name, &config.keybinds());
            NSApplication::sharedApplication(mtm).setMainMenu(Some(&menus));
        }
        let windows: Vec<_> = self.ivars().windows.borrow().clone();
        for (window, view) in windows {
            if !view.is_settings() {
                apply_opacity(&window, &config);
            }
            view.apply_config(config.clone(), &shown);
        }
        if let Some(w) = self.ivars().watcher.borrow().as_ref() {
            w.set_paths(self.ivars().origin.watched(&config));
        }
    }

    /// Dogfood builds: restart into a newer build when one is installed.
    fn watch_for_new_build(&self) {
        let Some(bundle) = crate::reload::own_bundle() else { return };
        if !crate::reload::is_dogfood_bundle(&bundle) {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let code = crate::reload::running_code(&bundle, exe);
        // Kept from now: once the bundle is gone, the executable can't be
        // resolved to it any more.
        let _ = crate::reload::watch(code, move || {
            dispatch2::DispatchQueue::main().exec_async(move || {
                let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
                if let Some(delegate) = delegate {
                    delegate.restart_into_new_build(&bundle);
                }
            });
        });
    }

    fn restart_into_new_build(&self, bundle: &std::path::Path) {
        let Some(bundle) = crate::reload::relaunch_target(bundle) else {
            eprintln!("terminal-decline: {} is gone and no new build is beside it", bundle.display());
            return;
        };
        let saved = self.saved_windows();
        let restored = crate::reload::save(&saved).and_then(|file| crate::reload::relaunch(&bundle, &file));
        if let Err(e) = restored {
            eprintln!("terminal-decline: couldn't start the new build: {e}");
            return;
        }
        self.quit_without_asking();
    }

    /// Quit, without asking whatever `confirm_close` says.
    fn quit_without_asking(&self) {
        self.ivars().quit_without_asking.set(true);
        let mtm = MainThreadMarker::from(self);
        let app = NSApplication::sharedApplication(mtm);
        // AppKit puts off terminating while a window shows a sheet (a
        // close question, say), so end those first.
        for window in app.windows().iter() {
            if let Some(sheet) = window.attachedSheet() {
                window.endSheet(&sheet);
            }
        }
        app.terminate(None);
    }

    /// A window in `dir`, or a tab of the window in front: as `how` says,
    /// else as macOS's tab preference says. The app comes to the front, as
    /// it does for any document opened.
    fn open_folder(&self, dir: String, how: Option<Open>) {
        let mtm = MainThreadMarker::from(self);
        let front = self.front_window();
        let how = how.unwrap_or_else(|| {
            let pref = match NSWindow::userTabbingPreference(mtm) {
                NSWindowUserTabbingPreference::Always => TabPreference::Always,
                NSWindowUserTabbingPreference::Manual => TabPreference::Manual,
                _ => TabPreference::InFullScreen,
            };
            let front = front.as_ref().map(|w| FrontWindow {
                full_screen: w.styleMask().contains(NSWindowStyleMask::FullScreen),
            });
            desktop::open_folder_as(pref, front)
        });
        let place = match (how, front) {
            (Open::Tab, Some(window)) => Place::TabOf(window),
            _ => Place::Cascade,
        };
        self.open_window(Some(dir), place, None);
        NSApplication::sharedApplication(mtm).activate();
    }

    /// The key window if it is ours, else the frontmost of ours (an
    /// inactive app has no key window).
    fn front_window(&self) -> Option<Retained<NSWindow>> {
        let mtm = MainThreadMarker::from(self);
        self.key_window().or_else(|| {
            NSApplication::sharedApplication(mtm)
                .orderedWindows()
                .iter()
                .find(|w| self.view_of(w).is_some())
        })
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
            let view = ours(w);
            SavedWindow {
                cwd: view.as_ref().and_then(|v| v.foreground_cwd()),
                frame: [f.origin.x, f.origin.y, f.size.width, f.size.height],
                tab_group,
                selected,
                panes: view.as_ref().and_then(|v| v.saved_panes()),
                command: view.as_ref().and_then(|v| v.command_to_restore()),
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

    /// Reopen saved windows, tabs as tabs, with the same tab in front and
    /// the same panes, and type the command each pane was running at its
    /// shell's prompt.
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
                // With panes, each pane types its own command.
                let command = w.command.filter(|_| w.panes.is_none());
                let opened = self.open_window_as(w.cwd, place, None, false, w.panes);
                if let (Some(command), Some(view)) = (command, opened.as_ref().and_then(|o| self.view_of(o))) {
                    view.type_at_prompt(command);
                }
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

    /// Terminal Decline > Settings…: the Settings tab to the front, else a
    /// new one in the front window running the preferences.
    fn show_settings(&self) {
        let open = self.ivars().settings.borrow().clone();
        if let Some(window) = open {
            window.makeKeyAndOrderFront(None);
            if let Some(view) = self.view_of(&window) {
                view.focus_settings();
            }
            let mtm = MainThreadMarker::from(self);
            NSApplication::sharedApplication(mtm).activate();
            crate::view::sync_visibility_soon();
            return;
        }
        let place = match self.front_window() {
            Some(window) => Place::TabOf(window),
            None => Place::Cascade,
        };
        let opened = self.open_window_as(None, place, None, true, None);
        *self.ivars().settings.borrow_mut() = opened;
    }

    /// Bytes typed into the Settings tab (test-only `type-settings`).
    fn type_into_settings(&self, bytes: Vec<u8>) {
        let window = self.ivars().settings.borrow().clone();
        if let Some(view) = window.and_then(|w| self.view_of(&w)) {
            view.type_bytes(bytes);
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

    fn key_view(&self) -> Option<Retained<WindowView>> {
        self.view_of(&*self.key_window()?)
    }

    /// The key window, if it is one of ours.
    fn key_window(&self) -> Option<Retained<NSWindow>> {
        let mtm = MainThreadMarker::from(self);
        let key = NSApplication::sharedApplication(mtm).keyWindow()?;
        self.view_of(&key).map(|_| key)
    }

    fn view_of(&self, window: &NSWindow) -> Option<Retained<WindowView>> {
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
        self.open_window_as(cwd, place, argv, false, None)
    }

    /// [`Self::open_window`], or with `settings` the Settings tab: the
    /// main app's binary running the preferences directly (not through
    /// the login shell), titled "Settings", opaque, without notices. With
    /// `layout`, the panes saved for a restart.
    fn open_window_as(
        &self,
        cwd: Option<String>,
        place: Place,
        argv: Option<Vec<String>>,
        settings: bool,
        layout: Option<crate::reload::SavedPanes>,
    ) -> Option<Retained<NSWindow>> {
        let mtm = MainThreadMarker::from(self);
        let config = self.ivars().config.borrow().clone();
        let scale = NSScreen::mainScreen(mtm).map(|s| s.backingScaleFactor()).unwrap_or(2.0);
        // Shared with every window whose fonts are the same.
        let renderer = Renderer::with_font_config(&config.font_config(config.font_size), scale);
        let metrics = renderer.metrics();
        if !self.ivars().fonts_checked.replace(true) && config.names_fonts() {
            // Fonts the config names but that aren't installed: shown with
            // the other config notices, in this window and later ones.
            let fonts = Fonts::with_config(&config.font_config(config.font_size), scale);
            let missing = config.missing_font_notices(fonts.missing());
            for n in &missing {
                eprintln!("terminal-decline: {n}");
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
        let title = match settings {
            true => "Settings".into(),
            false => config.title.clone().unwrap_or_else(|| "Terminal Decline".into()),
        };
        window.setTitle(&NSString::from_str(&title));

        let titlebar = window.frame().size.height - window.contentLayoutRect().size.height;
        let size = content_size(&config, metrics, scale, titlebar.max(0.0));
        window.setContentSize(size);
        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), size);

        let view = WindowView::new(mtm, bounds);
        view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
        if !settings {
            apply_opacity(&window, &config);
        }
        if config.opacity < 1.0 && config.blur && !settings {
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
        if let Some(pane) = view.first_responder() {
            window.makeFirstResponder(Some(&pane));
        }
        // Mouse moves come through each pane's tracking area: the window's
        // own would send them all to the first responder, wherever the
        // pointer is.

        let mut command = match settings {
            true => Command {
                argv: std::iter::once(prefs_binary().display().to_string())
                    .chain(self.ivars().origin.prefs_args())
                    .collect(),
                direct: true,
                ..Command::default()
            },
            false => Command {
                argv: argv.unwrap_or_else(|| config.command.clone()),
                cwd: cwd.or_else(|| config.cwd.clone()),
                env: config.env.clone().into_iter().collect(),
                shell: None,
                direct: false,
            },
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
            eprintln!("terminal-decline: window {}", window.windowNumber());
        }

        if let Err(e) = view.start(
            config,
            Launch {
                command,
                renderer,
                scale,
                notices: match settings {
                    true => Rc::from([]),
                    false => self.ivars().notices.borrow().clone(),
                },
                settings,
                layout,
            },
        ) {
            eprintln!("terminal-decline: could not open a window: {e}");
            window.close();
            return None;
        }
        // A restored layout has panes of its own: its focused one gets
        // the keys.
        if let Some(pane) = view.first_responder() {
            window.makeFirstResponder(Some(&pane));
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

/// The binary the Settings tab runs: the main app's executable. A stub
/// pin knows the app it loaded the library from; a clone pin finds the
/// app by its bundle id; the app and the bare binary run themselves.
fn prefs_binary() -> std::path::PathBuf {
    let exe = std::env::current_exe()
        .and_then(|e| e.canonicalize())
        .unwrap_or_else(|_| "terminal-decline".into());
    let paths = crate::Paths::current();
    if let Some(app) = &paths.app_bundle {
        let own = app.join("Contents/MacOS/terminal-decline");
        if own.is_file() {
            return own;
        }
    }
    if crate::pin::own_pin_config(&paths).is_none() {
        return exe;
    }
    let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
    workspace
        .URLForApplicationWithBundleIdentifier(&NSString::from_str(crate::BUNDLE_ID))
        .and_then(|u| u.path())
        .map(|app| std::path::PathBuf::from(app.to_string()).join("Contents/MacOS/terminal-decline"))
        .filter(|p| p.is_file())
        .unwrap_or(exe)
}

/// A window lets the desktop show through when `opacity` is below 1.
fn apply_opacity(window: &NSWindow, config: &Config) {
    let opaque = config.opacity >= 1.0;
    if window.isOpaque() != opaque {
        window.setOpaque(opaque);
        let color = if opaque {
            NSColor::windowBackgroundColor()
        } else {
            NSColor::clearColor()
        };
        window.setBackgroundColor(Some(&color));
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

/// The Settings pane of `window` closed, leaving other panes: Cmd+,
/// opens a new Settings tab from now on.
pub(crate) fn settings_closed(window: &NSWindow) {
    let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
    if let Some(delegate) = delegate {
        let mut settings = delegate.ivars().settings.borrow_mut();
        if settings.as_deref().is_some_and(|s| std::ptr::eq::<NSWindow>(s, window)) {
            *settings = None;
        }
    }
}

/// Write the windows as a restart would save them to `path`, as JSON
/// (the test-only `save` control command).
pub(crate) fn write_saved_windows(path: &std::path::Path) {
    let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
    if let Some(delegate) = delegate {
        let json = serde_json::to_string(&delegate.saved_windows()).expect("saved windows serialise");
        if let Err(e) = std::fs::write(path, json) {
            eprintln!("terminal-decline: writing {}: {e}", path.display());
        }
    }
}

/// Whether the app holds secure keyboard input (for the test dumps).
pub(crate) fn secure_input_held() -> bool {
    let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
    delegate.is_some_and(|d| d.ivars().secure.borrow().held())
}

/// Quit without asking (the test-only `quit` control command).
pub(crate) fn quit_now() {
    let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
    if let Some(delegate) = delegate {
        delegate.quit_without_asking();
    }
}

/// Terminal Decline > Settings… (the test-only `settings` control command).
pub(crate) fn show_settings_now() {
    let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
    if let Some(delegate) = delegate {
        delegate.show_settings();
    }
}

/// Whether the Settings tab's preferences read keys, and whether it is
/// recording a chord (for test dumps).
pub(crate) fn settings_status() -> (bool, bool) {
    let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
    let Some(delegate) = delegate else { return (false, false) };
    let window = delegate.ivars().settings.borrow().clone();
    window
        .and_then(|w| delegate.view_of(&w))
        .map_or((false, false), |v| v.settings_status())
}

/// Type into the Settings tab (the test-only `type-settings` command).
pub(crate) fn type_into_settings(bytes: Vec<u8>) {
    let delegate = DELEGATE.with(|d| d.borrow().as_ref().and_then(|w| w.load()));
    if let Some(delegate) = delegate {
        delegate.type_into_settings(bytes);
    }
}

/// The folders to open for `paths` (files open in their folder), each
/// once, in order.
fn folders_of(paths: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut folders: Vec<String> = Vec::new();
    for p in paths {
        if let Some(dir) = desktop::folder_for(std::path::Path::new(&p)) {
            let dir = dir.to_string_lossy().into_owned();
            if !folders.contains(&dir) {
                folders.push(dir);
            }
        }
    }
    folders
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

/// A menu item for `action`, showing the first chord bound to it, plus
/// hidden items for any other chords (their keys still work, as Cmd+1…9
/// do).
fn bound(mtm: MainThreadMarker, title: &str, action: MenuAction, binds: &Keybinds) -> Vec<Retained<NSMenuItem>> {
    let sel = Some(Sel::register(action.selector()));
    let chords = binds.chords(action);
    let mut items = vec![item(mtm, title, sel, "")];
    for (i, chord) in chords.iter().enumerate() {
        let it = if i == 0 {
            items[0].clone()
        } else {
            let hidden = item(mtm, title, sel, "");
            hidden.setHidden(true);
            hidden.setAllowsKeyEquivalentWhenHidden(true);
            items.push(hidden.clone());
            hidden
        };
        let (key, mask) = chord.key_equivalent();
        it.setKeyEquivalent(&NSString::from_str(&key));
        it.setKeyEquivalentModifierMask(NSEventModifierFlags(mask));
    }
    items
}

/// A menu item whose shortcut isn't configurable (`chord`, as `"cmd+q"`),
/// unless the config binds that chord to something else: then it has none.
fn fixed(mtm: MainThreadMarker, title: &str, action: Sel, chord: &str, binds: &Keybinds) -> Retained<NSMenuItem> {
    let it = item(mtm, title, Some(action), "");
    if binds.keeps_fixed(chord)
        && let Ok(c) = crate::keybind::Chord::parse(chord)
    {
        let (key, mask) = c.key_equivalent();
        it.setKeyEquivalent(&NSString::from_str(&key));
        it.setKeyEquivalentModifierMask(NSEventModifierFlags(mask));
    }
    it
}

/// The standard Services submenu, which macOS fills.
fn services_item(mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Services"));
    let holder = item(mtm, "Services", None, "");
    holder.setSubmenu(Some(&menu));
    NSApplication::sharedApplication(mtm).setServicesMenu(Some(&menu));
    holder
}

/// The menu bar. macOS needs one; these are the standard items plus the
/// terminal's few shortcuts, with the keys `binds` gives them.
pub fn build_menus(mtm: MainThreadMarker, app_name: &str, binds: &Keybinds) -> Retained<NSMenu> {
    let bar = NSMenu::new(mtm);
    let b = |title: &str, action: MenuAction| bound(mtm, title, action, binds);
    let f = |title: &str, action: Sel, chord: &str| fixed(mtm, title, action, chord, binds);
    submenu(
        mtm,
        &bar,
        app_name,
        &[
            b(MenuAction::Settings.label(), MenuAction::Settings),
            vec![
                item(mtm, "Edit config.toml", Some(sel!(editConfig:)), ""),
                NSMenuItem::separatorItem(mtm),
                item(mtm, "Secure Keyboard Entry", Some(sel!(toggleSecureKeyboardEntry:)), ""),
            ],
            vec![
                NSMenuItem::separatorItem(mtm),
                services_item(mtm),
                NSMenuItem::separatorItem(mtm),
                f(&format!("Hide {app_name}"), sel!(hide:), "cmd+h"),
                f("Hide Others", sel!(hideOtherApplications:), "alt+cmd+h"),
                NSMenuItem::separatorItem(mtm),
                f(&format!("Quit {app_name}"), sel!(terminate:), "cmd+q"),
            ],
        ]
        .concat(),
    );
    let shell = [
        b("New Window", MenuAction::NewWindow),
        b("New Tab", MenuAction::NewTab),
        vec![NSMenuItem::separatorItem(mtm)],
        b(MenuAction::SplitRight.label(), MenuAction::SplitRight),
        b(MenuAction::SplitDown.label(), MenuAction::SplitDown),
        vec![NSMenuItem::separatorItem(mtm)],
        b("Close", MenuAction::Close),
        b(MenuAction::CloseTab.label(), MenuAction::CloseTab),
        b(MenuAction::CloseWindow.label(), MenuAction::CloseWindow),
    ];
    submenu(mtm, &bar, "Shell", &shell.concat());
    let edit = [
        b("Copy", MenuAction::Copy),
        b("Paste", MenuAction::Paste),
        b("Select All", MenuAction::SelectAll),
        b(MenuAction::SelectOutput.label(), MenuAction::SelectOutput),
        vec![NSMenuItem::separatorItem(mtm)],
        b("Clear Scrollback", MenuAction::ClearScrollback),
        vec![NSMenuItem::separatorItem(mtm)],
        // Disabled until the find bar exists: nothing answers its action.
        b("Find…", MenuAction::Find),
    ];
    submenu(mtm, &bar, "Edit", &edit.concat());
    let view = [
        b("Bigger", MenuAction::FontBigger),
        b("Smaller", MenuAction::FontSmaller),
        b("Actual Size", MenuAction::FontReset),
        vec![NSMenuItem::separatorItem(mtm)],
        b(MenuAction::PreviousPrompt.label(), MenuAction::PreviousPrompt),
        b(MenuAction::NextPrompt.label(), MenuAction::NextPrompt),
        vec![NSMenuItem::separatorItem(mtm)],
        b("Enter Full Screen", MenuAction::ToggleFullScreen),
    ];
    submenu(mtm, &bar, "View", &view.concat());
    // AppKit's own tab actions; with these in the menu it adds no copies.
    let mut window_items = [
        vec![
            f("Minimize", sel!(performMiniaturize:), "cmd+m"),
            item(mtm, "Zoom", Some(sel!(performZoom:)), ""),
            NSMenuItem::separatorItem(mtm),
        ],
        b(MenuAction::FocusPanePrevious.label(), MenuAction::FocusPanePrevious),
        b(MenuAction::FocusPaneNext.label(), MenuAction::FocusPaneNext),
        b(MenuAction::FocusPaneLeft.label(), MenuAction::FocusPaneLeft),
        b(MenuAction::FocusPaneRight.label(), MenuAction::FocusPaneRight),
        b(MenuAction::FocusPaneUp.label(), MenuAction::FocusPaneUp),
        b(MenuAction::FocusPaneDown.label(), MenuAction::FocusPaneDown),
        vec![NSMenuItem::separatorItem(mtm)],
        b(MenuAction::ResizePaneLeft.label(), MenuAction::ResizePaneLeft),
        b(MenuAction::ResizePaneRight.label(), MenuAction::ResizePaneRight),
        b(MenuAction::ResizePaneUp.label(), MenuAction::ResizePaneUp),
        b(MenuAction::ResizePaneDown.label(), MenuAction::ResizePaneDown),
        b(MenuAction::EqualizePanes.label(), MenuAction::EqualizePanes),
        b(MenuAction::TogglePaneZoom.label(), MenuAction::TogglePaneZoom),
        vec![NSMenuItem::separatorItem(mtm)],
        b("Show Previous Tab", MenuAction::PreviousTab),
        b("Show Next Tab", MenuAction::NextTab),
        vec![
            item(mtm, "Move Tab to New Window", Some(sel!(moveTabToNewWindow:)), ""),
            item(mtm, "Merge All Windows", Some(sel!(mergeAllWindows:)), ""),
        ],
    ]
    .concat();
    // Cmd+1…9: hidden, as in Safari, but their keys still work.
    for n in 1..=9 {
        let select = f(&format!("Show Tab {n}"), sel!(selectTab:), &format!("cmd+{n}"));
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

pub fn run(config: Config, notices: Vec<String>, app_name: &str, restore: Vec<crate::reload::SavedWindow>, origin: crate::config::Source) {
    let mtm = MainThreadMarker::new().expect("must run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    crate::notify::on_click(crate::view::bring_to_front);
    let menus = build_menus(mtm, app_name, &config.keybinds());
    let delegate = AppDelegate::new(mtm, config, notices, restore, origin, app_name);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.setMainMenu(Some(&menus));
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
