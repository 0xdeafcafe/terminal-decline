//! The window view: one per window (or tab). Its backing layer is the
//! window's only `CAMetalLayer`, with one display link, and it draws every
//! pane of the window in one frame (splits design §1). The panes are
//! [`PaneView`]s laid over it, which take the keys, the input method and
//! the mouse; each owns a session (program, PTY and terminal). Panes are
//! split, closed, focused, resized and zoomed here; the window view draws
//! the 1 pt dividers between them and sets the cursor over them, and a
//! [`DividerView`] over each takes the drags that move it.

use std::cell::{Cell as StdCell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchTime};
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSBeep, NSCursor, NSEvent, NSEventMask, NSEventModifierFlags, NSMenuItem,
    NSModalResponse, NSPasteboard, NSPasteboardTypeString, NSResponder, NSView, NSWindow, NSWindowDelegate, NSWindowOrderingMode,
};
use objc2_foundation::{
    NSArray, NSKeyValueObservingOptions, NSNotification, NSObjectNSKeyValueObserverRegistration, NSObjectProtocol, NSPoint, NSRect,
    NSRunLoop, NSRunLoopCommonModes, NSSize, NSString,
};
use objc2_metal::{MTLCommandBuffer, MTLDrawable};
use objc2_quartz_core::{CADisplayLink, CALayer, CAMetalDrawable, CAMetalLayer};
use term_core::Term;
use term_pty::{Command, Size};
use term_render::transition::{self, Slots};
use term_render::{FrameInput, PaneDraw, PaneRender, RectInstance, Renderer, Rgba, Theme, Transition};

use crate::config::Config;
use crate::control::{self, Control, Dump, FocusTarget, PaneDump};
use crate::divider_view::DividerView;
use crate::frame_policy::{self, Decision, ECHO_WAIT, PaneFrame, WindowTick};
use crate::keybind::{self, Keybinds};
use crate::keys::{KeyInput, KeyModes, KeysDown};
use crate::latency::{self, FrameKeys, Latency};
use crate::layout::{self, CellSize, Padding};
use crate::pane_view::PaneView;
use crate::panes::{Dir, Direction, Divider, Geometry, PaneId, Panes, PxRect, SplitPath};
use crate::reload::{SavedPane, SavedPanes};
use crate::running::Closing;
use crate::select::{Block, Gesture};
use crate::session::{Events, Hooks, Outcome, Session};
use crate::theme::Appearance;

pub(crate) const BLINK_INTERVAL: Duration = Duration::from_millis(530);

/// The time a pane's frame may spend on glyphs it hasn't drawn before;
/// the rest come in the next frames (roadmap R.1). Right after input,
/// under half a 120 Hz refresh, so a flood of new CJK or emoji doesn't
/// hold up typing; otherwise a refresh, so a screenful of them still
/// fills in within a second at 30 Hz (with 3 ms always, it took ~4 s).
pub(crate) const GLYPH_BUDGET_TYPING: Duration = Duration::from_millis(3);
pub(crate) const GLYPH_BUDGET: Duration = Duration::from_millis(8);

/// What the window keeps: its layer's renderer and display link, and what
/// is the same for all its panes (splits design §2.2).
pub(crate) struct Win {
    pub(crate) renderer: Renderer,
    pub(crate) link: Retained<CADisplayLink>,
    pub(crate) config: Rc<Config>,
    /// The shortcuts in effect (`[keybind]`).
    pub(crate) binds: Keybinds,
    pub(crate) theme: Theme,
    /// The window's light or dark appearance, which picks the theme of a
    /// light/dark pair.
    appearance: Appearance,
    pub(crate) scale: f64,
    /// Per window (Cmd+= and Cmd+-), not per pane (splits design §11).
    font_size: f64,
    /// The window is key.
    pub(crate) focused: bool,
    pub(crate) idle_ticks: u8,
    /// Frames presented; lets a delayed buffer trim see if anything was drawn since.
    frames: u64,
    /// Display refresh rate currently asked for (Hz).
    rate: u16,
    /// Last key, paste, scroll or click in any pane: typing gets the full
    /// refresh rate.
    pub(crate) last_input: Instant,
    /// Only with `DECLINE_TEST_DUMP` set: see [`crate::control`].
    test: Option<TestControl>,
    /// The title bar's height, plus the tab bar's while one shows (points),
    /// as of the last resize.
    top_inset: f64,
    /// Times the window was hidden or covered and gave back its drawables.
    hidden: u64,
    /// Whether the window could be seen when last checked
    /// (`sync_visibility`), so hiding and showing happen once each.
    shown: bool,
    /// The image pool was told this window is hidden (images.md §5.5).
    images_hidden: bool,
    /// Bumped each time the title is set: a title clock armed before
    /// does nothing.
    title_clock: u64,
    /// This is the Settings tab, which the app started itself: a fixed
    /// title, always opaque, not a running program for `confirm_close`,
    /// and the only session whose chord recording is honoured.
    pub(crate) settings: bool,
    /// A chord being recorded for the preferences (OSC 7771).
    recording: Option<Recording>,
    /// Bumped by each recording, so a timeout cancels only its own.
    recordings: u64,
    /// The colour of the dividers between panes.
    divider: Rgba,
    /// The theme transition playing, if one is (see
    /// [`term_render::transition`]): the display link runs until it ends.
    transition: Option<Transition>,
    /// The window's `contentMinSize` before its panes raised it, and the
    /// one last set (points).
    base_min: Option<NSSize>,
    min: Option<NSSize>,
}

/// What each pane keeps: its view, its session and what the renderer
/// built for it last, and the input and selection state of its terminal.
pub(crate) struct Pane {
    pub(crate) view: Retained<PaneView>,
    pub(crate) session: Session,
    pub(crate) render: PaneRender,
    /// The window's theme with the colours its program set (OSC 4,
    /// 10–12) on top; `None` while it has set none.
    theme: Option<Theme>,
    /// Where it is in the window, in pixels.
    pub(crate) rect: PxRect,
    pub(crate) cursor_on: bool,
    pub(crate) blink_scheduled: bool,
    sync_since: Option<Instant>,
    pub(crate) marked: String,
    pub(crate) scroll_accum: f64,
    /// Mouse button held while reporting to the program.
    pub(crate) mouse_button: Option<u8>,
    /// The gesture that started the selection while the left button is
    /// held for it.
    pub(crate) selecting: Option<Gesture>,
    /// Whether the selection being made changed since the button went down.
    pub(crate) dragged: bool,
    /// A rectangular selection (Option-drag); see [`crate::select`].
    pub(crate) block: Option<Block>,
    /// A plain or Option click whose selection starts once the mouse
    /// has moved far enough ([`crate::select::Pending`]).
    pub(crate) pending: Option<crate::select::Pending>,
    /// Scrolling while a selection is dragged past the top or bottom edge.
    pub(crate) autoscroll: Option<AutoScroll>,
    /// An auto-scroll step is waiting to run.
    pub(crate) autoscroll_scheduled: bool,
    /// The session has ended, but the pane stays open to show why; a key
    /// closes it.
    pub(crate) held: bool,
    /// The key event being handed to the input method, and the modes it
    /// was translated with, so text it types can be encoded as that key
    /// (`keys::committed_text`).
    pub(crate) text_key: Option<(KeyInput, KeyModes)>,
    /// Keys whose press went to the program, for release events.
    pub(crate) keys_down: KeysDown,
    /// Only with `DECLINE_LATENCY_LOG` set: the key event being handled,
    /// its timestamp and when `keyDown` got it (see [`crate::latency`]).
    pub(crate) key_t0: Option<(f64, f64)>,
    /// When input was last sent and its echo not yet drawn: the first
    /// output after it is drawn at once, not on the next tick.
    pub(crate) awaiting_echo: Option<Instant>,
    /// It runs the login shell, not a program of its own (`-e PROGRAM`, a
    /// pin's, `command` in the config) or the Settings: only then is the
    /// program in the shell's foreground saved for a restart and typed at
    /// the prompt after it (see [`crate::reload::SavedWindow::command`]).
    runs_shell: bool,
    /// It runs the window's own program (the first pane, or one restored
    /// as such), not the user's shell as a split does.
    program: bool,
    /// The size its program was last told (columns, lines and cell
    /// pixels): the PTY is resized only when that changes, not for every
    /// pixel of a divider drag or window resize.
    told: Option<(u16, u16, u16, u16)>,
    /// The backing scale its terminal was last given, for iTerm2 image
    /// sizes in points; set again when the window moves between displays.
    image_scale: f64,
    /// It runs the preferences (the Settings tab's first pane): not a
    /// running program for `confirm_close`, the only pane whose chord
    /// recording is honoured, and titled "Settings".
    pub(crate) settings: bool,
    /// The title its program set last, shown while it has focus.
    title: Option<String>,
    /// The left click that focused it: it goes nowhere else (no
    /// selection, no mouse report) until the button comes up.
    pub(crate) swallow_click: bool,
    /// Times its frame was built (for the test dumps).
    builds: u64,
    /// Building its frame panicked (splits design §7).
    crash: Option<Crash>,
    /// The test-only `build-panic`: its next build panics, taking the
    /// snapshot (`Some(true)`) or building from it.
    panic_next: Option<bool>,
    /// Its find bar and search, while the bar is open (E.7).
    pub(crate) find: Option<crate::find_bar::Find>,
    /// What it last searched for, shown again when the bar reopens.
    pub(crate) find_query: crate::search::Query,
    /// What its shell is doing, from the OSC 133 marks.
    activity: crate::activity::Activity,
}

/// Building a pane's frame panicked: when last, how many times, and
/// whether it has stopped being built.
#[derive(Clone, Copy)]
struct Crash {
    at: Instant,
    count: u32,
    stopped: bool,
}

/// A second panic building the same pane within this long stops it being
/// built, so a panic doesn't repeat every frame.
const CRASH_REPEAT: Duration = Duration::from_secs(10);

pub(crate) struct State {
    pub(crate) win: Win,
    tree: Panes,
    pub(crate) panes: HashMap<PaneId, Pane>,
    /// The lines between the panes, as drawn (window pixels).
    dividers: Vec<RectInstance>,
    /// The dividers as laid out: their lines, grab zones and splits.
    grabs: Vec<Divider>,
    /// A view over each divider's grab zone, in the same order.
    divider_views: Vec<Retained<DividerView>>,
    /// The divider being dragged with the mouse.
    dragging: Option<Drag>,
}

/// A divider drag under way: its split, which way it moves, and where in
/// its grab zone the mouse took it (pixels from the line's start), so
/// the line keeps that distance from the pointer.
struct Drag {
    path: SplitPath,
    dir: Dir,
    offset: i32,
}

impl State {
    /// The pane with focus.
    fn focused(&mut self) -> Option<&mut Pane> {
        self.panes.get_mut(&self.tree.focused())
    }

    /// The panes shown: all of them, or the zoomed one.
    fn shown(&self) -> Vec<PaneId> {
        match self.tree.zoomed() {
            Some(z) => vec![z],
            None => self.tree.ids(),
        }
    }
}

/// A local key-down monitor waiting for the chord to record.
struct Recording {
    monitor: Retained<AnyObject>,
    /// The pane that asked, which gets the reply.
    pane: PaneId,
}

/// How long a chord recording waits for a key.
const RECORD_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct AutoScroll {
    /// Lines a second; positive scrolls back into history.
    pub(crate) speed: f64,
    /// The column under the mouse, for extending the selection.
    pub(crate) col: usize,
    pub(crate) last: Instant,
    /// Part of a line not yet scrolled.
    pub(crate) carry: f64,
}

/// Test-only control commands waiting to run, and the dump in progress.
struct TestControl {
    path: PathBuf,
    queue: VecDeque<Control>,
    dump: DumpState,
    seq: u64,
    /// Links that would have been opened.
    opened: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DumpState {
    Idle,
    /// Waiting for the next frame to be drawn.
    Requested,
    /// Waiting for the GPU to finish this frame.
    Drawn(u64),
}

pub struct Ivars {
    id: usize,
    layer: Retained<CAMetalLayer>,
    state: RefCell<Option<State>>,
    /// The pane tree and the first pane's view, made with the view so the
    /// app can make it first responder before [`WindowView::start`].
    first: RefCell<Option<(Panes, Retained<PaneView>)>>,
    /// The window whose `contentLayoutRect` this view observes (it changes
    /// when the tab bar shows or hides), until the window closes.
    observed: RefCell<Option<Retained<NSWindow>>>,
}

/// The key path observed for the tab bar showing or hiding.
const LAYOUT_RECT: &str = "contentLayoutRect";

thread_local! {
    static VIEWS: RefCell<HashMap<usize, Weak<WindowView>>> = RefCell::new(HashMap::new());
    /// Merges bursts of notifications per window.
    static BURSTS: RefCell<crate::activity::Coalescer> = RefCell::new(crate::activity::Coalescer::default());
    static NEXT_ID: StdCell<usize> = const { StdCell::new(1) };
}

pub(crate) fn with_view(id: usize, f: impl FnOnce(&WindowView)) {
    let view = VIEWS.with(|v| v.borrow().get(&id).and_then(|w| w.load()));
    if let Some(view) = view {
        f(&view);
    }
}

/// Every open window's view (main thread).
pub(crate) fn all_views() -> Vec<Retained<WindowView>> {
    VIEWS.with(|v| v.borrow().values().filter_map(|w| w.load()).collect())
}

/// On the next turn of the main queue, hide or show every window as it
/// can now be seen: a tab switch changes the key window, but the group's
/// selected tab may change after the key notifications are sent.
pub(crate) fn sync_visibility_soon() {
    DispatchQueue::main().exec_async(|| {
        let views: Vec<_> = VIEWS.with(|v| v.borrow().values().filter_map(|w| w.load()).collect());
        for v in views {
            v.sync_visibility();
        }
    });
}

/// Bring view `id`'s window or tab to the front.
pub(crate) fn bring_to_front(id: usize) {
    with_view(id, |v| {
        if let Some(window) = v.window() {
            crate::background::make_key_and_order_front(&window);
            crate::background::activate(MainThreadMarker::from(v));
            // An inactive app gets no key notifications to notice the switch.
            sync_visibility_soon();
        }
    });
}

/// Bring view `id`'s window or tab to the front and focus pane `pane` in
/// it (a click on a notification for it).
pub(crate) fn show_pane(id: usize, pane: usize) {
    bring_to_front(id);
    with_view(id, |v| {
        if let Ok(pane) = PaneId::try_from(pane) {
            v.focus_pane(pane);
        }
        crate::notify::seen(MainThreadMarker::from(v), id);
    });
}

/// Wake the main thread to draw pane `pane` of window `id` (called from
/// any thread).
fn wake(id: usize, pane: PaneId) {
    DispatchQueue::main().exec_async(move || with_view(id, |v| v.output_arrived(pane)));
}

define_class!(
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeclineWindowView"]
    #[ivars = Ivars]
    pub struct WindowView;

    impl WindowView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            self.redraw_all();
        }

        #[unsafe(method_id(makeBackingLayer))]
        fn make_backing_layer(&self) -> Retained<CALayer> {
            Retained::into_super(self.ivars().layer.clone())
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.resize();
        }

        #[unsafe(method(viewDidChangeBackingProperties))]
        fn did_change_backing(&self) {
            self.reload_fonts(None);
        }

