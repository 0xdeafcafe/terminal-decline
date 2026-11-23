//! A window's tabs (docs/design/tabs.md §5): the window's content view
//! and its delegate, holding a [`WindowView`] for each tab, all out of
//! sight but the selected one, with the tab bar ([`TabBar`]) over their title
//! row while there are two or more. A tab is a view, not a window of its
//! own: macOS's window tabs are turned off for our windows.
//!
//! A tab behind is moved out of the window, not hidden: a hidden view's
//! display link stops, and with it the frame a test's dump waits for and
//! the idle bookkeeping a tab does as a window behind another did.
//!
//! What the window tells its delegate goes to the selected tab (key,
//! covered and uncovered) or to them all (full screen, closing).

use std::cell::{Cell, RefCell};

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSResponder, NSView, NSWindow, NSWindowDelegate};
use objc2_foundation::{NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use crate::tab_bar::TabBar;
use crate::view::WindowView;

thread_local! {
    /// Every window's host, to find a window's (main thread).
    static HOSTS: RefCell<Vec<Weak<TabHost>>> = const { RefCell::new(Vec::new()) };
}

/// How far left of the window a tab behind is kept, past its own width.
const BEHIND: f64 = 64.0;

pub struct Ivars {
    /// The tabs, in the bar's order.
    tabs: RefCell<Vec<Retained<WindowView>>>,
    selected: Cell<usize>,
    bar: RefCell<Option<Retained<TabBar>>>,
}

define_class!(
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeclineTabHost"]
    #[ivars = Ivars]
    pub struct TabHost;

    impl TabHost {
        /// Every tab fills the host, and the bar its title row.
        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.lay_out();
        }

        /// Window > Show Next Tab (Cmd+Shift+]), from the tab with the
        /// keys up to here: around to the first after the last.
        #[unsafe(method(selectNextTab:))]
        fn select_next_tab(&self, _sender: Option<&AnyObject>) {
            self.step(1);
        }

        #[unsafe(method(selectPreviousTab:))]
        fn select_previous_tab(&self, _sender: Option<&AnyObject>) {
            self.step(-1);
        }
    }

    unsafe impl NSObjectProtocol for TabHost {}

    unsafe impl NSWindowDelegate for TabHost {
        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _n: &NSNotification) {
            if let Some(tab) = self.selected() {
                tab.became_key();
            }
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _n: &NSNotification) {
            if let Some(tab) = self.selected() {
                tab.resigned_key();
            }
        }

        #[unsafe(method(windowDidChangeOcclusionState:))]
        fn window_did_change_occlusion(&self, _n: &NSNotification) {
            for tab in self.views() {
                tab.occlusion_changed();
            }
        }

        /// The window's close button and `performClose:`: every tab,
        /// asking once while programs run in any of them.
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, window: &NSWindow) -> bool {
            match self.selected() {
                Some(tab) => tab.window_should_close(window),
                None => true,
            }
        }

        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _n: &NSNotification) {
            for tab in self.views() {
                tab.shut_down();
            }
            let me: *const TabHost = self;
            HOSTS.with(|h| h.borrow_mut().retain(|w| w.load().is_some_and(|o| !std::ptr::eq(&*o, me))));
        }

        #[unsafe(method(windowWillEnterFullScreen:))]
        fn window_will_enter_full_screen(&self, _n: &NSNotification) {
            for tab in self.views() {
                tab.will_enter_full_screen();
            }
        }

        #[unsafe(method(windowDidExitFullScreen:))]
        fn window_did_exit_full_screen(&self, _n: &NSNotification) {
            for tab in self.views() {
                tab.did_exit_full_screen();
            }
        }
    }
);

