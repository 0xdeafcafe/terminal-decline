//! The window view: one per window (or tab). Its backing layer is the
//! window's only `CAMetalLayer`, with one display link, and it draws every
//! pane of the window in one frame (splits design §1). The panes are
//! [`PaneView`]s laid over it, which take the keys, the input method and
//! the mouse; each owns a session (program, PTY and terminal). For now a
//! window has one pane, filling it.

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
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSEvent, NSEventMask, NSEventModifierFlags, NSModalResponse, NSPasteboard,
    NSPasteboardTypeString, NSResponder, NSView, NSWindow, NSWindowDelegate,
};
use objc2_foundation::{
    NSArray, NSKeyValueObservingOptions, NSNotification, NSObjectNSKeyValueObserverRegistration, NSObjectProtocol, NSPoint, NSRect,
    NSRunLoop, NSRunLoopCommonModes, NSSize, NSString,
};
use objc2_metal::{MTLCommandBuffer, MTLDrawable};
use objc2_quartz_core::{CADisplayLink, CALayer, CAMetalDrawable, CAMetalLayer};
use term_core::Term;
use term_pty::{Command, Size};
use term_render::{FrameInput, PaneDraw, PaneRender, Renderer, Theme};

use crate::config::Config;
use crate::control::{self, Control, Dump};
use crate::frame_policy::{self, Decision, ECHO_WAIT, PaneFrame, WindowTick};
use crate::keybind::{self, Keybinds};
use crate::keys::{KeyInput, KeyModes, KeysDown};
use crate::latency::{self, FrameKeys, Latency};
use crate::layout::{self, CellSize, Padding};
use crate::pane_view::PaneView;
use crate::panes::{Geometry, PaneId, Panes, PxRect};
use crate::select::{Block, Gesture};
use crate::session::{Events, Hooks, Outcome, Session};
use crate::theme::Appearance;

pub(crate) const BLINK_INTERVAL: Duration = Duration::from_millis(530);

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
    /// This is the Settings tab, which the app started itself: a fixed
    /// title, always opaque, not a running program for `confirm_close`,
    /// and the only session whose chord recording is honoured.
    pub(crate) settings: bool,
    /// A chord being recorded for the preferences (OSC 7771).
    recording: Option<Recording>,
    /// Bumped by each recording, so a timeout cancels only its own.
    recordings: u64,
}

/// What each pane keeps: its view, its session and what the renderer
/// built for it last, and the input and selection state of its terminal.
pub(crate) struct Pane {
    pub(crate) view: Retained<PaneView>,
    pub(crate) session: Session,
    pub(crate) render: PaneRender,
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
}

pub(crate) struct State {
    pub(crate) win: Win,
    tree: Panes,
    pub(crate) panes: HashMap<PaneId, Pane>,
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
    static NEXT_ID: StdCell<usize> = const { StdCell::new(1) };
}