        /// The window's `contentLayoutRect` changed: with full-size content
        /// the view keeps its size when the tab bar shows or hides, so the
        /// grid has to follow the part nothing covers.
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            _key_path: Option<&NSString>,
            _object: Option<&AnyObject>,
            _change: Option<&AnyObject>,
            _context: *mut std::ffi::c_void,
        ) {
            let inset = self.titlebar_height();
            if self.with_state(|st| st.win.top_inset != inset) == Some(true) {
                self.resize();
            }
        }

        #[unsafe(method(tick:))]
        fn tick(&self, _link: &CADisplayLink) {
            self.draw();
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            self.appearance_changed();
        }

        #[unsafe(method(increaseFontSize:))]
        fn increase_font_size(&self, _sender: Option<&AnyObject>) {
            let size = self.with_state(|st| st.win.font_size + 1.0);
            self.reload_fonts(size);
        }

        #[unsafe(method(decreaseFontSize:))]
        fn decrease_font_size(&self, _sender: Option<&AnyObject>) {
            let size = self.with_state(|st| (st.win.font_size - 1.0).max(6.0));
            self.reload_fonts(size);
        }

        #[unsafe(method(resetFontSize:))]
        fn reset_font_size(&self, _sender: Option<&AnyObject>) {
            let size = self.with_state(|st| st.win.config.font_size);
            self.reload_fonts(size);
        }

        /// Shell > Split Right (Cmd+D).
        #[unsafe(method(splitRight:))]
        fn split_right(&self, _sender: Option<&AnyObject>) {
            self.split(Dir::Right);
        }

        /// Shell > Split Down (Cmd+Shift+D).
        #[unsafe(method(splitDown:))]
        fn split_down(&self, _sender: Option<&AnyObject>) {
            self.split(Dir::Down);
        }

        /// Shell > Close (Cmd+W): the focused pane, or the tab with its last.
        #[unsafe(method(closePane:))]
        fn close_pane_item(&self, _sender: Option<&AnyObject>) {
            self.close_focused_pane();
        }

        /// Shell > Close Window (Cmd+Shift+W): every tab of the window.
        #[unsafe(method(closeWindow:))]
        fn close_window_item(&self, _sender: Option<&AnyObject>) {
            self.close_all_tabs();
        }

        #[unsafe(method(selectPaneLeft:))]
        fn select_pane_left(&self, _sender: Option<&AnyObject>) {
            self.focus_towards(Direction::Left);
        }

        #[unsafe(method(selectPaneRight:))]
        fn select_pane_right(&self, _sender: Option<&AnyObject>) {
            self.focus_towards(Direction::Right);
        }

        #[unsafe(method(selectPaneUp:))]
        fn select_pane_up(&self, _sender: Option<&AnyObject>) {
            self.focus_towards(Direction::Up);
        }

        #[unsafe(method(selectPaneDown:))]
        fn select_pane_down(&self, _sender: Option<&AnyObject>) {
            self.focus_towards(Direction::Down);
        }

        #[unsafe(method(selectNextPane:))]
        fn select_next_pane(&self, _sender: Option<&AnyObject>) {
            self.focus_in_order(true);
        }

        #[unsafe(method(selectPreviousPane:))]
        fn select_previous_pane(&self, _sender: Option<&AnyObject>) {
            self.focus_in_order(false);
        }

        /// Window > Move Divider Left (Cmd+Ctrl+←), and the other three.
        #[unsafe(method(resizePaneLeft:))]
        fn resize_pane_left(&self, _sender: Option<&AnyObject>) {
            self.resize_pane(Direction::Left);
        }

        #[unsafe(method(resizePaneRight:))]
        fn resize_pane_right(&self, _sender: Option<&AnyObject>) {
            self.resize_pane(Direction::Right);
        }

        #[unsafe(method(resizePaneUp:))]
        fn resize_pane_up(&self, _sender: Option<&AnyObject>) {
            self.resize_pane(Direction::Up);
        }

        #[unsafe(method(resizePaneDown:))]
        fn resize_pane_down(&self, _sender: Option<&AnyObject>) {
            self.resize_pane(Direction::Down);
        }

        /// Window > Equalize Panes (Cmd+Ctrl+=).
        #[unsafe(method(equalizePanes:))]
        fn equalize_panes(&self, _sender: Option<&AnyObject>) {
            self.equalize();
        }

        /// Window > Zoom Pane (Cmd+Shift+Enter): the focused pane fills the
        /// window, or the zoom ends.
        #[unsafe(method(togglePaneZoom:))]
        fn toggle_pane_zoom(&self, _sender: Option<&AnyObject>) {
            self.toggle_zoom();
        }

        /// Zoom Pane has a checkmark while a pane is zoomed. Every item
        /// sent here stays enabled (pane items beep when they can't act).
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            if item.action() == Some(sel!(togglePaneZoom:)) {
                let zoomed = self.with_state(|st| st.tree.zoomed().is_some()).unwrap_or(false);
                item.setState(if zoomed {
                    objc2_app_kit::NSControlStateValueOn
                } else {
                    objc2_app_kit::NSControlStateValueOff
                });
            }
            true
        }

        /// The I-beam over every pane below the title bar, and a resize
        /// cursor over each divider's grab zone. Set here for all of them,
        /// since overlapping cursor rects (of sibling views, or of one
        /// view) have no defined winner: the I-beam's rects leave the grab
        /// zones out.
        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            let titlebar = self.titlebar_height();
            let rects = self.with_state(|st| {
                let scale = st.win.scale;
                let inset = (titlebar * scale).round() as i32;
                let holes: Vec<PxRect> = st.grabs.iter().map(|d| d.grab).collect();
                let beams: Vec<NSRect> = st
                    .shown()
                    .iter()
                    .filter_map(|id| st.panes.get(id))
                    .flat_map(|p| {
                        let mut r = p.rect;
                        if r.y == 0 {
                            let cut = inset.min(r.h);
                            r.y += cut;
                            r.h -= cut;
                        }
                        crate::panes::subtract(r, &holes)
                    })
                    .map(|r| points(r, scale))
                    .collect();
                let grabs: Vec<(NSRect, Dir)> = st.grabs.iter().map(|d| (points(d.grab, scale), d.dir)).collect();
                (beams, grabs)
            });
            let Some((beams, grabs)) = rects else { return };
            let cursor = NSCursor::IBeamCursor();
            for r in beams {
                self.addCursorRect_cursor(r, &cursor);
            }
            for (r, dir) in grabs {
                self.addCursorRect_cursor(r, &resize_cursor(dir));
            }
        }
    }

    unsafe impl NSObjectProtocol for WindowView {}

    unsafe impl NSWindowDelegate for WindowView {
        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _n: &NSNotification) {
            // A tab brought to the front becomes key: show it now, so its
            // first frame is drawn before it reaches the screen.
            self.sync_visibility();
            self.set_focus(true);
            sync_visibility_soon();
            let mtm = MainThreadMarker::from(self);
            if objc2_app_kit::NSApplication::sharedApplication(mtm).isActive() {
                crate::notify::seen(mtm, self.ivars().id);
            }
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _n: &NSNotification) {
            // Another window or tab, or another app: a chord being
            // recorded is cancelled.
            self.finish_recording(None);
            self.set_focus(false);
            // The tab put behind resigns key before its group selects
            // another tab; look again once that has happened.
            sync_visibility_soon();
        }

        #[unsafe(method(windowDidChangeOcclusionState:))]
        fn window_did_change_occlusion(&self, _n: &NSNotification) {
            if self.sync_visibility() {
                self.redraw_all();
            }
        }

        /// With `confirm_close`, ask first while a program runs in any
        /// pane: once, naming them all.
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, window: &NSWindow) -> bool {
            let ask = self.with_state(|st| st.win.config.confirm_close).unwrap_or(false);
            let programs = if ask { self.running_programs() } else { Vec::new() };
            let close = programs.is_empty();
            if !close {
                self.ask_to_close(window, Closing::Window, &programs, AfterAsk::Window);
            }
            close
        }

        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _n: &NSNotification) {
            self.shut_down();
        }
    }
);

pub struct Launch {
    pub command: Command,
    /// Made by the app with the fonts the config sets
    /// ([`Renderer::with_font_config`], shared with other windows), so it
    /// could size the window from their cells.
    pub renderer: Renderer,
    pub scale: f64,
    /// Shown at the top of the window, before the program's output.
    pub notices: Rc<[String]>,
    /// The Settings tab (see `Win::settings`).
    pub settings: bool,
    /// The panes to open, as saved for a restart; `None` for one pane
    /// running `command`.
    pub layout: Option<SavedPanes>,
    /// The agent API's session this window shows (`ctl spawn --window`):
    /// its first pane's program is the session's.
    pub adopt: Option<crate::ctl::window::Adopt>,
}

/// The renderer's theme for a window: the Settings tab stays opaque
/// whatever `opacity` says, so it can always be read.
fn window_theme(config: &Config, appearance: Appearance, settings: bool) -> Theme {
    let mut theme = config.theme(appearance);
    if settings {
        theme.background_opacity = 255;
    }
    theme
}

impl WindowView {
    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<WindowView> {
        let id = NEXT_ID.with(|n| {
            let id = n.get();
            n.set(id + 1);
            id
        });
        let this = Self::alloc(mtm).set_ivars(Ivars {
            id,
            layer: CAMetalLayer::new(),
            state: RefCell::new(None),
            first: RefCell::new(None),
            observed: RefCell::new(None),
        });
        let view: Retained<WindowView> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setWantsLayer(true);
        let tree = Panes::new();
        let size = view.frame().size;
        let pane = PaneView::new(mtm, &view, tree.focused(), NSRect::new(NSPoint::new(0.0, 0.0), size));
        view.add_pane_view(&pane);
        *view.ivars().first.borrow_mut() = Some((tree, pane));
        VIEWS.with(|v| v.borrow_mut().insert(id, Weak::from_retained(&view)));
        view
    }

    /// The view that should have the keys: the focused pane's.
    pub fn first_responder(&self) -> Option<Retained<PaneView>> {
        if let Some((_, pane)) = self.ivars().first.borrow().as_ref() {
            return Some(pane.clone());
        }
        self.with_state(|st| st.focused().map(|p| p.view.clone())).flatten()
    }

    pub(crate) fn id(&self) -> usize {
        self.ivars().id
    }