impl TabHost {
    /// A host with no tabs yet, `frame` in its window.
    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<TabHost> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            tabs: RefCell::new(Vec::new()),
            selected: Cell::new(0),
            bar: RefCell::new(None),
        });
        let host: Retained<TabHost> = unsafe { msg_send![super(this), initWithFrame: frame] };
        let bar = TabBar::new(mtm, &host);
        host.addSubview(&bar);
        *host.ivars().bar.borrow_mut() = Some(bar);
        HOSTS.with(|h| h.borrow_mut().push(Weak::from_retained(&host)));
        host
    }

    /// `window`'s host, if it is one of our windows.
    pub fn of(window: &NSWindow) -> Option<Retained<TabHost>> {
        let hosts: Vec<Retained<TabHost>> = HOSTS.with(|h| h.borrow().iter().filter_map(|w| w.load()).collect());
        hosts
            .into_iter()
            .find(|h| h.window().is_some_and(|w| std::ptr::eq::<NSWindow>(&*w, window)))
    }

    /// The host `view` is a tab of.
    pub fn of_view(view: &WindowView) -> Option<Retained<TabHost>> {
        let hosts: Vec<Retained<TabHost>> = HOSTS.with(|h| h.borrow().iter().filter_map(|w| w.load()).collect());
        hosts.into_iter().find(|h| h.index_of(view).is_some())
    }

    /// The tabs, in order.
    pub fn views(&self) -> Vec<Retained<WindowView>> {
        self.ivars().tabs.borrow().clone()
    }

    pub fn count(&self) -> usize {
        self.ivars().tabs.borrow().len()
    }

    /// The tab in front.
    pub fn selected(&self) -> Option<Retained<WindowView>> {
        self.ivars().tabs.borrow().get(self.ivars().selected.get()).cloned()
    }

    pub fn selected_index(&self) -> usize {
        self.ivars().selected.get()
    }

    pub fn index_of(&self, view: &WindowView) -> Option<usize> {
        self.ivars()
            .tabs
            .borrow()
            .iter()
            .position(|v| std::ptr::eq::<WindowView>(&**v, view))
    }

    /// Whether `view` is the tab in front.
    pub fn is_selected(&self, view: &WindowView) -> bool {
        self.index_of(view) == Some(self.ivars().selected.get())
    }

    /// Add `view` as a tab after the selected one (the first: as the
    /// only one), behind the others until it is selected.
    pub fn add(&self, view: &WindowView) {
        let iv = self.ivars();
        let at = {
            let mut tabs = iv.tabs.borrow_mut();
            let at = if tabs.is_empty() {
                0
            } else {
                (iv.selected.get() + 1).min(tabs.len())
            };
            tabs.insert(at, objc2::Message::retain(view));
            at
        };
        // The selected tab is the same one as before.
        if at <= iv.selected.get() && self.count() > 1 {
            iv.selected.set(iv.selected.get() + 1);
        }
        view.setFrame(self.place(self.count() == 1));
        // Under the bar.
        self.addSubview_positioned_relativeTo(view, objc2_app_kit::NSWindowOrderingMode::Below, None);
        self.refresh_bar();
    }

    /// Bring tab `index` to the front: it alone shows, has the keys while
    /// the window does, and gives the window its title.
    pub fn select(&self, index: usize) {
        let tabs = self.views();
        let Some(new) = tabs.get(index) else { return };
        let old = self.ivars().selected.replace(index);
        if old != index
            && let Some(old) = tabs.get(old)
        {
            old.put_behind();
        }
        self.place_tabs();
        if let Some(window) = self.window() {
            if let Some(pane) = new.first_responder() {
                window.makeFirstResponder(Some(&pane));
            }
            window.setTitle(&NSString::from_str(&new.title()));
            if crate::background::is_key(&window) {
                new.became_key();
            }
        }
        for tab in &tabs {
            tab.sync_visibility();
        }
        self.lay_out();
    }

    /// Bring `view`'s tab to the front.
    pub fn select_view(&self, view: &WindowView) {
        if let Some(i) = self.index_of(view) {
            self.select(i);
        }
    }

    /// The next tab (`by` 1) or the one before (-1), around the ends.
    fn step(&self, by: isize) {
        let n = self.count() as isize;
        if n > 1 {
            self.select((self.ivars().selected.get() as isize + by).rem_euclid(n) as usize);
        }
    }

    /// The bar's `+`: a tab after the selected one, in its folder.
    pub fn new_tab(&self) {
        if let Some(window) = self.window() {
            crate::app::open_tab_beside(&window);
        }
    }

    /// A middle click on tab `index`: close it, asking first while a
    /// program runs in it.
    pub fn close_asking(&self, index: usize) {
        if let Some(tab) = self.views().get(index) {
            tab.close_tab_asking();
        }
    }

    /// Close `view`'s tab, ending what runs in it; the tab beside it
    /// comes to the front. Not for the last tab: the window closes then.
    pub fn close(&self, view: &WindowView) {
        let Some(at) = self.index_of(view) else { return };
        let selected = self.ivars().selected.get();
        view.shut_down();
        view.removeFromSuperview();
        self.ivars().tabs.borrow_mut().remove(at);
        crate::app::tab_closed(view);
        let n = self.count();
        if n == 0 {
            return;
        }
        // The tab that took its place, else the one before; a tab closed
        // behind the selected one changes nothing but the index.
        let next = if at < selected { selected - 1 } else { selected.min(n - 1) };
        if at == selected {
            // Nothing is selected until `select` says so.
            self.ivars().selected.set(usize::MAX);
        }
        self.select(next);
    }

    /// Every tab at the host's size, and the bar across the title row
    /// with what each tab says now (hidden with one tab, and with no
    /// title row: full screen).
    pub fn lay_out(&self) {
        self.place_tabs();
        self.refresh_bar();
    }

    /// Where a tab goes: the selected one fills the host, the others
    /// are the same size beside it, out of the window.
    fn place(&self, selected: bool) -> NSRect {
        let bounds = self.bounds();
        match selected {
            true => bounds,
            false => NSRect::new(NSPoint::new(-(bounds.size.width + BEHIND), 0.0), bounds.size),
        }
    }

    fn place_tabs(&self) {
        let selected = self.ivars().selected.get();
        for (i, tab) in self.views().iter().enumerate() {
            let (now, to) = (tab.frame(), self.place(i == selected));
            if now.size.width != to.size.width || now.size.height != to.size.height {
                tab.setFrame(to);
            } else if now.origin.x != to.origin.x || now.origin.y != to.origin.y {
                tab.setFrameOrigin(to.origin);
            }
        }
    }

    /// The bar, as the tabs are now.
    pub fn refresh_bar(&self) {
        let Some(bar) = self.ivars().bar.borrow().clone() else { return };
        let tabs = self.views();
        let selected = self.ivars().selected.get();
        let row = tabs.first().map_or(0.0, |t| t.titlebar_height());
        let look = tabs.get(selected).and_then(|t| t.look());
        let (true, Some(look)) = (tabs.len() > 1 && row > 0.0, look) else {
            bar.hide();
            return;
        };
        let bounds = self.bounds();
        // The host isn't flipped: the title row is at its top.
        let frame = NSRect::new(NSPoint::new(0.0, bounds.size.height - row), NSSize::new(bounds.size.width, row));
        let says: Vec<_> = tabs.iter().map(|t| crate::tab_bar::label(&t.tab_text())).collect();
        bar.show(frame, &says, selected, look);
    }

    /// What the bar's tabs say (for the test dumps); `None` while hidden.
    pub fn bar_texts(&self) -> Option<Vec<String>> {
        self.ivars().bar.borrow().as_ref().and_then(|b| b.texts())
    }
}

pub fn delegate_of(host: &TabHost) -> &ProtocolObject<dyn NSWindowDelegate> {
    ProtocolObject::from_ref(host)
}
