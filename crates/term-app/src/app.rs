//! Application: delegate, menus, and opening windows.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSAutoresizingMaskOptions, NSBackingStoreType, NSColor,
    NSEventModifierFlags, NSMenu, NSMenuItem, NSScreen, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState,
    NSVisualEffectView, NSWindow, NSWindowStyleMask, NSWindowTabbingMode, NSWindowTitleVisibility,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSString};
use term_font::Fonts;
use term_pty::Command;

use crate::config::Config;
use crate::view::{Launch, TerminalView, content_size, delegate_of};

pub struct AppIvars {
    config: Rc<Config>,
    /// Config errors, shown at the top of each window.
    notices: Rc<[String]>,
    windows: RefCell<Vec<(Retained<NSWindow>, Retained<TerminalView>)>>,
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
            self.open_window(None);
            let mtm = MainThreadMarker::from(self);
            #[allow(deprecated)]
            NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        }

        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(&self, _app: &NSApplication, has_visible: bool) -> bool {
            // Dock icon clicked with no windows open: open one.
            if !has_visible {
                self.open_window(None);
            }
            true
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _app: &NSApplication) -> bool {
            true
        }
    }

    impl AppDelegate {
        #[unsafe(method(newWindow:))]
        fn new_window(&self, _sender: Option<&AnyObject>) {
            // Open in the same directory as the window in front.
            let cwd = self.key_view().and_then(|v| v.foreground_cwd());
            self.open_window(cwd);
        }

        #[unsafe(method(windowClosed:))]
        fn window_closed(&self, n: &NSNotification) {
            let closed = n.object();
            self.ivars().windows.borrow_mut().retain(|(w, _)| closed.as_deref().map(|o| !std::ptr::eq(o, w.as_ref() as &AnyObject)).unwrap_or(true));
        }
    }
);

impl AppDelegate {
    pub fn new(mtm: MainThreadMarker, config: Config, notices: Vec<String>) -> Retained<AppDelegate> {
        let this = Self::alloc(mtm).set_ivars(AppIvars {
            config: Rc::new(config),
            notices: notices.into(),
            windows: RefCell::new(Vec::new()),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn key_view(&self) -> Option<Retained<TerminalView>> {
        let mtm = MainThreadMarker::from(self);
        let key = NSApplication::sharedApplication(mtm).keyWindow()?;
        self.ivars()
            .windows
            .borrow()
            .iter()
            .find(|(w, _)| std::ptr::eq::<NSWindow>(&**w, &*key))
            .map(|(_, v)| v.clone())
    }

    fn open_window(&self, cwd: Option<String>) {
        let mtm = MainThreadMarker::from(self);
        let config = self.ivars().config.clone();
        let scale = NSScreen::mainScreen(mtm).map(|s| s.backingScaleFactor()).unwrap_or(2.0);
        let fonts = Fonts::new(&config.font_family, config.font_size * scale);
        let metrics = fonts.metrics;

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
            argv: config.command.clone(),
            cwd: cwd.or_else(|| config.cwd.clone()),
            env: config.env.clone().into_iter().collect(),
            shell: None,
        };
        if let Some(dir) = &command.cwd {
            command.cwd = Some(expand_home(dir));
        }
        // Cascade from the window in front, otherwise centre.
        match self.ivars().windows.borrow().last() {
            Some((prev, _)) => {
                let f = prev.frame();
                window.cascadeTopLeftFromPoint(NSPoint::new(f.origin.x, f.origin.y + f.size.height));
                let _ = window.cascadeTopLeftFromPoint(NSPoint::new(f.origin.x + 22.0, f.origin.y + f.size.height - 22.0));
            }
            None => window.center(),
        }
        window.makeKeyAndOrderFront(None);
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
                notices: self.ivars().notices.clone(),
            },
        ) {
            eprintln!("decline: could not open a window: {e}");
            window.close();
            return;
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
        self.ivars().windows.borrow_mut().push((window, view));
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
    let hide_others = item(mtm, "Hide Others", Some(sel!(hideOtherApplications:)), "h");
    hide_others.setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Option);
    submenu(
        mtm,
        &bar,
        app_name,
        &[
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
            item(mtm, "Enter Full Screen", Some(sel!(toggleFullScreen:)), "f"),
        ],
    );
    let window_menu = submenu(
        mtm,
        &bar,
        "Window",
        &[
            item(mtm, "Minimize", Some(sel!(performMiniaturize:)), "m"),
            item(mtm, "Zoom", Some(sel!(performZoom:)), ""),
        ],
    );
    let app = NSApplication::sharedApplication(mtm);
    app.setWindowsMenu(Some(&window_menu));
    if let Some(full) = bar
        .itemArray()
        .iter()
        .nth(4)
        .and_then(|i| i.submenu())
        .and_then(|m| m.itemArray().iter().last())
    {
        full.setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Control);
    }
    bar
}

pub fn run(config: Config, notices: Vec<String>, app_name: &str) {
    let mtm = MainThreadMarker::new().expect("must run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    let delegate = AppDelegate::new(mtm, config, notices);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.setMainMenu(Some(&build_menus(mtm, app_name)));
    app.run();
}