    /// Start the program and drawing. The view must be in its window.
    /// With a saved layout, every pane of it is opened in its folder.
    pub fn start(&self, config: Rc<Config>, launch: Launch) -> std::io::Result<()> {
        let id = self.ivars().id;
        let Some((first_tree, first_view)) = self.ivars().first.borrow_mut().take() else {
            return Err(std::io::Error::other("started twice"));
        };
        let mtm = MainThreadMarker::from(self);
        let layer = &self.ivars().layer;
        let renderer = launch.renderer;
        let settings = launch.settings;
        let scale = launch.scale;
        let mut adopt = launch.adopt;
        let opaque = config.opacity >= 1.0 || settings;
        term_render::configure_layer(layer, renderer.device(), opaque);
        layer.setContentsScale(scale);

        let appearance = self.appearance();
        let theme = window_theme(&config, appearance, settings);
        let m = renderer.metrics();
        let size = self.frame().size;
        let titlebar = self.titlebar_height();
        let test = std::env::var_os(control::ENV).filter(|p| !p.is_empty()).map(|p| TestControl {
            path: PathBuf::from(p),
            queue: VecDeque::new(),
            dump: DumpState::Idle,
            seq: 0,
            opened: Vec::new(),
        });
        let spawn = |n: NewPane| spawn_pane(id, &config, &theme, m, test.is_some(), n);
        let mut panes = HashMap::new();
        let tree = match &launch.layout {
            None => {
                let pad = padding_for(&config, titlebar, scale);
                let grid = layout::grid_size((size.width, size.height), scale, m.into(), pad);
                let pane_id = first_tree.focused();
                let pane = spawn(NewPane {
                    id: pane_id,
                    view: first_view,
                    command: &launch.command,
                    grid,
                    notices: &launch.notices,
                    program: true,
                    settings,
                    adopt: adopt.take(),
                })?;
                panes.insert(pane_id, pane);
                first_tree
            }
            Some(saved) => {
                first_view.removeFromSuperview();
                let (mut tree, list) = Panes::restore(saved);
                let g = geometry_of(&config, m, scale, size, titlebar);
                // Panes a zoom hides are opened at the size they have
                // once it ends.
                let laid = tree.full_layout(&g);
                let mut failed = None;
                for (pane_id, saved) in list {
                    let rect = laid.rect(pane_id).unwrap_or_default();
                    let view = PaneView::new(mtm, self, pane_id, points(rect, scale));
                    self.add_pane_view(&view);
                    let mut command = if saved.program {
                        launch.command.clone()
                    } else {
                        shell_command(&config)
                    };
                    if saved.cwd.is_some() {
                        command.cwd = saved.cwd.clone();
                    }
                    let focused = pane_id == tree.focused();
                    let made = spawn(NewPane {
                        id: pane_id,
                        view: view.clone(),
                        command: &command,
                        grid: grid_of(&config, titlebar, scale, m, rect),
                        notices: if focused { &launch.notices } else { &[] },
                        program: saved.program,
                        settings: false,
                        adopt: None,
                    });
                    match made {
                        Ok(pane) => {
                            if let Some(c) = saved.command.filter(|_| pane.runs_shell) {
                                pane.session.type_at_prompt(c);
                            }
                            panes.insert(pane_id, pane);
                        }
                        Err(e) => {
                            eprintln!("terminal-decline: could not open a pane: {e}");
                            view.removeFromSuperview();
                            failed = Some(e);
                        }
                    }
                }
                // Panes that didn't open give their space to the others.
                for pane_id in tree.ids() {
                    if !panes.contains_key(&pane_id) && tree.close(pane_id).is_none() {
                        return Err(failed.unwrap_or_else(|| std::io::Error::other("no pane opened")));
                    }
                }
                tree
            }
        };

        let link = unsafe { self.displayLinkWithTarget_selector(self, sel!(tick:)) };
        unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes) };
        link.setPaused(false);

        *self.ivars().state.borrow_mut() = Some(State {
            win: Win {
                renderer,
                link,
                font_size: config.font_size,
                binds: config.keybinds(),
                divider: divider_colour(&config, appearance),
                transition: None,
                config,
                theme,
                appearance,
                scale,
                focused: true,
                idle_ticks: 0,
                frames: 0,
                rate: 0,
                last_input: Instant::now(),
                test,
                top_inset: titlebar,
                hidden: 0,
                shown: true,
                images_hidden: false,
                title_clock: 0,
                settings,
                recording: None,
                recordings: 0,
                base_min: None,
                min: None,
            },
            tree,
            panes,
            dividers: Vec::new(),
            grabs: Vec::new(),
            divider_views: Vec::new(),
            dragging: None,
        });
        if let Some(window) = self.window() {
            unsafe {
                window.addObserver_forKeyPath_options_context(
                    self,
                    &NSString::from_str(LAYOUT_RECT),
                    NSKeyValueObservingOptions::empty(),
                    std::ptr::null_mut(),
                )
            };
            *self.ivars().observed.borrow_mut() = Some(window);
        }
        self.resize();
        Ok(())
    }

    /// What pane `pane`'s program asked for, taken as it was parsed (see
    /// [`crate::session::Events`]): drawn or not, a tab's title is shown
    /// in the tab bar; a bell, a program's notification or a long command
    /// finishing is a notification unless you are looking at this pane
    /// (the app in front, this tab key, the pane focused); and progress
    /// goes on the Dock icon.
    fn events(&self, pane: PaneId, events: Events) {
        let mtm = MainThreadMarker::from(self);
        // Every pane keeps its title; only the focused pane's is the
        // window's. A title set with a command's start counts as the
        // shell's, not the command's: it is taken in first.
        let Some(Some(focused)) = self.with_state(|st| {
            let focused = st.tree.focused() == pane;
            let p = st.panes.get_mut(&pane)?;
            if let Some(title) = &events.title {
                p.title = Some(title.clone());
                p.activity.titled();
            }
            Some(focused)
        }) else {
            return;
        };
        let looking = self.looking(pane);
        let now = Instant::now();
        let finished = self
            .with_state(|st| {
                let (on, after) = (
                    st.win.config.notify_on_finish,
                    Duration::from_secs(st.win.config.notify_after.into()),
                );
                let p = st.panes.get_mut(&pane)?;
                let done: Vec<_> = events.commands.iter().filter_map(|c| p.activity.apply(c.clone(), now)).collect();
                Some((on, after, done, p.settings))
            })
            .flatten();
        let Some((on, after, finished, settings)) = finished else { return };
        if focused && (events.title.is_some() || !events.commands.is_empty()) {
            self.refresh_title();
        }
        if settings {
            // The Settings tab's own program.
        } else if !looking.at_pane() {
            // Something wants you where you aren't looking: a notification
            // (merged with others from this window), else the Dock icon
            // bounces.
            let (name, folder) = self.pane_names(pane);
            for f in finished.iter().filter(|f| crate::activity::should_notify(f, on, after, looking)) {
                let title = crate::activity::finish_title(f);
                let body = crate::activity::finish_body(f, folder.as_deref());
                self.notify_pane(pane, crate::activity::Kind::Finished, &title, &body, looking);
            }
            if let Some(n) = events.notification {
                // OSC 9 has no title: use the pane's (programs set it).
                let title = n.title.filter(|t| !t.is_empty()).unwrap_or_else(|| name.clone());
                self.notify_pane(pane, crate::activity::Kind::Message, &title, &n.body, looking);
            }
            if events.bell {
                let body = match &folder {
                    Some(f) => format!("Bell · {f}"),
                    None => "Bell".into(),
                };
                self.notify_pane(pane, crate::activity::Kind::Bell, &name, &body, looking);
            }
        }
        if let Some(p) = events.progress {
            crate::notify::set_progress(mtm, self.ivars().id, p);
        }
    }

    /// The window's (tab's) title: the Settings tab's, a pin's fixed one,
    /// else what the focused pane does with `title_status` on and its
    /// shell's marks ([`crate::activity::status_title`]), else its
    /// program's title. The clock is armed again for the next change.
    pub(crate) fn refresh_title(&self) {
        let now = Instant::now();
        let home = std::env::var("HOME").ok();
        let got = self.with_state(|st| {
            let fixed = st.win.config.title.clone();
            let pinned = fixed.is_some();
            let status = st.win.config.title_status;
            let after = Duration::from_secs(st.win.config.notify_after.into());
            let shown = st.win.shown;
            let p = st.focused()?;
            let title = match (p.settings, fixed) {
                (true, _) => "Settings".to_string(),
                (false, Some(t)) => t,
                (false, None) => status
                    .then(|| {
                        let folder = p.activity.marks.then(|| pane_cwd(p)).flatten();
                        let folder = folder.map(|f| crate::activity::tilde(&f, home.as_deref()));
                        crate::activity::status_title(&p.activity, p.title.as_deref(), folder.as_deref(), after, now)
                    })
                    .flatten()
                    .unwrap_or_else(|| p.title.clone().unwrap_or_else(|| crate::PRODUCT_NAME.into())),
            };
            let tick = (status && shown && !p.settings && !pinned)
                .then(|| crate::activity::next_change(&p.activity, after, now))
                .flatten();
            st.win.title_clock += 1;
            Some((title, tick, st.win.title_clock))
        });
        let Some(Some((title, tick, clock))) = got else { return };
        if let Some(window) = self.window()
            && window.title().to_string() != title
        {
            window.setTitle(&NSString::from_str(&title));
        }
        // One shot, to the next change of the title: none at the prompt,
        // none while hidden (showing the window refreshes it).
        if let Some(after) = tick {
            let id = self.ivars().id;
            let at = DispatchTime::NOW.time(after.as_nanos().min(i64::MAX as u128) as i64 + 1_000_000);
            let _ = DispatchQueue::main().after(at, move || {
                with_view(id, |v| {
                    if v.with_state(|st| st.win.title_clock == clock) == Some(true) {
                        v.refresh_title();
                    }
                })
            });
        }
    }

    /// Where you are looking, for pane `pane`: the app in front, this
    /// window (tab) key, and the pane focused.
    fn looking(&self, pane: PaneId) -> crate::activity::Looking {
        let app = objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(self));
        crate::activity::Looking {
            app_active: app.isActive(),
            window_key: self.window().is_some_and(|w| crate::background::is_key(&w)),
            pane_focused: self.with_state(|st| st.tree.focused() == pane).unwrap_or(false),
        }
    }

    /// Pane `pane`'s name for a notification (its program's title, else
    /// the window's, else the app's) and its folder, with `~` for home.
    fn pane_names(&self, pane: PaneId) -> (String, Option<String>) {
        let window_title = self.window().map(|w| w.title().to_string()).filter(|t| !t.is_empty());
        let (title, cwd) = self
            .with_state(|st| st.panes.get(&pane).map(|p| (p.title.clone(), pane_cwd(p))))
            .flatten()
            .unwrap_or_default();
        let home = std::env::var("HOME").ok();
        let folder = cwd.map(|c| crate::activity::tilde(&c, home.as_deref()));
        let name = title
            .filter(|t| !t.is_empty())
            .or(window_title)
            .unwrap_or_else(|| crate::PRODUCT_NAME.into());
        (name, folder)
    }

    /// A notification for pane `pane`, merged with the window's burst
    /// (see [`crate::activity::Coalescer`]), counted on the Dock icon
    /// unless you are looking at this window.
    fn notify_pane(&self, pane: PaneId, kind: crate::activity::Kind, title: &str, body: &str, looking: crate::activity::Looking) {
        let id = self.ivars().id;
        let post = BURSTS.with(|b| b.borrow_mut().add(id, pane as usize, kind, title, body, Instant::now()));
        let Some(post) = post else { return };
        let mut note = crate::notify::Note::new(
            post.title,
            post.body,
            crate::notify::Target::Pane {
                view: id,
                pane: pane as usize,
            },
        );
        note.group = Some(format!("window.{id}"));
        note.key = Some(format!("burst.{id}.{}", post.id));
        note.unseen = (!looking.at_window()).then_some(id);
        crate::notify::show(MainThreadMarker::from(self), note);
    }

    pub(crate) fn with_state<R>(&self, f: impl FnOnce(&mut State) -> R) -> Option<R> {
        let mut st = self.ivars().state.borrow_mut();
        st.as_mut().map(f)
    }

    /// `f` on the window and pane `pane`; `None` once it is gone.
    pub(crate) fn with_pane<R>(&self, pane: PaneId, f: impl FnOnce(&mut Win, &mut Pane) -> R) -> Option<R> {
        let mut st = self.ivars().state.borrow_mut();
        let st = st.as_mut()?;
        let p = st.panes.get_mut(&pane)?;
        Some(f(&mut st.win, p))
    }

    /// The focused pane's view.
    fn focused_view(&self) -> Option<Retained<PaneView>> {
        self.with_state(|st| st.focused().map(|p| p.view.clone())).flatten()
    }

    /// The focused pane's working directory.
    pub fn foreground_cwd(&self) -> Option<String> {
        self.with_state(|st| st.focused().and_then(|p| pane_cwd(p))).flatten()
    }

    /// The focused pane, if the window has started.
    pub(crate) fn focused_pane(&self) -> Option<PaneId> {
        self.with_state(|st| st.tree.focused())
    }

    /// The title bar's height, plus the tab bar's while one shows.
    pub(crate) fn titlebar_height(&self) -> f64 {
        match self.window() {
            Some(w) => layout::top_inset(w.frame().size.height, w.contentLayoutRect().size.height),
            None => 0.0,
        }
    }

    pub(crate) fn unpause(&self) {
        let was_paused = self.with_state(|st| {
            st.win.idle_ticks = 0;
            let paused = st.win.link.isPaused();
            st.win.link.setPaused(false);
            paused
        });
        if was_paused == Some(true) {
            // Nothing drawn recently: draw now instead of waiting a refresh.
            self.draw();
        }
    }

    /// Pane `pane`'s program wrote something (its dirty flag was clear).
    /// If it may be the echo of input, draw now rather than on the next
    /// tick. A pane that has closed is ignored.
    fn output_arrived(&self, pane: PaneId) {
        let echo = self.with_pane(pane, |win, p| {
            let echo = p.awaiting_echo.take().is_some();
            if echo {
                win.idle_ticks = 0;
                win.link.setPaused(false);
            }
            echo
        });
        match echo {
            Some(true) => self.draw(),
            Some(false) => self.unpause(),
            None => {}
        }
    }

    /// Draw pane `pane` again soon.
    /// A selection is starting in `pane`: the other panes' are cleared
    /// (one selection per window, [`crate::select::clear_others`]).
    pub(crate) fn clear_other_selections(&self, pane: PaneId) {
        let cleared = self
            .with_state(|st| {
                let mut cleared = Vec::new();
                for (&id, p) in st.panes.iter_mut().filter(|(id, _)| **id != pane) {
                    if crate::select::clear_others([&mut p.session.lock_term().state]) {
                        p.block = None;
                        cleared.push(id);
                    }
                }
                cleared
            })
            .unwrap_or_default();
        for id in cleared {
            self.request_redraw(id);
        }
    }

    pub(crate) fn request_redraw(&self, pane: PaneId) {
        self.with_pane(pane, |_, p| p.session.mark_dirty());
        self.unpause();
    }

    /// A batch of history hits from the search worker for `pane`'s
    /// search: added under the lock, and drawn (with the bar's count).
    pub(crate) fn search_batch(&self, pane: PaneId, batch: term_core::HitBatch) {
        let titlebar = self.titlebar_height();
        let added = self.with_pane(pane, |win, p| {
            if p.find.is_none() {
                return false;
            }
            let covered = rows_under_find_bar(win, p, titlebar);
            let mut t = p.session.lock_term();
            let before = t.state.current_hit();
            t.state.add_search_hits(batch);
            // A step that waited for this batch jumped to its hit.
            if let Some(h) = t.state.current_hit().filter(|h| before != Some(*h)) {
                crate::search::keep_clear(&mut t.state, h.start(), covered);
            }
            true
        });
        if added == Some(true) {
            self.request_redraw(pane);
        }
    }

    /// The app became active: the panes with a find bar open read the
    /// shared find text again.
    pub(crate) fn refresh_shared_find(&self) {
        let views: Vec<_> = self
            .with_state(|st| st.panes.values().filter(|p| p.find.is_some()).map(|p| p.view.clone()).collect())
            .unwrap_or_default();
        for v in views {
            v.read_shared_find();
        }
    }

    /// Draw every pane again soon.
    fn redraw_all(&self) {
        self.with_state(|st| {
            for p in st.panes.values() {
                p.session.mark_dirty();
            }
        });
        self.unpause();
    }

    /// Whether the window can be seen. A tab behind another tab of its
    /// group can't, whatever its occlusion state says: macOS doesn't
    /// reliably report a background tab as covered. The key window can,
    /// even before its occlusion state catches up with a tab switch.
    pub(crate) fn visible(&self) -> bool {
        self.window().is_some_and(|w| {
            let behind = crate::background::tab_group(&w)
                .and_then(|g| g.selectedWindow())
                .is_some_and(|front| !std::ptr::eq::<NSWindow>(&*front, &*w));
            !behind
                && (crate::background::is_key(&w)
                    || crate::background::on()
                    || w.occlusionState().contains(objc2_app_kit::NSWindowOcclusionState::Visible))
        })
    }

    /// Hide or show after the window was covered, uncovered, or its tab put
    /// behind or brought to the front. Hiding frees what can be rebuilt
    /// while nothing can be seen; showing draws at once, so the window
    /// never shows a stale frame. Returns whether the window can be seen.
    fn sync_visibility(&self) -> bool {
        let visible = self.visible();
        let minimised = self.window().is_some_and(|w| w.isMiniaturized());
        let changed = self.with_state(|st| {
            let changed = st.win.shown != visible;
            st.win.shown = visible;
            if changed && !visible {
                st.win.hidden += 1;
                st.win.renderer.hide(minimised);
                images_shown(&mut st.win, &st.panes, false);
            }
            changed
        });
        if changed == Some(true) {
            // The title's clock runs only while the window can be seen.
            self.refresh_title();
            let layer = &self.ivars().layer;
            if visible {
                // Its drawables back at full size, then drawn at once.
                if let Some(window) = self.window() {
                    let (size, scale) = (self.frame().size, window.backingScaleFactor());
                    layer.setDrawableSize(NSSize::new((size.width * scale).round(), (size.height * scale).round()));
                }
                self.redraw_all();
            } else if minimised {
                // The Dock shows a minimised window's last frame.
                self.trim_drawables();
            } else {
                // Nothing of it can be seen: let both drawables go, not
                // only the spare one, until it is shown again.
                layer.setDrawableSize(NSSize::new(1.0, 1.0));
                self.hide_again_soon();
            }
        }
        visible
    }

    /// Mark a hidden window's surfaces purgeable again once the window
    /// server has taken up its last frame: one presented just before the
    /// window was hidden (a tab switch that drew once more) stays counted
    /// when marked while it is still on its way to the screen.
    fn hide_again_soon(&self) {
        let id = self.ivars().id;
        for after in [Duration::from_millis(500), Duration::from_secs(3)] {
            let _ = DispatchQueue::main().after(DispatchTime::NOW.time(after.as_nanos() as i64), move || {
                with_view(id, |v| {
                    v.with_state(|st| st.win.renderer.hide_again());
                })
            });
        }
    }

    /// One display-link tick (or a draw asked for at once): each shown
    /// pane says whether it builds a new frame, reuses its last one, is
    /// held in a synchronized update or waits for an echo
    /// ([`frame_policy::pane`]); the window decides from those
    /// ([`frame_policy::combine`]), builds the panes that need it and
    /// draws them all in one frame.
    fn draw(&self) {
        let titlebar = self.titlebar_height();
        let visible = self.visible();
        let minimised = self.window().is_some_and(|w| w.isMiniaturized());
        let mut guard = self.ivars().state.borrow_mut();
        let Some(st) = guard.as_mut() else { return };
        let shown = st.shown();
        let State {
            win,
            panes,
            tree,
            dividers,
            ..
        } = st;
        let focused_pane = tree.focused();
        let dividers: &[RectInstance] = if tree.zoomed().is_some() { &[] } else { dividers };
        // Under test control, draw even when covered by other windows, so a
        // dump never waits forever for a frame.
        let seen = visible;
        let visible = visible || win.test.is_some();
        let terms: Vec<(PaneId, Arc<Mutex<Term>>)> = shown
            .iter()
            .filter_map(|id| panes.get(id).map(|p| (*id, p.session.term().clone())))
            .collect();
        let mut ticks = Vec::with_capacity(terms.len());
        for (id, term) in &terms {
            let p = panes.get_mut(id).expect("shown pane");
            let dirty = p.session.take_dirty();
            let echo = p.awaiting_echo.map(|sent| sent.elapsed());
            let waited_out = echo.is_some_and(|e| e >= ECHO_WAIT);
            if waited_out {
                // No echo: draw what the input changed (cursor, scroll).
                p.awaiting_echo = None;
            }
            // Only a frame that may be drawn needs the terminal's lock,
            // and never waits long for it (roadmap R.1): a pane whose
            // reader still holds it is held for a tick, its change kept.
            let wants = (dirty || waited_out) && visible;
            let t = wants.then(|| crate::session::lock_for_frame(term)).flatten();
            let busy = wants && t.is_none();
            let synchronized = t.as_ref().is_some_and(|t| t.state.modes().synchronized);
            let sync_elapsed = if synchronized {
                p.sync_since.get_or_insert_with(Instant::now).elapsed()
            } else {
                if t.is_some() {
                    p.sync_since = None;
                }
                Duration::ZERO
            };
            // Waited out, it builds like a changed pane (unless held).
            let frame = if busy {
                PaneFrame::Held
            } else {
                frame_policy::pane(dirty || waited_out, synchronized, sync_elapsed, echo.filter(|_| !waited_out))
            };
            ticks.push((*id, frame, t));
        }
        let frames: Vec<PaneFrame> = ticks.iter().map(|(_, f, _)| *f).collect();
        // A theme transition draws every tick until it ends, from the
        // frames already built: only its uniforms change. A window hidden
        // or covered while one plays takes the new theme at once (its
        // frames are already built with it) and draws nothing.
        let animating = win.transition.is_some();
        if animating && !visible {
            win.transition = None;
            win.renderer.set_blend(None);
        }
        let decision = frame_policy::combine(
            &frames,
            &WindowTick {
                visible,
                minimised,
                focused: win.focused,
                idle_ticks: win.idle_ticks,
                last_input_elapsed: win.last_input.elapsed(),
                animating,
            },
        );
        let now = Instant::now();
        // A pane held in a synchronized update keeps its change for later.
        let keep_held = |panes: &HashMap<PaneId, Pane>, ticks: &[(PaneId, PaneFrame, _)]| {
            for (id, f, _) in ticks {
                if *f == PaneFrame::Held {
                    panes[id].session.mark_dirty();
                }
            }
        };
        let rate = match decision {
            Decision::Skip => {
                win.idle_ticks = win.idle_ticks.saturating_add(1);
                return;
            }
            Decision::Pause { trim_after } => {
                win.idle_ticks = win.idle_ticks.saturating_add(1);
                win.link.setPaused(true);
                let token = win.frames;
                drop(ticks);
                drop(guard);
                self.schedule_trim(token, trim_after);
                self.schedule_filters(token);
                return;
            }
            Decision::Hide { keep_surface } => {
                // Hidden or fully covered: keep the changes and draw when shown.
                win.idle_ticks = 0;
                for (id, _, _) in &ticks {
                    panes[id].session.mark_dirty();
                }
                win.link.setPaused(true);
                win.renderer.hide(keep_surface);
                images_shown(win, panes, false);
                return;
            }
            Decision::Hold => {
                // Keep ticking so the timeout (or the echo) is noticed.
                win.idle_ticks = 0;
                keep_held(panes, &ticks);
                win.link.setPaused(false);
                return;
            }
            Decision::Draw { rate } => latency::get().and_then(|l| l.hz()).unwrap_or(rate),
        };
        keep_held(panes, &ticks);
        win.idle_ticks = 0;
        win.renderer.show();
        let typing = win.last_input.elapsed() < frame_policy::INPUT_BOOST;
        win.renderer
            .set_glyph_budget(Some(if typing { GLYPH_BUDGET_TYPING } else { GLYPH_BUDGET }));
        images_shown(win, panes, true);
        if rate != win.rate {
            win.rate = rate;
            let r = rate as f32;
            win.link.setPreferredFrameRateRange(objc2_quartz_core::CAFrameRateRange {
                minimum: r / 2.0,
                maximum: r,
                preferred: r,
            });
        }
        let mut clipboard = None;
        let mut blinking = Vec::new();
        // The panes whose snapshots were taken, shaped and built once the
        // terminals are unlocked (ligatures design §5).
        let mut snapped = Vec::new();
        // The panes whose snapshot or build panicked (splits design §7).
        let mut crashed = Vec::new();
        // Panes whose search found new hits, and those to poll again.
        let mut find_changed = Vec::new();
        let mut find_again = Vec::new();
        for ((id, frame, t), (_, term)) in ticks.iter_mut().zip(&terms) {
            let (id, frame) = (*id, *frame);
            let p = panes.get_mut(&id).expect("shown pane");
            // A pane built against emptied glyph textures or old fonts is
            // built again before it is drawn, held or not.
            let build = frame == PaneFrame::Build || !win.renderer.is_current(&p.render);
            // A pane whose builds kept panicking shows its background only.
            if !build || p.crash.is_some_and(|c| c.stopped) {
                continue;
            }
            if t.is_none() {
                *t = crate::session::lock_for_frame(term);
            }
            let Some(t) = t.as_mut() else {
                // Its reader holds the lock: built next tick.
                p.session.mark_dirty();
                win.link.setPaused(false);
                continue;
            };
            if let Some(text) = t.state.take_clipboard() {
                clipboard = Some(text);
            }
            // Only the focused pane's cursor blinks; the others are hollow.
            let focused = win.focused && id == focused_pane;
            if t.state.cursor_blink() && focused && !p.blink_scheduled {
                p.blink_scheduled = true;
                blinking.push(id);
            }
            // Colours a program set are drawn from the next frame on; the
            // theme changing in the row cache's key builds every row again.
            if t.state.take_colors_changed() {
                p.theme = crate::config::program_theme(&win.theme, &t.state);
            }
            let pad = pane_padding(&win.config, titlebar, win.scale, p.rect);
            let (padding, cursor_on) = ((pad.x, pad.top), p.cursor_on || !focused);
            let input = FrameInput {
                theme: p.theme.as_ref().unwrap_or(&win.theme),
                metrics: win.renderer.metrics(),
                padding,
                cursor_on,
                focused,
                preedit: Some(p.marked.as_str()),
            };
            // The find bar's search keeps up with output.
            let covered = rows_under_find_bar(win, p, titlebar);
            if let Some(f) = &mut p.find {
                // A new terminal (after a caught panic) or a switch to or
                // from the alternate screen leaves the hits meaning
                // something else: search again.
                let now = (t.state.terminal_id(), t.state.alt_screen_active());
                if f.seen.is_some_and(|seen| seen != now) {
                    f.search.start(&mut t.state, &p.find_query, covered);
                    f.seen = Some(now);
                }
                let polled = f.search.poll(&mut t.state);
                let status = f.search.status(&t.state);
                if polled.changed || status != f.shown {
                    f.shown = status;
                    find_changed.push((id, status));
                }
                if let Some(after) = polled.again {
                    find_again.push((id, after));
                }
            }
            p.builds += 1;
            // A panic taking the snapshot stays in this pane. The
            // terminal's guard is held outside the catch, and a terminal
            // that may be half-updated is replaced before it is released,
            // so its reader never parses into it.
            let panic_snapshot = p.panic_next.take_if(|snapshot| *snapshot).is_some();
            let snap = crate::crash::contain(|| {
                test_build_panic(panic_snapshot);
                win.renderer.snapshot_pane(&mut p.render, &mut t.state, &input)
            });
            if snap.is_err() {
                crate::session::replace_term(t);
                p.render.forget();
                crashed.push(id);
                continue;
            }
            // Re-decodes the snapshot asked for (images.md §5.4).
            p.session.take_image_jobs(t);
            if let Some(f) = &mut p.find {
                let snap = p.render.frame.snapshot_mut();
                f.drawn = snap
                    .highlights
                    .iter()
                    .filter(|h| {
                        matches!(
                            h.kind,
                            term_core::HighlightKind::SearchHit | term_core::HighlightKind::CurrentSearchHit
                        )
                    })
                    .count();
            }
            snapped.push((id, padding, cursor_on, focused));
        }
        // With the terminals still locked: the keys whose output this frame shows.
        let timed = latency::get().map(|l| (l, l.take_for_frame(self.ivars().id, rate)));
        drop(ticks);
        // Shaping and building, unlocked. Rarely the glyph textures were
        // emptied since a pane's snapshot (by another pane's build): that
        // pane is built again with its terminal locked. A panic in either
        // stays in its pane.
        for (id, padding, cursor_on, focused) in snapped {
            let p = panes.get_mut(&id).expect("shown pane");
            // The input the snapshot was taken with: the colours a
            // program set included.
            let input = FrameInput {
                theme: p.theme.as_ref().unwrap_or(&win.theme),
                metrics: win.renderer.metrics(),
                padding,
                cursor_on,
                focused,
                preedit: Some(p.marked.as_str()),
            };
            let panic_now = p.panic_next.take().is_some();
            // Unlocked, the terminal is untouched by a panic here.
            let built = crate::crash::contain(|| {
                test_build_panic(panic_now);
                win.renderer.build_from_snapshot(&mut p.render, &input)
            });
            let ok = match built {
                Ok(true) => true,
                Ok(false) => {
                    let mut t = crate::session::lock(p.session.term());
                    let built = crate::crash::contain(|| win.renderer.build_pane(&mut p.render, &mut t.state, &input));
                    if built.is_err() {
                        crate::session::replace_term(&mut t);
                    }
                    built.is_ok()
                }
                Err(_) => false,
            };
            if !ok {
                p.render.forget();
                crashed.push(id);
            } else if p.render.glyphs_deferred() {
                // Drawn without some new glyphs: built again next tick.
                p.session.mark_dirty();
                win.link.setPaused(false);
            }
        }
        if !crashed.is_empty() {
            crash_panes(win, panes, &crashed);
        }
        // Panes that stopped being built fill their rectangle with the
        // background, drawn with the dividers.
        let stopped: Vec<RectInstance> = shown
            .iter()
            .filter_map(|id| panes.get(id))
            .filter(|p| p.crash.is_some_and(|c| c.stopped))
            .map(|p| rect_instance(p.rect, win.theme.background, transition::slot::BACKGROUND))
            .collect();
        let lines: Vec<RectInstance>;
        let dividers = if stopped.is_empty() {
            dividers
        } else {
            lines = [stopped, dividers.to_vec()].concat();
            &lines
        };
        let (w, h) = {
            let s = self.ivars().layer.drawableSize();
            (s.width as u32, s.height as u32)
        };
        let draws: Vec<PaneDraw> = shown
            .iter()
            .filter_map(|id| panes.get(id))
            .filter_map(|p| {
                let r = term_render::PxRect {
                    x: p.rect.x,
                    y: p.rect.y,
                    w: p.rect.w,
                    h: p.rect.h,
                };
                Some(PaneDraw {
                    pane: &p.render,
                    origin: (r.x, r.y),
                    scissor: r.clamped(w, h)?,
                })
            })
            .collect();
        // One pane clears to its own background; several clear to nothing
        // and each fills its own rectangle, so a see-through background's
        // alpha is applied once.
        let clear = match draws.as_slice() {
            [one] => one.pane.frame.clear,
            _ => 0,
        };
        let blend = win.transition.as_ref().map(|t| t.blend(now)).filter(|b| !b.finished());
        if blend.is_none() {
            // Ended: this frame is the new theme's own.
            win.transition = None;
        }
        win.renderer.set_blend(blend);
        match timed {
            Some((lat, keys)) if !keys.is_empty() => {
                present_timed(&mut win.renderer, &self.ivars().layer, clear, &draws, dividers, lat, keys)
            }
            _ => win.renderer.present_panes(&self.ivars().layer, clear, &draws, dividers),
        }
        drop(draws);
        if !seen && !win.shown {
            // Drawn only for the test control while hidden: hidden again,
            // so its surfaces don't stay counted.
            win.renderer.hide(minimised);
        }
        win.frames += 1;
        let mut poll_dump = false;
        // A dump shows every glyph: it waits for the frame that has them.
        let complete = !shown.iter().filter_map(|id| panes.get(id)).any(|p| p.render.glyphs_deferred());
        if let Some(test) = &mut win.test
            && test.dump == DumpState::Requested
            && complete
        {
            test.dump = DumpState::Drawn(win.renderer.frames_submitted());
            poll_dump = true;
        }
        let bars: Vec<_> = find_changed
            .iter()
            .filter_map(|(id, status)| panes.get(id)?.find.as_ref().map(|f| (f.bar.clone(), *status)))
            .collect();
        drop(guard);
        for pane in blinking {
            self.schedule_blink(pane);
        }
        for (bar, status) in bars {
            bar.show_status(status);
        }
        for (pane, after) in find_again {
            let id = self.ivars().id;
            let _ = DispatchQueue::main().after(DispatchTime::NOW.time(after.as_nanos() as i64), move || {
                with_view(id, |v| v.request_redraw(pane));
            });
        }
        if let Some(text) = clipboard {
            let pb = NSPasteboard::generalPasteboard();
            pb.clearContents();
            pb.setString_forType(&NSString::from_str(&text), unsafe { NSPasteboardTypeString });
        }
        if poll_dump {
            self.poll_dump();
        }
    }

    /// After a couple of seconds with nothing drawn, give back the spare
    /// drawable: resizing the layer's drawables to 1×1 and back releases the
    /// one not on screen (6.6 MB at 1624×1018) without changing what's shown.
    fn schedule_trim(&self, token: u64, after: Duration) {
        let id = self.ivars().id;
        let _ = DispatchQueue::main().after(DispatchTime::NOW.time(after.as_nanos() as i64), move || {
            with_view(id, |v| {
                let idle =
                    v.with_state(|st| st.win.frames == token && st.win.link.isPaused() && !st.panes.values().any(|p| p.blink_scheduled));
                if idle == Some(true) {
                    v.trim_drawables();
                }
            })
        });
    }

    /// Once the display link has stayed paused (no output, no frame) for
    /// a while, have the filters thread build the panes' history search
    /// filters. One timer per pause; nothing repeats.
    fn schedule_filters(&self, token: u64) {
        let id = self.ivars().id;
        let after = crate::idle_filters::IDLE_BEFORE;
        let _ = DispatchQueue::main().after(DispatchTime::NOW.time(after.as_nanos() as i64), move || {
            with_view(id, |v| {
                v.with_state(|st| {
                    let idle = st.win.frames == token && st.win.link.isPaused();
                    if !crate::idle_filters::due(after, !idle) {
                        return;
                    }
                    for p in st.panes.values() {
                        crate::idle_filters::request(p.session.term().clone(), crate::idle_filters::Quiet::new(p.session.batches()));
                    }
                });
            })
        });
    }

    /// Under memory pressure (roadmap R.5, see [`crate::memory`]): give
    /// back the spare drawable, the renderer's textures and buffers, and
    /// what each pane's history can, then draw every pane again.
    pub(crate) fn give_back_memory(&self) -> crate::memory::Freed {
        self.trim_drawables();
        let mut freed = crate::memory::Freed {
            windows: 1,
            ..Default::default()
        };
        self.with_state(|st| {
            freed.render += st.win.renderer.purge();
            for p in st.panes.values() {
                let (before, after) = p.session.lock_term().state.shrink_history();
                freed.history += before.saturating_sub(after);
                freed.panes += 1;
            }
        });
        self.redraw_all();
        freed
    }

    fn trim_drawables(&self) {
        self.with_state(|st| st.win.renderer.forget_spare());
        let layer = &self.ivars().layer;
        let size = layer.drawableSize();
        layer.setDrawableSize(NSSize::new(1.0, 1.0));
        layer.setDrawableSize(size);
    }

    pub(crate) fn schedule_blink(&self, pane: PaneId) {
        let id = self.ivars().id;
        let _ = DispatchQueue::main().after(DispatchTime::NOW.time(BLINK_INTERVAL.as_nanos() as i64), move || {
            with_view(id, |v| {
                let again = v.with_state(|st| {
                    let focused = st.win.focused && st.tree.focused() == pane;
                    let p = st.panes.get_mut(&pane)?;
                    p.blink_scheduled = false;
                    let blinking = focused && p.session.lock_term().state.cursor_blink();
                    p.cursor_on = !p.cursor_on || !blinking;
                    Some(blinking)
                });
                if again == Some(Some(true)) {
                    v.request_redraw(pane);
                }
            })
        });
    }

    /// The window's content in pixels, for laying out its panes.
    fn geometry(&self, st: &State, titlebar: f64) -> Geometry {
        geometry_of(&st.win.config, st.win.renderer.metrics(), st.win.scale, self.frame().size, titlebar)
    }

    /// Lay out the panes for the view's size, title bar and fonts, and
    /// draw them at once.
    fn resize(&self) {
        if self.lay_out() {
            // Draw now so live resizing doesn't show a stretched old frame.
            self.draw();
        }
    }

    /// Lay out the panes for the view's size, title bar, fonts and shares:
    /// each shown pane view gets its rectangle (panes a zoom hides are
    /// hidden, keeping theirs) and each session its grid, its program told
    /// only when the grid or cell size changed; each divider gets its view.
    /// A pane that moved or changed size is drawn again. False before the
    /// view is in a window.
    fn lay_out(&self) -> bool {
        let Some(window) = self.window() else { return false };
        let mtm = MainThreadMarker::from(self);
        let titlebar = self.titlebar_height();
        let min = self.with_state(|st| {
            st.win.scale = window.backingScaleFactor();
            st.win.top_inset = titlebar;
            let size = self.frame().size;
            let scale = st.win.scale;
            let layer = &self.ivars().layer;
            layer.setContentsScale(scale);
            let pixels = ((size.width * scale).round(), (size.height * scale).round());
            layer.setDrawableSize(NSSize::new(pixels.0, pixels.1));
            crate::images::window_size(&term_core::ImagePool::process(), self.ivars().id, pixels.0 as u32, pixels.1 as u32);
            let layout = st.tree.layout(&self.geometry(st, titlebar));
            let m = st.win.renderer.metrics();
            let alone = layout.panes.len() == 1;
            for (id, p) in st.panes.iter_mut() {
                let shown = layout.rect(*id).is_some();
                if p.view.isHidden() == shown {
                    p.view.setHidden(!shown);
                }
            }
            for &(id, rect) in &layout.panes {
                let Some(p) = st.panes.get_mut(&id) else { continue };
                if p.rect != rect {
                    p.rect = rect;
                    p.session.mark_dirty();
                }
                // One pane fills the view, in points as the view has them.
                let frame = if alone {
                    NSRect::new(NSPoint::new(0.0, 0.0), size)
                } else {
                    points(rect, scale)
                };
                p.view.setFrame(frame);
                if let Some(f) = &p.find {
                    f.bar.place(frame.size.width, if rect.y == 0 { titlebar } else { 0.0 });
                }
                if p.image_scale != scale {
                    p.image_scale = scale;
                    p.session.set_image_scale(scale as f32);
                }
                let pad = pane_padding(&st.win.config, titlebar, scale, rect);
                let (cols, lines) = layout::grid_size((frame.size.width, frame.size.height), scale, m.into(), pad);
                let size = pty_size(cols, lines, m);
                if p.told != Some(told(&size)) {
                    p.told = Some(told(&size));
                    let _ = p.session.resize(size);
                }
            }
            // A view over each divider, above the panes.
            while st.divider_views.len() > layout.dividers.len() {
                if let Some(v) = st.divider_views.pop() {
                    v.removeFromSuperview();
                }
            }
            for (i, d) in layout.dividers.iter().enumerate() {
                let frame = points(d.grab, scale);
                match st.divider_views.get(i) {
                    Some(v) => v.setFrame(frame),
                    None => {
                        let v = DividerView::new(mtm, self, i, frame);
                        self.addSubview(&v);
                        st.divider_views.push(v);
                    }
                }
            }
            let colour = st.win.divider;
            st.dividers = layout
                .dividers
                .iter()
                .map(|d| rect_instance(d.line, colour, transition::slot::DIVIDER))
                .collect();
            st.grabs = layout.dividers;
            // With several panes the window can't be made smaller than
            // they all fit at their minimum.
            let base = *st.win.base_min.get_or_insert_with(|| window.contentMinSize());
            let min = if st.tree.len() > 1 {
                let (w, h) = st.tree.min_size(&self.geometry(st, titlebar));
                NSSize::new((w as f64 / scale).max(base.width), (h as f64 / scale).max(base.height))
            } else {
                base
            };
            (st.win.min != Some(min)).then(|| {
                st.win.min = Some(min);
                min
            })
        });
        if let Some(Some(min)) = min {
            window.setContentMinSize(min);
        }
        window.invalidateCursorRectsForView(self);
        true
    }

    /// Add a pane's view below the dividers' views (and below the other
    /// panes', which it doesn't overlap), so the dividers win hit testing.
    fn add_pane_view(&self, view: &PaneView) {
        self.addSubview_positioned_relativeTo(view, NSWindowOrderingMode::Below, None);
    }

    fn reload_fonts(&self, size: Option<f64>) {
        let Some(window) = self.window() else { return };
        let scale = window.backingScaleFactor();
        let changed = self.with_state(|st| {
            let win = &mut st.win;
            let size = size.unwrap_or(win.font_size);
            if size == win.font_size && scale == win.scale {
                return false;
            }
            win.font_size = size;
            win.scale = scale;
            // Only the size changes: fallbacks, faces, line height, cell
            // width and features stay as configured.
            win.renderer.set_font_config(&win.config.font_config(size), scale);
            true
        });
        if changed == Some(true) {
            self.resize();
        }
    }

    /// A reloaded config: colours, fonts, padding, opacity and shortcuts
    /// change at once, option-as-meta and copy-on-select from the next key
    /// or selection; `notices` are written into the terminals. A new
    /// `font_size` replaces a size set with Cmd+= / Cmd+-; otherwise the
    /// window keeps its size.
    pub fn apply_config(&self, config: Rc<Config>, notices: &[String]) {
        let Some(window) = self.window() else { return };
        let scale = window.backingScaleFactor();
        let visible = self.visible();
        let changed = self.with_state(|st| {
            let win = &mut st.win;
            let old = std::mem::replace(&mut win.config, config.clone());
            let old_fonts = old.font_config(win.font_size);
            if config.font_size != old.font_size {
                win.font_size = config.font_size;
            }
            let fonts = config.font_config(win.font_size);
            if fonts != old_fonts || scale != win.scale {
                win.scale = scale;
                win.renderer.set_font_config(&fonts, scale);
            }
            win.binds = config.keybinds();
            let shown = transition::slots(&win.theme, win.divider);
            win.theme = window_theme(&config, win.appearance, win.settings);
            win.divider = divider_colour(&config, win.appearance);
            begin_transition(win, shown, visible);
            for p in st.panes.values() {
                let mut t = p.session.lock_term();
                let report = crate::config::set_term_colors(&mut t.state, &win.theme);
                if report {
                    p.session.report(t.state.take_replies());
                }
                t.state.set_cursor_default(config.cursor_style.shape(), config.cursor_blink);
                if t.state.ambiguous_width() != config.ambiguous_width {
                    // For output from now on; what is on screen stays.
                    t.state.set_ambiguous_width(config.ambiguous_width);
                }
                // Not into a full-screen program (vim, the preferences):
                // the lines would land under its cursor in its colours.
                if !p.settings && !t.state.alt_screen_active() {
                    for n in notices {
                        crate::session::write_late_notice(&mut t, n);
                    }
                }
            }
            self.ivars().layer.setOpaque(config.opacity >= 1.0 || win.settings);
        });
        if changed.is_some() {
            // Padding or cell size may have changed the grid.
            self.resize();
            self.redraw_all();
            // `title_status` may have changed.
            self.refresh_title();
        }
    }

    /// Light or dark, as macOS draws this view.
    fn appearance(&self) -> Appearance {
        let names = unsafe { [objc2_app_kit::NSAppearanceNameAqua, objc2_app_kit::NSAppearanceNameDarkAqua] };
        use objc2_app_kit::NSAppearanceCustomization;
        let best = self
            .effectiveAppearance()
            .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&names));
        match best {
            Some(name) if &*name == unsafe { objc2_app_kit::NSAppearanceNameAqua } => Appearance::Light,
            _ => Appearance::Dark,
        }
    }

    /// The window's appearance switched between light and dark (macOS, or
    /// the `appearance` setting): a light/dark theme pair changes colours,
    /// and programs that set mode 2031 are told if they went from dark to
    /// light or back.
    fn appearance_changed(&self) {
        let appearance = self.appearance();
        let visible = self.visible();
        let changed = self.with_state(|st| {
            let win = &mut st.win;
            if win.appearance == appearance {
                return false;
            }
            win.appearance = appearance;
            let theme = window_theme(&win.config, appearance, win.settings);
            let divider = divider_colour(&win.config, appearance);
            let recolour = divider != win.divider
                || theme.foreground != win.theme.foreground
                || theme.background != win.theme.background
                || theme.palette != win.theme.palette
                || theme.cursor != win.theme.cursor
                || theme.selection != win.theme.selection;
            let shown = transition::slots(&win.theme, win.divider);
            win.theme = theme;
            win.divider = divider;
            begin_transition(win, shown, visible);
            for p in st.panes.values() {
                let mut t = p.session.lock_term();
                if crate::config::set_term_colors(&mut t.state, &win.theme) {
                    // Programs that asked (mode 2031) hear of it.
                    p.session.report(t.state.take_replies());
                }
            }
            recolour
        });
        if changed == Some(true) {
            // The dividers take the new colour.
            self.resize();
            self.redraw_all();
        }
    }

    /// The window became key or stopped being key: the focused pane's
    /// program is told if it asked (focus events).
    fn set_focus(&self, focused: bool) {
        let report = self.with_state(|st| {
            st.win.focused = focused;
            let id = st.tree.focused();
            let p = st.focused()?;
            p.cursor_on = true;
            let report = p.session.lock_term().state.modes().focus_events;
            Some((id, report, p.view.clone()))
        });
        if let Some(Some((id, report, view))) = report {
            if report {
                view.send_raw(if focused { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
            }
            self.request_redraw(id);
        }
    }

    fn shut_down(&self) {
        let observed = self.ivars().observed.borrow_mut().take();
        if let Some(window) = observed {
            unsafe { window.removeObserver_forKeyPath(self, &NSString::from_str(LAYOUT_RECT)) };
        }
        self.finish_recording(None);
        let st = self.ivars().state.borrow_mut().take();
        if let Some(st) = st {
            st.win.link.invalidate();
            drop(st); // hangs up on the programs
            crate::notify::closed(MainThreadMarker::from(self), self.ivars().id);
            BURSTS.with(|b| b.borrow_mut().forget(self.ivars().id));
        }
        VIEWS.with(|v| v.borrow_mut().remove(&self.ivars().id));
        crate::images::window_size(&term_core::ImagePool::process(), self.ivars().id, 0, 0);
    }

    /// Pane `pane`'s program ended: the pane closes (the window with its
    /// last pane), or holds to show why. A pane that has closed already
    /// is ignored.
    fn program_exited(&self, pane: PaneId, outcome: Outcome) {
        match outcome {
            Outcome::Closed => self.close_pane(pane),
            Outcome::Hold => {
                // The session wrote why into the terminal. A pane stopped
                // for a crash while drawing is held already.
                let mut settings = false;
                let testing = self.with_pane(pane, |win, p| {
                    let crashed = p.crash.is_some();
                    p.held = true;
                    p.session.mark_dirty();
                    settings = p.settings;
                    win.test.is_some() && !crashed
                });
                if settings {
                    // Settings stopped (it crashed, or quit) with its tab
                    // kept open: no one is left to end its preview.
                    crate::app::preview_theme(term_prefs::Preview::Cancel);
                }
                self.request_redraw(pane);
                if testing == Some(true) {
                    // Smoke tests can't send a dump command once the
                    // program is gone: dump what the held window shows.
                    self.control("dump");
                }
            }
        }
    }

    pub(crate) fn close_window(&self) {
        if let Some(window) = self.window() {
            window.close();
        }
    }

    /// Split the focused pane: a new pane right of or below it, running
    /// the user's shell in its working directory (even in a pin or a
    /// `-e PROGRAM` window), gets focus. Beeps if the two wouldn't both
    /// fit at their minimum size.
    pub(crate) fn split(&self, dir: Dir) {
        let mtm = MainThreadMarker::from(self);
        let titlebar = self.titlebar_height();
        let cwd = self.foreground_cwd();
        let id = self.ivars().id;
        let made = self.with_state(|st| {
            let g = self.geometry(st, titlebar);
            let mut tree = st.tree.clone();
            let new = tree.split(dir, &g).ok()?;
            let rect = tree.layout(&g).rect(new)?;
            let win = &st.win;
            let (scale, m) = (win.scale, win.renderer.metrics());
            let view = PaneView::new(mtm, self, new, points(rect, scale));
            let mut command = shell_command(&win.config);
            command.cwd = cwd.clone().or(command.cwd);
            let pane = spawn_pane(
                id,
                &win.config,
                &win.theme,
                m,
                win.test.is_some(),
                NewPane {
                    id: new,
                    view: view.clone(),
                    command: &command,
                    grid: grid_of(&win.config, titlebar, scale, m, rect),
                    notices: &[],
                    program: false,
                    settings: false,
                    adopt: None,
                },
            );
            match pane {
                Ok(pane) => {
                    st.tree = tree;
                    st.panes.insert(new, pane);
                    Some(view)
                }
                Err(e) => {
                    eprintln!("terminal-decline: could not open a pane: {e}");
                    None
                }
            }
        });
        let Some(Some(view)) = made else {
            NSBeep();
            return;
        };
        self.add_pane_view(&view);
        self.resize();
        if let Some(window) = self.window() {
            window.makeFirstResponder(Some(&view));
        }
    }

    /// Shell > Close: the focused pane, asking first with `confirm_close`
    /// while a program runs in it. The last pane closes the tab, through
    /// `performClose:` (so `windowShouldClose:` asks).
    pub(crate) fn close_focused_pane(&self) {
        let Some(window) = self.window() else { return };
        let Some((pane, count, ask)) = self.with_state(|st| (st.tree.focused(), st.tree.len(), st.win.config.confirm_close)) else {
            return;
        };
        if count == 1 {
            window.performClose(None);
            return;
        }
        match ask.then(|| self.pane_program(pane)).flatten() {
            Some(program) => self.ask_to_close(&window, Closing::Pane, &[program], AfterAsk::Pane(pane)),
            None => self.close_pane(pane),
        }
    }

    /// Close pane `pane` without asking: focus moves first (to the pane
    /// taking its space), then its program is hung up on and the others
    /// are laid out again. The last pane closes the window instead.
    pub(crate) fn close_pane(&self, pane: PaneId) {
        let Some(window) = self.window() else { return };
        let Some((was_focused, next)) = self
            .with_state(|st| {
                if !st.panes.contains_key(&pane) {
                    return None;
                }
                let was_focused = st.tree.focused() == pane;
                Some((
                    was_focused,
                    st.tree.close(pane).and_then(|n| st.panes.get(&n)).map(|p| p.view.clone()),
                ))
            })
            .flatten()
        else {
            return;
        };
        let Some(next) = next else {
            self.close_window();
            return;
        };
        if self.with_state(|st| st.win.recording.as_ref().is_some_and(|r| r.pane == pane)) == Some(true) {
            self.finish_recording(None);
        }
        if was_focused {
            // Never leave the first responder a view being removed.
            window.makeFirstResponder(Some(&next));
        }
        let Some(Some(closed)) = self.with_state(|st| st.panes.remove(&pane)) else {
            return;
        };
        closed.view.removeFromSuperview();
        if closed.settings {
            crate::app::settings_closed(&window);
        }
        drop(closed); // hangs up on its program
        self.resize();
    }

    /// Shell > Close Window: every tab of this window, asking once with
    /// `confirm_close` while programs run in any of them.
    pub(crate) fn close_all_tabs(&self) {
        let Some(window) = self.window() else { return };
        let ask = self.with_state(|st| st.win.config.confirm_close).unwrap_or(false);
        let programs: Vec<String> = if ask {
            tab_views(&window).iter().flat_map(|v| v.running_programs()).collect()
        } else {
            Vec::new()
        };
        if programs.is_empty() {
            close_tabs(&window);
        } else {
            self.ask_to_close(&window, Closing::Window, &programs, AfterAsk::AllTabs);
        }
    }

    /// Focus the pane next to the focused one in `dir`; beep if there is
    /// none.
    pub(crate) fn focus_towards(&self, dir: Direction) {
        let titlebar = self.titlebar_height();
        let next = self.with_state(|st| st.tree.neighbour(dir, &self.geometry(st, titlebar))).flatten();
        match next {
            Some(pane) => self.focus_pane(pane),
            None => NSBeep(),
        }
    }

    /// Focus the next (or previous) pane in order, wrapping; beep with one.
    pub(crate) fn focus_in_order(&self, forward: bool) {
        let next = self
            .with_state(|st| (st.tree.len() > 1).then(|| if forward { st.tree.next() } else { st.tree.previous() }))
            .flatten();
        match next {
            Some(pane) => self.focus_pane(pane),
            None => NSBeep(),
        }
    }

    /// Give pane `pane` the keys. Its view becoming first responder does
    /// the rest ([`Self::pane_focused`]). A pane hidden by a zoom ends the
    /// zoom first, so a shown view gets the keys.
    pub(crate) fn focus_pane(&self, pane: PaneId) {
        let unzoom =
            self.with_state(|st| st.panes.contains_key(&pane) && st.tree.zoomed().is_some_and(|z| z != pane) && st.tree.toggle_zoom());
        if unzoom == Some(true) {
            self.resize();
        }
        let view = self.with_state(|st| st.panes.get(&pane).map(|p| p.view.clone())).flatten();
        if let (Some(view), Some(window)) = (view, self.window()) {
            window.makeFirstResponder(Some(&view));
        }
    }

    /// Pane `pane`'s view became first responder (a click, a focus
    /// action, a split or a close): it is the focused pane. Its program is
    /// told if it asked for focus events and the window is key, and its
    /// title becomes the window's. `clicked`: a left click in it did this,
    /// which goes nowhere else.
    pub(crate) fn pane_focused(&self, pane: PaneId, clicked: bool) {
        let mut unzoomed = false;
        let got = self.with_state(|st| {
            let zoomed = st.tree.zoomed();
            st.tree.focus(pane);
            unzoomed = zoomed != st.tree.zoomed();
            let key = st.win.focused;
            let p = st.panes.get_mut(&pane)?;
            p.swallow_click = clicked;
            p.cursor_on = true;
            let report = key && p.session.lock_term().state.modes().focus_events;
            Some((report, p.view.clone()))
        });
        let Some(Some((report, view))) = got else { return };
        if unzoomed {
            // Focusing a pane hidden by a zoom ended it.
            self.resize();
            self.redraw_all();
        }
        if report {
            view.send_raw(b"\x1b[I".to_vec());
        }
        self.refresh_title();
        self.request_redraw(pane);
    }

    /// Window > Move Divider …: the divider nearest the focused pane that
    /// runs across `dir` moves one cell that way. Beeps when there is none,
    /// it can't move further, or a pane is zoomed.
    pub(crate) fn resize_pane(&self, dir: Direction) {
        let titlebar = self.titlebar_height();
        let moved = self.with_state(|st| {
            if st.tree.zoomed().is_some() {
                return false;
            }
            let m = st.win.renderer.metrics();
            let by = match dir {
                Direction::Left | Direction::Right => m.cell_width,
                Direction::Up | Direction::Down => m.cell_height,
            } as i32;
            let g = self.geometry(st, titlebar);
            st.tree.resize(dir, by, &g)
        });
        match moved {
            Some(true) => self.resize(),
            _ => NSBeep(),
        }
    }

    /// Window > Equalize Panes: every run of side-by-side or stacked
    /// panes gets equal shares. Beeps with one pane.
    pub(crate) fn equalize(&self) {
        let done = self.with_state(|st| {
            let several = st.tree.len() > 1;
            if several {
                st.tree.equalize();
            }
            several
        });
        match done {
            Some(true) => self.resize(),
            _ => NSBeep(),
        }
    }

    /// Window > Zoom Pane: the focused pane fills the window, the others
    /// hidden but still running; again, they all come back. Beeps with one
    /// pane.
    pub(crate) fn toggle_zoom(&self) {
        let done = self.with_state(|st| {
            st.dragging = None;
            st.tree.toggle_zoom()
        });
        if done != Some(true) {
            NSBeep();
            return;
        }
        self.resize();
        // Panes coming back show what changed while they were hidden.
        self.redraw_all();
    }

    /// The mouse went down on divider `index` at `at` (the window view's
    /// points): a drag starts, keeping where in the grab zone it was
    /// taken; a double-click evens out the panes it runs between instead.
    pub(crate) fn divider_down(&self, index: usize, at: NSPoint, clicks: isize) {
        let equalized = self.with_state(|st| {
            let d = st.grabs.get(index)?;
            if clicks >= 2 {
                let path = d.path.clone();
                st.dragging = None;
                return Some(st.tree.equalize_at(&path));
            }
            let (pos, start) = match d.dir {
                Dir::Right => ((at.x * st.win.scale).round() as i32, d.line.x),
                Dir::Down => ((at.y * st.win.scale).round() as i32, d.line.y),
            };
            st.dragging = Some(Drag {
                path: d.path.clone(),
                dir: d.dir,
                offset: pos - start,
            });
            Some(false)
        });
        if equalized == Some(Some(true)) {
            self.resize();
        }
        if let Some(Some(dir)) = self.with_state(|st| st.grabs.get(index).map(|d| d.dir)) {
            resize_cursor(dir).set();
        }
    }

    /// The mouse dragged a divider to `at` (the window view's points): its
    /// split moves there (clamped so no pane gets under its minimum), the
    /// panes are laid out and their programs told of new grid sizes, and
    /// the next display-link tick draws it at the input rate (120 Hz).
    pub(crate) fn divider_dragged(&self, at: NSPoint) {
        let titlebar = self.titlebar_height();
        let moved = self.with_state(|st| {
            let drag = st.dragging.as_ref()?;
            let pos = match drag.dir {
                Dir::Right => (at.x * st.win.scale).round() as i32,
                Dir::Down => (at.y * st.win.scale).round() as i32,
            };
            let (path, to, dir) = (drag.path.clone(), pos - drag.offset, drag.dir);
            let g = self.geometry(st, titlebar);
            st.win.last_input = Instant::now();
            Some((st.tree.drag(&path, to, &g), dir))
        });
        let Some(Some((moved, dir))) = moved else { return };
        if moved {
            self.lay_out();
            self.unpause();
        }
        resize_cursor(dir).set();
    }

    /// The mouse came up after a divider drag.
    pub(crate) fn divider_up(&self) {
        self.with_state(|st| st.dragging = None);
        if let Some(window) = self.window() {
            window.invalidateCursorRectsForView(self);
        }
    }

    /// The test-only `drag N PX`: divider `index` is dragged, as with the
    /// mouse (taken 2 px past its line), until its line starts at `to`
    /// pixels. Beeps if there is no such divider.
    fn drag_divider(&self, index: usize, to: i32) {
        let Some(Some((line, dir, scale))) = self.with_state(|st| st.grabs.get(index).map(|d| (d.line, d.dir, st.win.scale))) else {
            NSBeep();
            return;
        };
        let pt = |x: i32, y: i32| NSPoint::new(x as f64 / scale, y as f64 / scale);
        let (from, at) = match dir {
            Dir::Right => (pt(line.x + 2, line.y + line.h / 2), pt(to + 2, line.y + line.h / 2)),
            Dir::Down => (pt(line.x + line.w / 2, line.y + 2), pt(line.x + line.w / 2, to + 2)),
        };
        self.divider_down(index, from, 1);
        self.divider_dragged(at);
        self.divider_up();
        self.draw();
    }

    /// Bytes typed for the latency harness (its `type` command), into the
    /// focused pane.
    pub fn latency_type(&self, bytes: Vec<u8>) {
        let t = latency::now();
        let Some(view) = self.focused_view() else { return };
        view.with(|_, p| p.key_t0 = Some((t, t)));
        view.send_input(bytes);
        view.with(|_, p| p.key_t0 = None);
    }

    /// The programs running in the panes' foregrounds, other than their
    /// shells (see [`crate::running`]), in pane order. Never the Settings
    /// pane's: closing it loses nothing, each edit being written at once.
    pub fn running_programs(&self) -> Vec<String> {
        let pids = self
            .with_state(|st| {
                st.tree
                    .ids()
                    .iter()
                    .filter_map(|id| st.panes.get(id))
                    .filter(|p| !p.settings)
                    .filter_map(|p| p.session.pid())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        pids.into_iter().filter_map(crate::running::foreground_program).collect()
    }

    /// The program running in pane `pane`'s foreground, as
    /// [`Self::running_programs`].
    fn pane_program(&self, pane: PaneId) -> Option<String> {
        let pid = self
            .with_state(|st| st.panes.get(&pane).filter(|p| !p.settings).and_then(|p| p.session.pid()))
            .flatten()?;
        crate::running::foreground_program(pid)
    }

    /// The command line of the program in the focused pane's shell's
    /// foreground, to save for a restart (see
    /// [`crate::running::foreground_command`]). Only for a pane running
    /// the login shell.
    pub fn command_to_restore(&self) -> Option<String> {
        let pid = self
            .with_state(|st| st.focused().and_then(|p| p.runs_shell.then(|| p.session.pid()).flatten()))
            .flatten()?;
        crate::running::foreground_command(pid)
    }

    /// The panes to save for a restart, when there are several: the tree
    /// with its shares, and each pane's folder, the command in its shell's
    /// foreground and whether it ran the window's program.
    pub fn saved_panes(&self) -> Option<SavedPanes> {
        self.with_state(|st| {
            if st.tree.len() < 2 {
                return None;
            }
            let panes = &st.panes;
            Some(st.tree.save(|id| {
                match panes.get(&id) {
                    Some(p) => SavedPane {
                        cwd: pane_cwd(p),
                        command: p
                            .runs_shell
                            .then(|| p.session.pid())
                            .flatten()
                            .and_then(crate::running::foreground_command),
                        program: p.program,
                    },
                    None => SavedPane::default(),
                }
            }))
        })
        .flatten()
    }

    /// Type a restored tab's `command` at its shell's prompt once the
    /// shell is reading, without running it (see
    /// [`Session::type_at_prompt`]). Only for a pane running the login
    /// shell.
    pub fn type_at_prompt(&self, command: String) {
        self.with_state(|st| {
            if let Some(p) = st.focused()
                && p.runs_shell
            {
                p.session.type_at_prompt(command);
            }
        });
    }

    /// Whether the Settings pane's program is on the alternate screen, and
    /// whether a chord is being recorded (see [`crate::app::settings_status`]).
    pub(crate) fn settings_status(&self) -> (bool, bool) {
        self.with_state(|st| {
            let recording = st.win.recording.is_some();
            let alt = st
                .panes
                .values()
                .find(|p| p.settings)
                .is_some_and(|p| p.session.lock_term().state.alt_screen_active());
            (alt, recording)
        })
        .unwrap_or((false, false))
    }

    /// Whether this is the Settings tab.
    pub fn is_settings(&self) -> bool {
        self.with_state(|st| st.win.settings).unwrap_or(false)
    }

    /// The Settings pane gets the keys (Cmd+, with the Settings tab open).
    pub fn focus_settings(&self) {
        let pane = self
            .with_state(|st| st.panes.iter().find(|(_, p)| p.settings).map(|(id, _)| *id))
            .flatten();
        if let Some(pane) = pane {
            self.focus_pane(pane);
        }
    }

    /// Type `bytes` into the Settings pane as if typed (the test-only
    /// `type-settings`).
    pub fn type_bytes(&self, bytes: Vec<u8>) {
        let view = self
            .with_state(|st| st.panes.values().find(|p| p.settings).map(|p| p.view.clone()))
            .flatten();
        if let Some(view) = view {
            view.send_input(bytes);
        }
    }

    /// Pane `pane`'s preferences sent `OSC 7771 ; <token> ; <what>`
    /// ([`crate::prefs::token`]): record a chord,
    /// or preview a theme in every window. Only heard from the Settings
    /// pane.
    fn prefs_asked(&self, pane: PaneId, what: &str) {
        if self.with_state(|st| st.panes.get(&pane).is_some_and(|p| p.settings)) != Some(true) {
            return;
        }
        // Only the preferences themselves know the token, not what they
        // run or show.
        let Some(what) = term_prefs::preview::verify(what, crate::prefs::token()) else {
            return;
        };
        if what == "record-chord" {
            self.start_recording(pane);
        } else if let Some(preview) = term_prefs::Preview::parse(what) {
            crate::app::preview_theme(preview);
        }
    }

    /// Pane `pane`'s preferences asked for a chord (`OSC 7771 ;
    /// record-chord`, only heard in the Settings tab): the next key down
    /// in this window is taken before the menus, the input method and
    /// `[keybind]` see it, and sent back as `OSC 7771 ; chord ; <chord>`.
    /// Plain Esc, the window losing focus (a tab switch too) or
    /// [`RECORD_TIMEOUT`] cancel.
    fn start_recording(&self, pane: PaneId) {
        let Some(Some(view)) = self.with_state(|st| st.panes.get(&pane).filter(|p| p.settings).map(|p| p.view.clone())) else {
            return;
        };
        // Asked again while waiting: the old monitor goes, silently.
        if let Some(old) = self.with_state(|st| st.win.recording.take()).flatten() {
            crate::background::remove_monitor(&old.monitor);
        }
        if !self.window().is_some_and(|w| crate::background::is_key(&w)) {
            view.send_raw(keybind::chord_reply(None).into_bytes());
            return;
        }
        let id = self.ivars().id;
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            let mut taken = false;
            with_view(id, |v| taken = v.recorded_key(unsafe { event.as_ref() }));
            if taken { std::ptr::null_mut() } else { event.as_ptr() }
        });
        let monitor = crate::background::add_local_monitor(NSEventMask::KeyDown, handler);
        let Some(monitor) = monitor else {
            view.send_raw(keybind::chord_reply(None).into_bytes());
            return;
        };
        let Some(token) = self.with_state(|st| {
            st.win.recordings += 1;
            st.win.recording = Some(Recording { monitor, pane });
            st.win.recordings
        }) else {
            return;
        };
        let _ = DispatchQueue::main().after(DispatchTime::NOW.time(RECORD_TIMEOUT.as_nanos() as i64), move || {
            with_view(id, |v| {
                if v.with_state(|st| st.win.recordings == token && st.win.recording.is_some()) == Some(true) {
                    v.finish_recording(None);
                }
            })
        });
    }

    /// A key down while a chord is recorded: whether it was taken.
    fn recorded_key(&self, event: &NSEvent) -> bool {
        if self.with_state(|st| st.win.recording.is_some()) != Some(true) {
            return false;
        }
        let ours = self
            .window()
            .zip(event.window(MainThreadMarker::from(self)))
            .is_some_and(|(w, e)| std::ptr::eq::<NSWindow>(&*w, &*e));
        if !ours {
            return false;
        }
        let input = crate::pane_view::key_input(event);
        let mods = crate::keys::flags::COMMAND | crate::keys::flags::CONTROL | crate::keys::flags::OPTION | crate::keys::flags::SHIFT;
        if input.keycode == 0x35 && input.flags & mods == 0 {
            self.finish_recording(None);
            return true;
        }
        let chord = self.with_state(|st| keybind::recorded_chord(&input, &st.win.binds)).flatten();
        // A key with no character or name: keep waiting.
        if chord.is_some() {
            self.finish_recording(chord);
        }
        true
    }

    /// Ends a chord recording, if one is waiting, replying with `chord`
    /// or, for `None`, that it was cancelled.
    fn finish_recording(&self, chord: Option<keybind::Chord>) {
        let Some(Some((rec, view))) = self.with_state(|st| {
            let rec = st.win.recording.take()?;
            let view = st.panes.get(&rec.pane).map(|p| p.view.clone());
            Some((rec, view))
        }) else {
            return;
        };
        crate::background::remove_monitor(&rec.monitor);
        if let Some(view) = view {
            view.send_raw(keybind::chord_reply(chord.as_ref()).into_bytes());
        }
    }

    /// Ask, in a sheet on `window`, whether to close `what` while
    /// `programs` run in it; do `then` if so.
    fn ask_to_close(&self, window: &NSWindow, what: Closing, programs: &[String], then: AfterAsk) {
        if window.attachedSheet().is_some() {
            return;
        }
        let mtm = MainThreadMarker::from(self);
        let (title, text) = crate::running::close_question(what, programs);
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(&title));
        alert.setInformativeText(&NSString::from_str(&text));
        alert.addButtonWithTitle(&NSString::from_str("Close"));
        alert.addButtonWithTitle(&NSString::from_str("Cancel"));
        let id = self.ivars().id;
        let done = RcBlock::new(move |response: NSModalResponse| {
            if response == NSAlertFirstButtonReturn {
                with_view(id, |v| match then {
                    AfterAsk::Pane(pane) => v.close_pane(pane),
                    AfterAsk::Window => v.close_window(),
                    AfterAsk::AllTabs => {
                        if let Some(window) = v.window() {
                            close_tabs(&window);
                        }
                    }
                });
            }
        });
        alert.beginSheetModalForWindow_completionHandler(window, Some(&done));
        crate::background::hide(&alert.window());
    }
}

