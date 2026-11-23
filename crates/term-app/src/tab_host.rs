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
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSMenuItem, NSResponder, NSView, NSWindow, NSWindowDelegate};
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
    /// The tab a press in the bar is dragging, where it is now.
    dragging: Cell<Option<usize>>,
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

        /// Window > Move Tab to New Window: the tab in front.
        #[unsafe(method(moveTabToNewWindow:))]
        fn move_tab_to_new_window(&self, _sender: Option<&AnyObject>) {
            if let Some(tab) = self.selected() {
                crate::app::tab_to_new_window(&tab, None);
            }
        }

        /// Window > Merge All Windows: every window's tabs into this one.
        #[unsafe(method(mergeAllWindows:))]
        fn merge_all_windows(&self, _sender: Option<&AnyObject>) {
            self.merge_all();
        }

        /// Move Tab to New Window needs a second tab to leave behind, and
        /// Merge All Windows another window to take from.
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            let action = item.action();
            if action == Some(sel!(moveTabToNewWindow:)) {
                self.count() > 1
            } else if action == Some(sel!(mergeAllWindows:)) {
                !self.others().is_empty()
            } else {
                true
            }
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
            dragging: Cell::new(None),
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
        let at = if self.count() == 0 {
            0
        } else {
            (self.ivars().selected.get() + 1).min(self.count())
        };
        self.insert(view, at);
    }

    /// `view` as tab `at`, behind the others unless it is the only one.
    fn insert(&self, view: &WindowView, at: usize) {
        let iv = self.ivars();
        let at = at.min(self.count());
        iv.tabs.borrow_mut().insert(at, objc2::Message::retain(view));
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
        if self.index_of(view).is_none() {
            return;
        }
        view.shut_down();
        self.take_out(view);
        crate::app::tab_closed(view);
    }

    /// Take `view`'s tab out, running as it is, for another window
    /// ([`TabHost::adopt`]); the tab beside it comes to the front.
    pub fn detach(&self, view: &WindowView) {
        self.take_out(view);
    }

    fn take_out(&self, view: &WindowView) {
        let Some(at) = self.index_of(view) else { return };
        let selected = self.ivars().selected.get();
        self.ivars().dragging.set(None);
        view.removeFromSuperview();
        self.ivars().tabs.borrow_mut().remove(at);
        let n = self.count();
        if n == 0 {
            return;
        }
        // The tab that took its place, else the one before; a tab taken
        // from behind the selected one changes nothing but the index.
        let next = if at < selected { selected - 1 } else { selected.min(n - 1) };
        if at == selected {
            // Nothing is selected until `select` says so.
            self.ivars().selected.set(usize::MAX);
        }
        self.select(next);
    }

    /// Take in a tab from another window ([`TabHost::detach`]) as the
    /// last one, in front if `front`.
    pub fn adopt(&self, view: &WindowView, front: bool) {
        self.insert(view, self.count());
        view.moved_to_window();
        if front || self.count() == 1 {
            self.select_view(view);
        }
    }

    /// Move tab `from` to place `to`; the tab in front stays in front.
    pub fn move_tab(&self, from: usize, to: usize) {
        let front = self.selected();
        {
            let mut tabs = self.ivars().tabs.borrow_mut();
            if from == to || from >= tabs.len() || to >= tabs.len() {
                return;
            }
            let tab = tabs.remove(from);
            tabs.insert(to, tab);
        }
        if let Some(front) = front
            && let Some(i) = self.index_of(&front)
        {
            self.ivars().selected.set(i);
        }
        self.refresh_bar();
    }

    /// The other windows' hosts, the quick terminal's left out.
    fn others(&self) -> Vec<Retained<TabHost>> {
        let me: *const TabHost = self;
        let hosts: Vec<Retained<TabHost>> = HOSTS.with(|h| h.borrow().iter().filter_map(|w| w.load()).collect());
        hosts
            .into_iter()
            .filter(|h| !std::ptr::eq(&**h, me) && h.count() > 0)
            .filter(|h| h.window().is_some_and(|w| !crate::app::is_quick_window(&w)))
            .collect()
    }

    /// Every other window's tabs become tabs of this one, after its own,
    /// and those windows close. A window showing a question is left.
    pub fn merge_all(&self) {
        if self.window().is_none_or(|w| crate::app::is_quick_window(&w)) {
            return;
        }
        for other in self.others() {
            let Some(window) = other.window() else { continue };
            if window.attachedSheet().is_some() {
                continue;
            }
            for tab in other.views() {
                self.give(&tab, false);
            }
            window.close();
        }
    }

    /// Move `view`'s tab from the window it is in to this one.
    fn give(&self, view: &WindowView, front: bool) {
        let (Some(from), Some(window)) = (TabHost::of_view(view), self.window()) else {
            return;
        };
        from.detach(view);
        self.adopt(view, front);
        crate::app::tab_moved(view, &window);
    }

    /// A press on tab `index` in the bar: it may be dragged from here.
    pub fn drag_from(&self, index: usize) {
        self.ivars().dragging.set(Some(index));
    }

    /// The pressed tab is dragged over place `to` in the bar: it moves
    /// there, the others making room.
    pub fn drag_over(&self, to: usize) {
        if let Some(from) = self.ivars().dragging.get()
            && from != to
            && to < self.count()
        {
            self.move_tab(from, to);
            self.ivars().dragging.set(Some(to));
        }
    }

    /// The press ended; with `outside`, off the bar at that point of the
    /// screen: over another window's title row the tab joins that
    /// window, anywhere else it becomes a window there.
    pub fn drag_ended(&self, outside: Option<NSPoint>) {
        let Some(index) = self.ivars().dragging.take() else { return };
        let (Some(at), Some(tab)) = (outside, self.views().get(index).cloned()) else {
            return;
        };
        let over = self.others().into_iter().find(|h| {
            h.window().is_some_and(|w| {
                let f = w.frame();
                let row = h.selected().map_or(0.0, |t| t.titlebar_height());
                at.x >= f.origin.x
                    && at.x < f.origin.x + f.size.width
                    && at.y <= f.origin.y + f.size.height
                    && at.y > f.origin.y + f.size.height - row
            })
        });
        match over {
            Some(host) => {
                let last = self.count() == 1;
                let mine = self.window();
                host.give(&tab, true);
                if let Some(window) = host.window() {
                    crate::background::make_key_and_order_front(&window);
                }
                if let (true, Some(mine)) = (last, mine) {
                    mine.close();
                }
            }
            None => crate::app::tab_to_new_window(&tab, Some(at)),
        }
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