pub(crate) fn with_view(id: usize, f: impl FnOnce(&WindowView)) {
    let view = VIEWS.with(|v| v.borrow().get(&id).and_then(|w| w.load()));
    if let Some(view) = view {
        f(&view);
    }
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

/// Bring view `id`'s window or tab to the front (a click on its
/// notification).
pub(crate) fn bring_to_front(id: usize) {
    with_view(id, |v| {
        if let Some(window) = v.window() {
            window.makeKeyAndOrderFront(None);
            objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(v)).activate();
            // An inactive app gets no key notifications to notice the switch.
            sync_visibility_soon();
        }
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

        /// With `confirm_close`, ask first while a program runs.
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, window: &NSWindow) -> bool {
            let ask = self.with_state(|st| st.win.config.confirm_close).unwrap_or(false);
            match self.running_program().filter(|_| ask) {
                Some(program) => {
                    self.ask_to_close(window, &program);
                    false
                }
                None => true,
            }
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
        view.addSubview(&pane);
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
    pub fn start(&self, config: Rc<Config>, launch: Launch) -> std::io::Result<()> {
        let id = self.ivars().id;
        let Some((tree, view)) = self.ivars().first.borrow_mut().take() else {
            return Err(std::io::Error::other("started twice"));
        };
        let pane_id = tree.focused();
        let layer = &self.ivars().layer;
        let renderer = launch.renderer;
        let settings = launch.settings;
        let runs_shell = !settings && launch.command.argv.is_empty();
        let opaque = config.opacity >= 1.0 || settings;
        term_render::configure_layer(layer, renderer.device(), opaque);
        layer.setContentsScale(launch.scale);

        let appearance = self.appearance();
        let theme = window_theme(&config, appearance, settings);
        let m = renderer.metrics();
        let size = self.frame().size;
        let pad = padding_for(&config, self.titlebar_height(), launch.scale);
        let (cols, lines) = layout::grid_size((size.width, size.height), launch.scale, m.into(), pad);

        let mut t = Term::new(cols, lines, config.scrollback);
        t.state.set_ambiguous_width(config.ambiguous_width);
        t.state.set_default_colors(crate::config::query_colors(&theme));
        for n in launch.notices.iter() {
            crate::session::write_notice(&mut t, n);
        }
        let test = std::env::var_os(control::ENV).filter(|p| !p.is_empty()).map(|p| TestControl {
            path: PathBuf::from(p),
            queue: VecDeque::new(),
            dump: DumpState::Idle,
            seq: 0,
        });
        let hooks = pane_hooks(id, pane_id, test.is_some(), settings);
        let session = Session::spawn(&launch.command, pty_size(cols, lines, m), t, hooks)?;

        let link = unsafe { self.displayLinkWithTarget_selector(self, sel!(tick:)) };
        unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes) };
        link.setPaused(false);

        let pane = Pane {
            view,
            session,
            render: PaneRender::new(),
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
            autoscroll: None,
            autoscroll_scheduled: false,
            held: false,
            text_key: None,
            keys_down: KeysDown::default(),
            key_t0: None,
            awaiting_echo: None,
            runs_shell,
        };
        *self.ivars().state.borrow_mut() = Some(State {
            win: Win {
                renderer,
                link,
                font_size: config.font_size,
                binds: config.keybinds(),
                config,
                theme,
                appearance,
                scale: launch.scale,
                focused: true,
                idle_ticks: 0,
                frames: 0,
                rate: 0,
                last_input: Instant::now(),
                test,
                top_inset: self.titlebar_height(),
                hidden: 0,
                shown: true,
                settings,
                recording: None,
                recordings: 0,
            },
            tree,
            panes: HashMap::from([(pane_id, pane)]),
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
    /// in the tab bar, a bell bounces the Dock icon, a notification is
    /// shown if this window or tab isn't the one in front, and progress
    /// goes on the Dock icon.
    fn events(&self, pane: PaneId, events: Events) {
        let mtm = MainThreadMarker::from(self);
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        // Only the focused pane's title is the window's.
        let Some((fixed_title, focused)) = self.with_state(|st| {
            (
                st.win.settings || st.win.config.title.is_some(),
                st.panes.contains_key(&pane).then_some(st.tree.focused() == pane),
            )
        }) else {
            return;
        };
        let Some(focused) = focused else { return };
        let window = self.window();
        if let Some(window) = &window
            && !fixed_title
            && focused
            && let Some(title) = &events.title
        {
            window.setTitle(&NSString::from_str(title));
        }
        if events.bell {
            // Bounce the Dock icon if the app isn't in front.
            if !app.isActive() {
                app.requestUserAttention(objc2_app_kit::NSRequestUserAttentionType::InformationalRequest);
            }
        }
        if let Some(n) = events.notification {
            let in_front = app.isActive() && window.as_ref().is_some_and(|w| w.isKeyWindow());
            if !in_front {
                // OSC 9 has no title: use the window's (programs set it).
                let title = n
                    .title
                    .filter(|t| !t.is_empty())
                    .or_else(|| window.as_ref().map(|w| w.title().to_string()))
                    .unwrap_or_else(|| "Terminal Decline".into());
                crate::notify::post(mtm, self.ivars().id, &title, &n.body);
            }
        }
        if let Some(p) = events.progress {
            crate::notify::set_progress(mtm, self.ivars().id, p);
        }
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

    pub fn foreground_cwd(&self) -> Option<String> {
        self.with_state(|st| {
            let p = st.focused()?;
            p.session
                .lock_term()
                .state
                .cwd()
                .map(str::to_owned)
                .or_else(|| p.session.foreground_cwd())
        })
        .flatten()
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
    pub(crate) fn request_redraw(&self, pane: PaneId) {
        self.with_pane(pane, |_, p| p.session.mark_dirty());
        self.unpause();
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
            let behind = w
                .tabGroup()
                .and_then(|g| g.selectedWindow())
                .is_some_and(|front| !std::ptr::eq::<NSWindow>(&*front, &*w));
            !behind && (w.isKeyWindow() || w.occlusionState().contains(objc2_app_kit::NSWindowOcclusionState::Visible))
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
            }
            changed
        });
        if changed == Some(true) {
            if visible {
                self.redraw_all();
            } else {
                self.trim_drawables();
            }
        }
        visible
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
        let State { win, panes, .. } = st;
        // Under test control, draw even when covered by other windows, so a
        // dump never waits forever for a frame.
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
            // Only a frame that may be drawn needs the terminal's lock.
            let t = ((dirty || waited_out) && visible).then(|| crate::session::lock(term));
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
            let frame = frame_policy::pane(dirty || waited_out, synchronized, sync_elapsed, echo.filter(|_| !waited_out));
            ticks.push((*id, frame, t));
        }
        let frames: Vec<PaneFrame> = ticks.iter().map(|(_, f, _)| *f).collect();
        let decision = frame_policy::combine(
            &frames,
            &WindowTick {
                visible,
                minimised,
                focused: win.focused,
                idle_ticks: win.idle_ticks,
                last_input_elapsed: win.last_input.elapsed(),
            },
        );
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
        for ((id, frame, t), (_, term)) in ticks.iter_mut().zip(&terms) {
            let (id, frame) = (*id, *frame);
            let p = panes.get_mut(&id).expect("shown pane");
            // A pane built against emptied glyph textures or old fonts is
            // built again before it is drawn, held or not.
            let build = frame == PaneFrame::Build || !win.renderer.is_current(&p.render);
            if !build {
                continue;
            }
            let t = t.get_or_insert_with(|| crate::session::lock(term));
            if let Some(text) = t.state.take_clipboard() {
                clipboard = Some(text);
            }
            if t.state.cursor_blink() && win.focused && !p.blink_scheduled {
                p.blink_scheduled = true;
                blinking.push(id);
            }
            let pad = pane_padding(&win.config, titlebar, win.scale, p.rect);
            let input = FrameInput {
                theme: &win.theme,
                metrics: win.renderer.metrics(),
                padding: (pad.x, pad.top),
                cursor_on: p.cursor_on,
                focused: win.focused,
                preedit: Some(p.marked.as_str()),
            };
            win.renderer.build_pane(&mut p.render, &mut t.state, &input);
        }
        // With the terminals still locked: the keys whose output this frame shows.
        let timed = latency::get().map(|l| (l, l.take_for_frame(self.ivars().id, rate)));
        drop(ticks);
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
        match timed {
            Some((lat, keys)) if !keys.is_empty() => present_timed(&mut win.renderer, &self.ivars().layer, clear, &draws, lat, keys),
            _ => win.renderer.present_panes(&self.ivars().layer, clear, &draws, &[]),
        }
        drop(draws);
        win.frames += 1;
        let mut poll_dump = false;
        if let Some(test) = &mut win.test
            && test.dump == DumpState::Requested
        {
            test.dump = DumpState::Drawn(win.renderer.frames_submitted());
            poll_dump = true;
        }
        drop(guard);
        for pane in blinking {
            self.schedule_blink(pane);
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

    fn trim_drawables(&self) {
        let layer = &self.ivars().layer;
        let size = layer.drawableSize();
        layer.setDrawableSize(NSSize::new(1.0, 1.0));
        layer.setDrawableSize(size);
    }

    pub(crate) fn schedule_blink(&self, pane: PaneId) {
        let id = self.ivars().id;
        let _ = DispatchQueue::main().after(DispatchTime::NOW.time(BLINK_INTERVAL.as_nanos() as i64), move || {
            with_view(id, |v| {
                let again = v.with_pane(pane, |win, p| {
                    p.blink_scheduled = false;
                    let blinking = win.focused && p.session.lock_term().state.cursor_blink();
                    p.cursor_on = !p.cursor_on || !blinking;
                    blinking
                });
                if again == Some(true) {
                    v.request_redraw(pane);
                }
            })
        });
    }

    /// The window's content in pixels, for laying out its panes.
    fn geometry(&self, st: &State, titlebar: f64) -> Geometry {
        let size = self.frame().size;
        let scale = st.win.scale;
        let m = st.win.renderer.metrics();
        let pad = padding_for(&st.win.config, 0.0, scale);
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

    /// Lay out the panes for the view's size, title bar and fonts: each
    /// pane view gets its rectangle and each session its grid.
    fn resize(&self) {
        let Some(window) = self.window() else { return };
        let titlebar = self.titlebar_height();
        self.with_state(|st| {
            st.win.scale = window.backingScaleFactor();
            st.win.top_inset = titlebar;
            let size = self.frame().size;
            let scale = st.win.scale;
            let layer = &self.ivars().layer;
            layer.setContentsScale(scale);
            layer.setDrawableSize(NSSize::new((size.width * scale).round(), (size.height * scale).round()));
            let layout = st.tree.layout(&self.geometry(st, titlebar));
            let m = st.win.renderer.metrics();
            for (id, rect) in layout.panes {
                let Some(p) = st.panes.get_mut(&id) else { continue };
                p.rect = rect;
                // One pane fills the view, in points as the view has them.
                let frame = if st.tree.len() == 1 {
                    NSRect::new(NSPoint::new(0.0, 0.0), size)
                } else {
                    let pt = |v: i32| v as f64 / scale;
                    NSRect::new(NSPoint::new(pt(rect.x), pt(rect.y)), NSSize::new(pt(rect.w), pt(rect.h)))
                };
                p.view.setFrame(frame);
                let pad = pane_padding(&st.win.config, titlebar, scale, rect);
                let (cols, lines) = layout::grid_size((frame.size.width, frame.size.height), scale, m.into(), pad);
                let _ = p.session.resize(pty_size(cols, lines, m));
            }
        });
        // Draw now so live resizing doesn't show a stretched old frame.
        self.draw();
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
            win.theme = window_theme(&config, win.appearance, win.settings);
            for p in st.panes.values() {
                let mut t = p.session.lock_term();
                t.state.set_default_colors(crate::config::query_colors(&win.theme));
                if t.state.ambiguous_width() != config.ambiguous_width {
                    // For output from now on; what is on screen stays.
                    t.state.set_ambiguous_width(config.ambiguous_width);
                }
                // Not into a full-screen program (vim, the preferences):
                // the lines would land under its cursor in its colours.
                if !win.settings && !t.state.alt_screen_active() {
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

    /// The system switched between light and dark: a light/dark theme pair
    /// changes colours. (Colour-scheme reports to programs, mode 2031,
    /// wait for term-core to support them.)
    fn appearance_changed(&self) {
        let appearance = self.appearance();
        let changed = self.with_state(|st| {
            let win = &mut st.win;
            if win.appearance == appearance {
                return false;
            }
            win.appearance = appearance;
            let theme = window_theme(&win.config, appearance, win.settings);
            let recolour = theme.foreground != win.theme.foreground
                || theme.background != win.theme.background
                || theme.palette != win.theme.palette
                || theme.cursor != win.theme.cursor
                || theme.selection != win.theme.selection;
            win.theme = theme;
            for p in st.panes.values() {
                p.session
                    .lock_term()
                    .state
                    .set_default_colors(crate::config::query_colors(&win.theme));
            }
            recolour
        });
        if changed == Some(true) {
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
            crate::notify::set_progress(MainThreadMarker::from(self), self.ivars().id, term_core::Progress::Clear);
        }
        VIEWS.with(|v| v.borrow_mut().remove(&self.ivars().id));
    }

    /// Pane `pane`'s program ended. A pane that has closed already is
    /// ignored.
    fn program_exited(&self, pane: PaneId, outcome: Outcome) {
        match outcome {
            Outcome::Closed => {
                // The last pane closes the window (closing one of several
                // comes with splitting).
                if self.with_state(|st| st.panes.contains_key(&pane)) == Some(true) {
                    self.close_window();
                }
            }
            Outcome::Hold => {
                // The session wrote why into the terminal.
                let testing = self.with_pane(pane, |win, p| {
                    p.held = true;
                    p.session.mark_dirty();
                    win.test.is_some()
                });
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

    /// Bytes typed for the latency harness (its `type` command), into the
    /// focused pane.
    pub fn latency_type(&self, bytes: Vec<u8>) {
        let t = latency::now();
        let Some(view) = self.focused_view() else { return };
        view.with(|_, p| p.key_t0 = Some((t, t)));
        view.send_input(bytes);
        view.with(|_, p| p.key_t0 = None);
    }

    /// The program running in the focused pane's foreground, if it isn't
    /// the shell (see [`crate::running`]). Never the Settings tab: closing
    /// it loses nothing, each edit being written at once.
    pub fn running_program(&self) -> Option<String> {
        let pid = self
            .with_state(|st| {
                let settings = st.win.settings;
                (!settings).then(|| st.focused().and_then(|p| p.session.pid())).flatten()
            })
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

    /// Whether the program in the focused pane is on the alternate screen,
    /// and whether a chord is being recorded (see [`crate::app::settings_status`]).
    pub(crate) fn settings_status(&self) -> (bool, bool) {
        self.with_state(|st| {
            let recording = st.win.recording.is_some();
            let alt = st.focused().is_some_and(|p| p.session.lock_term().state.alt_screen_active());
            (alt, recording)
        })
        .unwrap_or((false, false))
    }

    /// Whether this is the Settings tab.
    pub fn is_settings(&self) -> bool {
        self.with_state(|st| st.win.settings).unwrap_or(false)
    }

    /// Type `bytes` into the focused pane as if typed (the test-only
    /// `type-settings`).
    pub fn type_bytes(&self, bytes: Vec<u8>) {
        if let Some(view) = self.focused_view() {
            view.send_input(bytes);
        }
    }

    /// Pane `pane`'s preferences asked for a chord (`OSC 7771 ;
    /// record-chord`, only heard in the Settings tab): the next key down
    /// in this window is taken before the menus, the input method and
    /// `[keybind]` see it, and sent back as `OSC 7771 ; chord ; <chord>`.
    /// Plain Esc, the window losing focus (a tab switch too) or
    /// [`RECORD_TIMEOUT`] cancel.
    fn start_recording(&self, pane: PaneId) {
        let Some(Some(view)) = self.with_state(|st| st.win.settings.then(|| st.panes.get(&pane).map(|p| p.view.clone())).flatten()) else {
            return;
        };
        // Asked again while waiting: the old monitor goes, silently.
        if let Some(old) = self.with_state(|st| st.win.recording.take()).flatten() {
            unsafe { NSEvent::removeMonitor(&old.monitor) };
        }
        if !self.window().is_some_and(|w| w.isKeyWindow()) {
            view.send_raw(keybind::chord_reply(None).into_bytes());
            return;
        }
        let id = self.ivars().id;
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            let mut taken = false;
            with_view(id, |v| taken = v.recorded_key(unsafe { event.as_ref() }));
            if taken { std::ptr::null_mut() } else { event.as_ptr() }
        });
        let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler) };
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
        unsafe { NSEvent::removeMonitor(&rec.monitor) };
        if let Some(view) = view {
            view.send_raw(keybind::chord_reply(chord.as_ref()).into_bytes());
        }
    }

    /// Ask, in a sheet on `window`, whether to close it while `program`
    /// runs; close it if so.
    fn ask_to_close(&self, window: &NSWindow, program: &str) {
        if window.attachedSheet().is_some() {
            return;
        }
        let mtm = MainThreadMarker::from(self);
        let (title, text) = crate::running::close_question(program);
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(&title));
        alert.setInformativeText(&NSString::from_str(&text));
        alert.addButtonWithTitle(&NSString::from_str("Close"));
        alert.addButtonWithTitle(&NSString::from_str("Cancel"));
        let id = self.ivars().id;
        let done = RcBlock::new(move |response: NSModalResponse| {
            if response == NSAlertFirstButtonReturn {
                with_view(id, |v| v.close_window());
            }
        });
        alert.beginSheetModalForWindow_completionHandler(window, Some(&done));
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
        hooks.on_record_chord = Some(Box::new(move || {
            DispatchQueue::main().exec_async(move || with_view(id, |v| v.start_recording(pane)));
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
    lat: &'static Latency,
    keys: Vec<latency::Key>,
) {
    let Some(drawable) = layer.nextDrawable() else {
        lat.write(&keys);
        return;
    };
    let cmd = renderer.encode_panes(&drawable.texture(), clear, panes, &[]);
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
pub(crate) fn pane_padding(config: &Config, titlebar: f64, scale: f64, rect: PxRect) -> Padding {
    padding_for(config, if rect.y == 0 { titlebar } else { 0.0 }, scale)
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
                Some(Control::TypeSettings(bytes)) => crate::app::type_into_settings(bytes),
                Some(Control::Key { keycode, flags }) => post_key(MainThreadMarker::from(self), keycode, flags),
                Some(Control::Close) => {
                    if let Some(window) = self.window() {
                        window.performClose(None);
                    }
                }
                Some(Control::Quit) => {
                    crate::app::quit_now();
                    return;
                }
            }
        }
    }

    /// Resize the window to show `cols` × `lines` (in its one pane).
    fn set_grid(&self, cols: usize, lines: usize) {
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

/// A key press and release put on the app's event queue for the key
/// window (the test-only `key` command): it goes where a real key goes,
/// local event monitors and the menus first. The characters are the
/// ASCII-capable layout's.
fn post_key(mtm: MainThreadMarker, keycode: u16, flags: u64) {
    let app = NSApplication::sharedApplication(mtm);
    let Some(window) = app.keyWindow() else { return };
    let flags = NSEventModifierFlags(flags as usize);
    let (plain, shifted) = crate::ascii_layout::chars(keycode);
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
            app.postEvent_atStart(&event, false);
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
    badge: Option<String>,
    active: bool,
    secure_input: bool,
    settings: (bool, bool),
}

/// One line of JSON about the window and its focused pane.
fn write_dump(st: &mut State, window: Window) {
    use std::io::Write;
    let focused = st.tree.focused();
    let State { win, panes, .. } = st;
    let Some(p) = panes.get(&focused) else { return };
    let pad_top = pane_padding(&win.config, window.top_inset, win.scale, p.rect).top;
    let Some(test) = &mut win.test else { return };
    test.seq += 1;
    test.dump = DumpState::Idle;
    let (screen, cols, lines) = {
        let t = p.session.lock_term();
        (t.state.dump(), t.state.cols(), t.state.lines())
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
        active: window.active,
        secure_input: window.secure_input,
        settings_ready: window.settings.0,
        settings_recording: window.settings.1,
        screen,
        cols,
        lines,
        frames: win.renderer.frames_completed(),
        glyphs: p.render.frame.glyphs.len(),
        rects: p.render.frame.rects.len(),
        mask_texture,
        color_texture,
        cell_width: m.cell_width,
        cell_height: m.cell_height,
        font_size: win.font_size,
        scale: win.scale,
        foreground: format!("#{:06x}", crate::config::rgb_of(win.theme.foreground)),
        background: format!("#{:06x}", crate::config::rgb_of(win.theme.background)),
        phys_footprint: control::phys_footprint(),
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