/// The session hooks of pane `pane` in window `id`: each finds its window
/// and pane again on the main thread, and does nothing once either is gone.
fn pane_hooks(id: usize, pane: PaneId, test: bool, settings: bool) -> Hooks {
    let mut hooks = Hooks::new(
        Arc::new(move || wake(id, pane)),
        Box::new(move |outcome| DispatchQueue::main().exec_async(move || with_view(id, |v| v.program_exited(pane, outcome)))),
    );
    if test {
        hooks.on_control = Some(Box::new(move |cmd| {
            DispatchQueue::main().exec_async(move || {
                with_view(id, |v| {
                    if v.with_state(|st| st.panes.contains_key(&pane)) == Some(true) {
                        v.control(&cmd)
                    }
                })
            });
        }));
    }
    if let Some(lat) = latency::get() {
        // The latency log counts by window (one pane each until B2.5).
        hooks.on_parsed = Some(Box::new(move || lat.parsed(id)));
    }
    hooks.on_events = Some(Box::new(move |events| {
        DispatchQueue::main().exec_async(move || with_view(id, |v| v.events(pane, events)));
    }));
    if settings {
        hooks.on_prefs = Some(Box::new(move |what: String| {
            DispatchQueue::main().exec_async(move || with_view(id, |v| v.prefs_asked(pane, &what)));
        }));
    }
    hooks
}

/// Present `panes` like [`Renderer::present_panes`], with handlers that
/// time the frame for the keys it shows (latency harness only). The
/// renderer doesn't see this drawable, so a window hidden right after such
/// a frame keeps its previous surface instead of offering it for purging.
fn present_timed(
    renderer: &mut Renderer,
    layer: &CAMetalLayer,
    clear: term_render::Rgba,
    panes: &[PaneDraw],
    dividers: &[RectInstance],
    lat: &'static Latency,
    keys: Vec<latency::Key>,
) {
    let Some(drawable) = layer.nextDrawable() else {
        lat.write(&keys);
        return;
    };
    let cmd = renderer.encode_panes(&drawable.texture(), clear, panes, dividers);
    let frame = Arc::new(FrameKeys::new(keys));
    let on_gpu = frame.clone();
    let done = RcBlock::new(move |cb: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
        on_gpu.gpu_done(lat, unsafe { cb.as_ref() }.GPUEndTime());
    });
    unsafe { cmd.addCompletedHandler(RcBlock::as_ptr(&done)) };
    let on_glass = frame.clone();
    let shown = RcBlock::new(move |d: NonNull<ProtocolObject<dyn MTLDrawable>>| {
        on_glass.presented(lat, unsafe { d.as_ref() }.presentedTime());
    });
    unsafe { drawable.addPresentedHandler(RcBlock::as_ptr(&shown)) };
    cmd.presentDrawable(ProtocolObject::from_ref(&*drawable));
    frame.committed();
    cmd.commit();
}

pub(crate) fn padding_for(config: &Config, titlebar: f64, scale: f64) -> Padding {
    layout::padding(config.padding, titlebar, scale)
}

/// The padding inside a pane at `rect`: the title bar's inset on top of it
/// only when it touches the window's top.
/// How many of `p`'s grid rows its find bar is over.
pub(crate) fn rows_under_find_bar(win: &Win, p: &Pane, titlebar: f64) -> usize {
    let top = p.rect.y == 0;
    let bottom = (if top { titlebar } else { 0.0 } + crate::find_bar::HEIGHT) * win.scale;
    let grid_top = pane_padding(&win.config, titlebar, win.scale, p.rect).top as f64;
    let cell = win.renderer.metrics().cell_height.max(1) as f64;
    ((bottom - grid_top) / cell).ceil().max(0.0) as usize
}

pub(crate) fn pane_padding(config: &Config, titlebar: f64, scale: f64, rect: PxRect) -> Padding {
    padding_for(config, if rect.y == 0 { titlebar } else { 0.0 }, scale)
}

/// What closing does once its question is answered with Close.
#[derive(Clone, Copy)]
enum AfterAsk {
    Pane(PaneId),
    Window,
    AllTabs,
}

/// The window views of `window`'s tabs (just its own without tabs).
fn tab_views(window: &NSWindow) -> Vec<Retained<WindowView>> {
    let tabs: Vec<Retained<NSWindow>> = match window.tabbedWindows() {
        Some(tabs) => tabs.iter().collect(),
        None => vec![objc2::Message::retain(window)],
    };
    let views: Vec<Retained<WindowView>> = VIEWS.with(|v| v.borrow().values().filter_map(|w| w.load()).collect());
    views
        .into_iter()
        .filter(|v| v.window().is_some_and(|w| tabs.iter().any(|t| std::ptr::eq::<NSWindow>(&**t, &*w))))
        .collect()
}

/// Close every tab of `window` without asking.
fn close_tabs(window: &NSWindow) {
    let tabs: Vec<Retained<NSWindow>> = match window.tabbedWindows() {
        Some(tabs) => tabs.iter().collect(),
        None => vec![objc2::Message::retain(window)],
    };
    for tab in tabs {
        tab.close();
    }
}

/// A pane about to be opened.
struct NewPane<'a> {
    id: PaneId,
    view: Retained<PaneView>,
    command: &'a Command,
    /// Columns and lines.
    grid: (usize, usize),
    /// Written at the top before the program's output.
    notices: &'a [String],
    /// `command` is the window's own program, not the user's shell.
    program: bool,
    /// It runs the preferences.
    settings: bool,
    /// It is the agent API's session: see [`Launch::adopt`].
    adopt: Option<crate::ctl::window::Adopt>,
}

/// Start pane `n` of window `window`: its terminal and program.
fn spawn_pane(window: usize, config: &Config, theme: &Theme, m: term_font::Metrics, test: bool, n: NewPane) -> std::io::Result<Pane> {
    let (cols, lines) = n.grid;
    let mut t = Term::new(cols, lines, config.scrollback, term_core::ImagePool::process());
    t.state.set_ambiguous_width(config.ambiguous_width);
    t.state.set_cursor_default(config.cursor_style.shape(), config.cursor_blink);
    // Before the program starts: nothing to report yet.
    crate::config::set_term_colors(&mut t.state, theme);
    for notice in n.notices {
        crate::session::write_notice(&mut t, notice);
    }
    let mut hooks = pane_hooks(window, n.id, test, n.settings);
    let adopted = n.adopt.map(|a| a.attach(&mut hooks));
    // The login shell (or a command that is just a shell) gets the shell
    // integration; Settings and other programs don't.
    let mut command = n.command.clone();
    if config.shell_integration && !n.settings && !command.direct {
        command.integration = crate::Paths::current().shell_integration_dir();
    }
    let session = Session::spawn(&command, pty_size(cols, lines, m), t, hooks)?;
    if let Some(ready) = adopted {
        ready(&session);
    }
    Ok(Pane {
        view: n.view,
        session,
        render: PaneRender::new(),
        theme: None,
        rect: PxRect::default(),
        cursor_on: true,
        blink_scheduled: false,
        sync_since: None,
        marked: String::new(),
        scroll_accum: 0.0,
        mouse_button: None,
        selecting: None,
        dragged: false,
        block: None,
        pending: None,
        autoscroll: None,
        autoscroll_scheduled: false,
        held: false,
        text_key: None,
        keys_down: KeysDown::default(),
        key_t0: None,
        awaiting_echo: None,
        runs_shell: !n.settings && n.command.argv.is_empty(),
        program: n.program,
        told: Some(told(&pty_size(cols, lines, m))),
        image_scale: 0.0,
        settings: n.settings,
        title: None,
        swallow_click: false,
        builds: 0,
        crash: None,
        panic_next: None,
        find: None,
        find_query: crate::search::Query::default(),
        activity: crate::activity::Activity::default(),
    })
}

/// The user's login shell with the config's environment, for a split:
/// not the window's own program, even in a pin (splits design §11).
fn shell_command(config: &Config) -> Command {
    Command {
        argv: Vec::new(),
        cwd: config.cwd.clone(),
        env: config.env.clone().into_iter().collect(),
        shell: None,
        direct: false,
        integration: None,
    }
}

/// A pane's working directory: the one its shell reported (OSC 7), else
/// its foreground program's.
fn pane_cwd(p: &Pane) -> Option<String> {
    let reported = p.session.lock_term().state.cwd().map(str::to_owned);
    reported.or_else(|| p.session.foreground_cwd())
}

/// The window's theme changed from `shown` (the colours on screen, as
/// [`transition::slots`]): play `theme_transition` to the new ones while
/// the window can be seen, from where a transition still playing has got
/// to. Hidden windows (background tabs) just take the new theme. With
/// Reduce Motion on, a wave, pulse or glimmer is a plain fade.
fn begin_transition(win: &mut Win, shown: Slots, visible: bool) {
    let new = transition::slots(&win.theme, win.divider);
    let config = &win.config;
    let kind = config.theme_transition.kind().reduced(reduce_motion());
    let ms = config.theme_transition_ms.min(crate::config::THEME_TRANSITION_MS_MAX);
    let now = Instant::now();
    let playing = win.transition.take();
    if visible {
        win.transition = Transition::start(playing.as_ref(), shown, new, kind, Duration::from_millis(ms as u64), now);
    }
}

/// macOS's Reduce Motion (System Settings > Accessibility > Display).
fn reduce_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// The divider colour for `config` in `appearance`.
fn divider_colour(config: &Config, appearance: Appearance) -> Rgba {
    term_render::rgb(config.palette(appearance).divider())
}

/// A rectangle in window pixels as the view's points.
fn points(r: PxRect, scale: f64) -> NSRect {
    let pt = |v: i32| v as f64 / scale;
    NSRect::new(NSPoint::new(pt(r.x), pt(r.y)), NSSize::new(pt(r.w), pt(r.h)))
}

/// Columns and lines of a pane at `rect` (window pixels).
fn grid_of(config: &Config, titlebar: f64, scale: f64, m: term_font::Metrics, rect: PxRect) -> (usize, usize) {
    let r = points(rect, scale);
    let pad = pane_padding(config, titlebar, scale, rect);
    layout::grid_size((r.size.width, r.size.height), scale, m.into(), pad)
}

/// A window's content of `size` points, in pixels, for laying out its
/// panes: the smallest a pane may be is 10 columns by 3 lines.
fn geometry_of(config: &Config, m: term_font::Metrics, scale: f64, size: NSSize, titlebar: f64) -> Geometry {
    let pad = padding_for(config, 0.0, scale);
    Geometry {
        bounds: PxRect {
            x: 0,
            y: 0,
            w: (size.width * scale).round() as i32,
            h: (size.height * scale).round() as i32,
        },
        top_inset: (titlebar * scale).round() as i32,
        min: (
            (10 * m.cell_width + 2 * pad.x) as i32,
            (3 * m.cell_height + pad.top + pad.bottom) as i32,
        ),
        divider: scale.round().max(1.0) as i32,
    }
}

/// Panes whose snapshot or frame build panicked this frame: each program
/// is killed and its pane holds with the crash's message in a fresh
/// terminal, as a parse panic does; the glyph textures and the fonts'
/// shaping are forgotten, so every pane is built again; a second panic in
/// the same pane within [`CRASH_REPEAT`] stops it being built.
fn crash_panes(win: &mut Win, panes: &mut HashMap<PaneId, Pane>, crashed: &[PaneId]) {
    win.renderer.forget_glyphs();
    let now = Instant::now();
    for id in crashed {
        let Some(p) = panes.get_mut(id) else { continue };
        let again = p.crash.is_some_and(|c| now.duration_since(c.at) < CRASH_REPEAT);
        let count = p.crash.map_or(0, |c| c.count) + 1;
        p.crash = Some(Crash {
            at: now,
            count,
            stopped: again,
        });
        p.held = true;
        p.session.stop_for_bug();
        // A fresh terminal with the message only, also after a panic that
        // left the terminal untouched (building unlocked): what it shows
        // likely led to the panic, and drawing it again would only panic
        // again, hiding the message behind the repeat limit.
        let mut t = p.session.lock_term();
        crate::session::replace_term(&mut t);
        crate::session::write_bug(&mut t, crate::crash::take_last_log());
    }
    for p in panes.values() {
        p.session.mark_dirty();
    }
    win.link.setPaused(false);
}

/// With the `test-hooks` feature, the test-only `build-panic` makes a
/// pane's build panic here.
#[cfg(feature = "test-hooks")]
fn test_build_panic(now: bool) {
    if now {
        panic!("test-hooks: building this pane's frame was asked to panic");
    }
}

#[cfg(not(feature = "test-hooks"))]
#[inline(always)]
fn test_build_panic(_now: bool) {}

/// A filled rectangle at `r` (window pixels).
fn rect_instance(r: PxRect, color: Rgba, slot: usize) -> RectInstance {
    RectInstance {
        tag: term_render::frame::tag_of(slot),
        x: r.x.clamp(0, i16::MAX as i32) as i16,
        y: r.y.clamp(0, i16::MAX as i32) as i16,
        w: r.w.clamp(0, u16::MAX as i32) as u16,
        h: r.h.clamp(0, u16::MAX as i32) as u16,
        color,
    }
}

/// What a program was told of its size, to compare.
fn told(s: &Size) -> (u16, u16, u16, u16) {
    (s.cols, s.lines, s.cell_width, s.cell_height)
}

/// The cursor over a divider's grab zone: for side-by-side panes (`Right`)
/// the column resize cursor, for stacked ones the row resize cursor (macOS
/// 15's, else the older left-right and up-down arrows).
// The older cursors are deprecated in macOS 15, which has the newer.
#[allow(deprecated)]
fn resize_cursor(dir: Dir) -> Retained<NSCursor> {
    let newer = |name| <NSCursor as objc2::ClassType>::class().class_method(name).is_some();
    match dir {
        Dir::Right if newer(sel!(columnResizeCursor)) => NSCursor::columnResizeCursor(),
        Dir::Right => NSCursor::resizeLeftRightCursor(),
        Dir::Down if newer(sel!(rowResizeCursor)) => NSCursor::rowResizeCursor(),
        Dir::Down => NSCursor::resizeUpDownCursor(),
    }
}

fn pty_size(cols: usize, lines: usize, m: term_font::Metrics) -> Size {
    Size {
        cols: cols as u16,
        lines: lines as u16,
        cell_width: m.cell_width as u16,
        cell_height: m.cell_height as u16,
    }
}

/// Window content size (points) for the configured grid.
pub fn content_size(config: &Config, m: term_font::Metrics, scale: f64, titlebar: f64) -> NSSize {
    let cell = CellSize::from(m);
    let (w, h) = layout::content_size(
        config.columns as usize,
        config.lines as usize,
        scale,
        cell,
        padding_for(config, titlebar, scale),
    );
    NSSize::new(w, h)
}

pub fn delegate_of(view: &WindowView) -> &ProtocolObject<dyn NSWindowDelegate> {
    ProtocolObject::from_ref(view)
}

/// Test-only control (`DECLINE_TEST_DUMP`); see [`crate::control`].
impl WindowView {
    fn control(&self, cmd: &str) {
        let Some(c) = Control::parse(cmd) else {
            eprintln!("terminal-decline: unknown test control command {cmd:?}");
            return;
        };
        self.with_state(|st| {
            if let Some(test) = &mut st.win.test {
                test.queue.push_back(c);
            }
        });
        self.run_controls();
    }

    /// Run queued commands until one has to wait (a dump in progress).
    fn run_controls(&self) {
        loop {
            let next = self.with_state(|st| {
                let test = st.win.test.as_mut()?;
                if test.dump != DumpState::Idle {
                    return None;
                }
                test.queue.pop_front()
            });
            match next.flatten() {
                None => return,
                Some(Control::Dump) => {
                    self.with_state(|st| {
                        if let Some(test) = &mut st.win.test {
                            test.dump = DumpState::Requested;
                        }
                    });
                    self.redraw_all();
                }
                Some(Control::Resize { cols, lines }) => self.set_grid(cols as usize, lines as usize),
                Some(Control::Font(delta)) => {
                    let size = self.with_state(|st| (st.win.font_size + delta as f64).max(6.0));
                    self.reload_fonts(size);
                }
                Some(Control::Tab) => {
                    if let Some(window) = self.window() {
                        crate::app::open_tab_beside(&window);
                    }
                }
                Some(Control::Front) => bring_to_front(self.ivars().id),
                Some(Control::Drop(paths)) => {
                    if let Some(view) = self.focused_view() {
                        view.paste_text(&crate::desktop::dropped_text(&paths));
                    }
                }
                Some(Control::Settings) => crate::app::show_settings_now(),
                Some(Control::EditConfig) => crate::app::edit_config_now(),
                Some(Control::UpdateCheck) => crate::update::sparkle::check_in_background(),
                Some(Control::MemoryPressure) => {
                    crate::memory::relieve("memory pressure (test control)");
                }
                Some(Control::TypeSettings(bytes)) => crate::app::type_into_settings(bytes),
                Some(Control::Key { keycode, flags }) => post_key(MainThreadMarker::from(self), keycode, flags),
                Some(Control::Menu(action)) => {
                    let app = NSApplication::sharedApplication(MainThreadMarker::from(self));
                    let sent = unsafe { app.sendAction_to_from(objc2::runtime::Sel::register(action.selector()), None, None) };
                    if !sent {
                        NSBeep();
                    }
                }
                Some(Control::Hover(cell)) => {
                    if let Some(view) = self.focused_view() {
                        view.set_hover(cell);
                    }
                }
                Some(Control::Click(line, col, true)) => {
                    if !self.focused_view().is_some_and(|view| view.move_shell_cursor(line, col)) {
                        NSBeep();
                    }
                }
                Some(Control::Click(line, col, false)) => {
                    let pane = self.focused_pane();
                    let uri = pane.and_then(|pane| {
                        self.with_pane(pane, |_, p| {
                            let t = p.session.lock_term();
                            crate::links::under(&t.state, t.state.point_at(line, col))
                        })
                    });
                    match (self.focused_view(), uri.flatten()) {
                        (Some(view), Some(uri)) => view.open_link(&uri),
                        _ => NSBeep(),
                    }
                }
                Some(Control::Mouse {
                    kind,
                    line,
                    col,
                    shift,
                    option,
                }) => {
                    if let Some(view) = self.focused_view() {
                        view.test_mouse(kind, line, col, shift, option);
                    }
                }
                Some(Control::Scroll(lines)) => {
                    if let Some(pane) = self.focused_pane() {
                        self.with_pane(pane, |_, p| p.session.lock_term().state.scroll_view(lines));
                        self.request_redraw(pane);
                    }
                }
                Some(Control::Close) => {
                    if let Some(window) = self.window() {
                        window.performClose(None);
                    }
                }
                Some(Control::Split(dir)) => self.split(dir),
                Some(Control::ClosePane) => self.close_focused_pane(),
                Some(Control::Drag { index, to }) => self.drag_divider(index, to),
                Some(Control::ResizePane(dir)) => self.resize_pane(dir),
                Some(Control::Equalize) => self.equalize(),
                Some(Control::Zoom) => self.toggle_zoom(),
                Some(Control::BuildPanic { index, snapshot }) => {
                    let pane = self.with_state(|st| st.tree.ids().get(index).copied()).flatten();
                    if let Some(pane) = pane {
                        self.with_pane(pane, |_, p| p.panic_next = Some(snapshot));
                        self.request_redraw(pane);
                    }
                }
                Some(Control::FindClick(index)) => {
                    let view = self
                        .with_state(|st| st.tree.ids().get(index).and_then(|id| st.panes.get(id)).map(|p| p.view.clone()))
                        .flatten();
                    match view {
                        Some(view) => view.click_find_bar(),
                        None => NSBeep(),
                    }
                }
                Some(Control::Save) => {
                    let path = self.with_state(|st| st.win.test.as_ref().map(|t| t.path.clone())).flatten();
                    if let Some(path) = path {
                        let mut name = path.into_os_string();
                        name.push(".saved");
                        crate::app::write_saved_windows(std::path::Path::new(&name));
                    }
                }
                Some(Control::Focus(target)) => match target {
                    FocusTarget::Next => self.focus_in_order(true),
                    FocusTarget::Previous => self.focus_in_order(false),
                    FocusTarget::Towards(dir) => self.focus_towards(dir),
                    FocusTarget::Index(n) => match self.with_state(|st| st.tree.ids().get(n).copied()).flatten() {
                        Some(pane) => self.focus_pane(pane),
                        None => NSBeep(),
                    },
                },
                Some(Control::Find(text)) => match self.focused_view() {
                    Some(view) => view.find_text(&text),
                    None => NSBeep(),
                },
                Some(Control::Quit) => {
                    crate::app::quit_now();
                    return;
                }
            }
        }
    }

    /// Under test control, a link that would be opened is listed in the
    /// dump instead: whether it was.
    pub(crate) fn opened_in_test(&self, url: &str) -> bool {
        self.with_state(|st| match &mut st.win.test {
            Some(test) => {
                test.opened.push(url.to_string());
                true
            }
            None => false,
        })
        .unwrap_or(false)
    }

    /// Resize the window to show `cols` × `lines` (in its one pane).
    pub(crate) fn set_grid(&self, cols: usize, lines: usize) {
        let Some(window) = self.window() else { return };
        let titlebar = self.titlebar_height();
        let size = self.with_state(|st| {
            let pad = padding_for(&st.win.config, titlebar, st.win.scale);
            layout::content_size(cols, lines, st.win.scale, st.win.renderer.metrics().into(), pad)
        });
        if let Some((w, h)) = size {
            window.setContentSize(NSSize::new(w, h));
        }
    }

    /// Write the dump once the GPU has finished the frame drawn for it.
    fn poll_dump(&self) {
        let ready = self.with_state(|st| {
            let test = st.win.test.as_ref()?;
            let DumpState::Drawn(frame) = test.dump else { return None };
            Some(st.win.renderer.frames_completed() >= frame)
        });
        match ready.flatten() {
            None => {}
            Some(false) => {
                let id = self.ivars().id;
                let _ = DispatchQueue::main().after(DispatchTime::NOW.time(2_000_000), move || with_view(id, |v| v.poll_dump()));
            }
            Some(true) => {
                let me = self.window();
                let tabs = me.as_ref().and_then(|w| w.tabbedWindows());
                let window = Window {
                    id: self.ivars().id,
                    tabs: tabs.as_ref().map_or(1, |t| t.len()),
                    tab: tabs
                        .zip(me)
                        .and_then(|(t, me)| t.iter().position(|w| std::ptr::eq::<NSWindow>(&*w, &*me)))
                        .unwrap_or(0),
                    visible: self.visible(),
                    sheet: self.window().is_some_and(|w| w.attachedSheet().is_some()),
                    top_inset: self.titlebar_height(),
                    title: self.window().map(|w| w.title().to_string()).unwrap_or_default(),
                    badge: objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(self))
                        .dockTile()
                        .badgeLabel()
                        .map(|b| b.to_string()),
                    active: objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(self)).isActive(),
                    secure_input: crate::app::secure_input_held(),
                    // Read before this window's state is borrowed: it may
                    // be the Settings tab.
                    settings: crate::app::settings_status(),
                };
                self.with_state(|st| write_dump(st, window));
                self.run_controls();
            }
        }
    }
}

/// A key press and release handed to the app for the key window (the
/// test-only `key` command): it goes where a real key goes, local event
/// monitors and the menus first. It is handled before this returns, not
/// queued: the next control command or dump (which may be the very next
/// thing in the pty's output) must see what the key did. The characters are the
/// ASCII-capable layout's; the arrow keys get AppKit's function-key
/// characters and flags, as real ones do, so menu shortcuts match them.
fn post_key(mtm: MainThreadMarker, keycode: u16, flags: u64) {
    let app = NSApplication::sharedApplication(mtm);
    let Some(window) = app.keyWindow() else { return };
    let mut flags = NSEventModifierFlags(flags as usize);
    if (123..=126).contains(&keycode) {
        flags |= NSEventModifierFlags::Function | NSEventModifierFlags::NumericPad;
    }
    // Return and the arrows type no printable character; AppKit gives
    // them these, which their menu key equivalents match.
    let (plain, shifted) = match keycode {
        36 => (Some('\r'), Some('\r')),
        123..=126 => {
            let c = char::from_u32([0xF702, 0xF703, 0xF701, 0xF700][keycode as usize - 123]);
            (c, c)
        }
        _ => crate::ascii_layout::chars(keycode),
    };
    let typed = if flags.contains(NSEventModifierFlags::Shift) {
        shifted
    } else {
        plain
    };
    let text = |c: Option<char>| NSString::from_str(&c.map(String::from).unwrap_or_default());
    for kind in [objc2_app_kit::NSEventType::KeyDown, objc2_app_kit::NSEventType::KeyUp] {
        let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            kind,
            NSPoint::new(0.0, 0.0),
            flags,
            0.0,
            window.windowNumber(),
            None,
            &text(typed),
            &text(plain),
            false,
            keycode,
        );
        if let Some(event) = event {
            crate::background::send_key(&app, &window, &event);
        }
    }
}

/// What a dump says about the window, read from AppKit.
struct Window {
    id: usize,
    tabs: usize,
    tab: usize,
    visible: bool,
    sheet: bool,
    top_inset: f64,
    title: String,
    badge: Option<String>,
    active: bool,
    secure_input: bool,
    settings: (bool, bool),
}

/// One line of JSON about the window, its focused pane and every pane.
fn write_dump(st: &mut State, window: Window) {
    use std::io::Write;
    let focused = st.tree.focused();
    let zoomed = st.tree.zoomed();
    let mut images = control::ImagesDump::default();
    let every: Vec<PaneDump> = st
        .tree
        .ids()
        .into_iter()
        .filter_map(|id| {
            let p = st.panes.get(&id)?;
            let t = p.session.lock_term();
            let g = t.state.graphics();
            images.count += g.images().count();
            images.ready += g
                .images()
                .filter(|i| matches!(i.state, term_core::image::ImageState::Ready))
                .count();
            if t.state.lines() > 0 {
                let top = t.state.point_at(0, 0).line;
                let bottom = t.state.point_at(t.state.lines() - 1, 0).line + 1;
                images.placements_in_view += g.placements_in(top, bottom).count();
            }
            Some(PaneDump {
                id,
                rect: [p.rect.x, p.rect.y, p.rect.w, p.rect.h],
                cols: t.state.cols(),
                lines: t.state.lines(),
                focused: id == focused,
                shown: zoomed.is_none_or(|z| z == id),
                builds: p.builds,
                crashes: p.crash.map_or(0, |c| c.count),
                history_bytes: t.state.scrollback_bytes(),
                screen: t.state.dump(),
            })
        })
        .collect();
    let pool = term_core::ImagePool::process();
    images.enabled = pool.enabled();
    images.bytes = pool.used();
    images.budget = pool.limit();
    let dividers = st.dividers.len();
    let State { win, panes, .. } = st;
    let Some(p) = panes.get(&focused) else { return };
    // The focused pane's colours, a program's included.
    let theme = p.theme.as_ref().unwrap_or(&win.theme);
    let pad_top = pane_padding(&win.config, window.top_inset, win.scale, p.rect).top;
    let Some(test) = &mut win.test else { return };
    test.seq += 1;
    test.dump = DumpState::Idle;
    let find = p.find.as_ref().map(|f| {
        let t = p.session.lock_term();
        let status = f.search.status(&t.state);
        let top = t.state.point_at(0, 0).line;
        let current_row = f
            .search
            .current(&t.state)
            .and_then(|h| h.start.line.checked_sub(top))
            .filter(|&r| r < t.state.lines() as u64)
            .map(|r| r as usize);
        control::FindDump {
            query: f.bar.query().text,
            count: f.bar.count_text(),
            hits: status.count,
            current: status.current,
            current_row,
            highlights: f.drawn,
            focused: f.bar.has_focus(),
        }
    });
    let (screen, cols, lines, hovered_link, view_offset, selection) = {
        let t = p.session.lock_term();
        (
            t.state.dump(),
            t.state.cols(),
            t.state.lines(),
            t.state.hover().and_then(|at| crate::links::under(&t.state, at)),
            t.state.display_offset(),
            t.state.selection_text(),
        )
    };
    let m = win.renderer.metrics();
    let (mask_texture, color_texture) = win.renderer.glyph_texture_sizes();
    let dump = Dump {
        seq: test.seq,
        window: window.id,
        tabs: window.tabs,
        tab: window.tab,
        visible: window.visible,
        sheet: window.sheet,
        hidden: win.hidden,
        pad_top,
        notifications: crate::notify::stats(),
        badge: window.badge,
        title: window.title,
        active: window.active,
        secure_input: window.secure_input,
        settings_ready: window.settings.0,
        settings_recording: window.settings.1,
        update: crate::update::status(),
        screen,
        cols,
        lines,
        panes: every,
        hovered_link,
        opened: test.opened.clone(),
        view_offset,
        selection,
        find,
        dividers,
        zoomed,
        frames: win.renderer.frames_completed(),
        glyphs: p.render.frame.glyphs.len(),
        rects: p.render.frame.rects.len(),
        mask_texture,
        color_texture,
        cell_width: m.cell_width,
        cell_height: m.cell_height,
        font_size: win.font_size,
        scale: win.scale,
        foreground: format!("#{:06x}", crate::config::rgb_of(theme.foreground)),
        background: format!("#{:06x}", crate::config::rgb_of(theme.background)),
        phys_footprint: control::phys_footprint(),
        memory_reliefs: crate::memory::reliefs(),
        images,
    };
    let line = serde_json::to_string(&dump).expect("dump serialises");
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&test.path)
        .and_then(|mut f| writeln!(f, "{line}"));
    if let Err(e) = written {
        eprintln!("terminal-decline: writing {}: {e}", test.path.display());
    }
}

/// Tell the image pool that `win` hid (after its renderer dropped its
/// textures) or shows (before it draws), once per change (images.md §5.5).
fn images_shown(win: &mut Win, panes: &HashMap<PaneId, Pane>, shown: bool) {
    if win.images_hidden != shown || !crate::images::enabled() {
        return;
    }
    win.images_hidden = !shown;
    let owners = panes.values().filter_map(|p| p.session.image_owner());
    crate::images::set_shown(&term_core::ImagePool::process(), owners, shown);
}
