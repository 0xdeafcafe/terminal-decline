//! The terminal view: one per window. Owns the session (program, PTY,
//! terminal state) and the renderer, and turns AppKit events into the inputs
//! of the pure modules (`keys`, `mouse`, `layout`, `frame_policy`).

use std::cell::{Cell as StdCell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchTime};
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSCursor, NSEvent, NSEventModifierFlags, NSPasteboard, NSPasteboardTypeString, NSResponder, NSTextInputClient, NSView, NSWindow,
    NSWindowDelegate,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSKeyValueObservingOptions, NSNotFound, NSNotification,
    NSObjectNSKeyValueObserverRegistration, NSObjectProtocol, NSPoint, NSRange, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString,
};
use objc2_metal::{MTLCommandBuffer, MTLDrawable};
use objc2_quartz_core::{CADisplayLink, CALayer, CAMetalDrawable, CAMetalLayer};
use term_core::{Selection, Term};
use term_font::Fonts;
use term_pty::{Command, Size};
use term_render::{FrameInput, Renderer, Theme};

use crate::config::Config;
use crate::control::{self, Control, Dump};
use crate::frame_policy::{self, Decision};
use crate::keys::{KeyAction, KeyInput, KeyModes, Mods, encode_key, paste_bytes};
use crate::latency::{self, FrameKeys, Latency};
use crate::layout::{self, CellSize, Padding};
use crate::mouse;
use crate::select::{self, Block, Gesture};
use crate::session::{Events, Hooks, Outcome, Session};

const BLINK_INTERVAL: Duration = Duration::from_millis(530);
/// How long a tick waits for the echo of input before drawing without it.
const ECHO_WAIT: Duration = Duration::from_millis(8);

struct State {
    session: Session,
    renderer: Renderer,
    link: Retained<CADisplayLink>,
    config: Rc<Config>,
    theme: Theme,
    scale: f64,
    font_size: f64,
    focused: bool,
    cursor_on: bool,
    blink_scheduled: bool,
    idle_ticks: u8,
    /// Frames presented; lets a delayed buffer trim see if anything was drawn since.
    frames: u64,
    /// Display refresh rate currently asked for (Hz).
    rate: u16,
    /// Last key, paste, scroll or click: typing gets the full refresh rate.
    last_input: Instant,
    sync_since: Option<Instant>,
    marked: String,
    scroll_accum: f64,
    /// Mouse button held while reporting to the program.
    mouse_button: Option<u8>,
    /// The gesture that started the selection while the left button is
    /// held for it.
    selecting: Option<Gesture>,
    /// Whether the selection being made changed since the button went down.
    dragged: bool,
    /// A rectangular selection (Option-drag); see [`crate::select`].
    block: Option<Block>,
    /// Scrolling while a selection is dragged past the top or bottom edge.
    autoscroll: Option<AutoScroll>,
    /// An auto-scroll step is waiting to run.
    autoscroll_scheduled: bool,
    /// The session has ended, but the window stays open to show why; a key
    /// closes it.
    held: bool,
    /// Only with `DECLINE_TEST_DUMP` set: see [`crate::control`].
    test: Option<TestControl>,
    /// Only with `DECLINE_LATENCY_LOG` set: the key event being handled,
    /// its timestamp and when `keyDown` got it (see [`crate::latency`]).
    key_t0: Option<(f64, f64)>,
    /// When input was last sent and its echo not yet drawn: the first
    /// output after it is drawn at once, not on the next tick.
    awaiting_echo: Option<Instant>,
    /// The title bar's height, plus the tab bar's while one shows (points),
    /// as of the last resize.
    top_inset: f64,
    /// Times the window was hidden or covered and gave back its drawables.
    hidden: u64,
}

struct AutoScroll {
    /// Lines a second; positive scrolls back into history.
    speed: f64,
    /// The column under the mouse, for extending the selection.
    col: usize,
    last: Instant,
    /// Part of a line not yet scrolled.
    carry: f64,
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
    /// The window whose `contentLayoutRect` this view observes (it changes
    /// when the tab bar shows or hides), until the window closes.
    observed: RefCell<Option<Retained<NSWindow>>>,
}

/// The key path observed for the tab bar showing or hiding.
const LAYOUT_RECT: &str = "contentLayoutRect";

thread_local! {
    static VIEWS: RefCell<HashMap<usize, Weak<TerminalView>>> = RefCell::new(HashMap::new());
    static NEXT_ID: StdCell<usize> = const { StdCell::new(1) };
}

fn with_view(id: usize, f: impl FnOnce(&TerminalView)) {
    let view = VIEWS.with(|v| v.borrow().get(&id).and_then(|w| w.load()));
    if let Some(view) = view {
        f(&view);
    }
}

/// Bring view `id`'s window or tab to the front (a click on its
/// notification).
pub(crate) fn bring_to_front(id: usize) {
    with_view(id, |v| {
        if let Some(window) = v.window() {
            window.makeKeyAndOrderFront(None);
            objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(v)).activate();
        }
    });
}

/// Wake the main thread to draw (called from any thread).
fn wake(id: usize) {
    DispatchQueue::main().exec_async(move || with_view(id, |v| v.output_arrived()));
}

define_class!(
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeclineTerminalView"]
    #[ivars = Ivars]
    pub struct TerminalView;

    impl TerminalView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

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
            self.request_redraw();
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
            if self.with_state(|st| st.top_inset != inset) == Some(true) {
                self.resize();
            }
        }

        #[unsafe(method(tick:))]
        fn tick(&self, _link: &CADisplayLink) {
            self.draw();
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            self.handle_key(event);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.mouse_button_event(event, 0, true);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.mouse_button_event(event, 0, false);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.mouse_motion(event, Some(0));
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.mouse_button_event(event, 2, true);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.mouse_button_event(event, 2, false);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            self.mouse_button_event(event, 1, true);
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            self.mouse_button_event(event, 1, false);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.mouse_motion(event, None);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            self.handle_scroll(event);
        }

        #[unsafe(method(copy:))]
        fn copy(&self, _sender: Option<&AnyObject>) {
            self.copy_selection();
        }

        #[unsafe(method(paste:))]
        fn paste(&self, _sender: Option<&AnyObject>) {
            let pb = NSPasteboard::generalPasteboard();
            let Some(text) = pb.stringForType(unsafe { NSPasteboardTypeString }) else { return };
            let bracketed = self.with_state(|st| st.session.lock_term().state.modes().bracketed_paste).unwrap_or(false);
            self.send_input(paste_bytes(&text.to_string(), bracketed));
        }

        #[unsafe(method(selectAll:))]
        fn select_all(&self, _sender: Option<&AnyObject>) {
            self.with_state(|st| st.session.lock_term().state.select_all());
            self.request_redraw();
        }

        #[unsafe(method(clearBuffer:))]
        fn clear_buffer(&self, _sender: Option<&AnyObject>) {
            self.with_state(|st| st.session.lock_term().advance(b"\x1b[H\x1b[2J\x1b[3J"));
            // Ask the shell to redraw its prompt.
            self.send_input(b"\x0c".to_vec());
        }

        #[unsafe(method(increaseFontSize:))]
        fn increase_font_size(&self, _sender: Option<&AnyObject>) {
            let size = self.with_state(|st| st.font_size + 1.0);
            self.reload_fonts(size);
        }

        #[unsafe(method(decreaseFontSize:))]
        fn decrease_font_size(&self, _sender: Option<&AnyObject>) {
            let size = self.with_state(|st| (st.font_size - 1.0).max(6.0));
            self.reload_fonts(size);
        }

        #[unsafe(method(resetFontSize:))]
        fn reset_font_size(&self, _sender: Option<&AnyObject>) {
            let size = self.with_state(|st| st.config.font_size);
            self.reload_fonts(size);
        }
    }

    unsafe impl NSObjectProtocol for TerminalView {}

    unsafe impl NSWindowDelegate for TerminalView {
        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _n: &NSNotification) {
            self.set_focus(true);
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _n: &NSNotification) {
            self.set_focus(false);
        }

        #[unsafe(method(windowDidChangeOcclusionState:))]
        fn window_did_change_occlusion(&self, _n: &NSNotification) {
            if self.visible() {
                self.request_redraw();
            } else {
                // Free what can be rebuilt while nothing can be seen. A
                // background tab is a hidden window, so it lands here too.
                let minimised = self.window().is_some_and(|w| w.isMiniaturized());
                self.with_state(|st| {
                    st.hidden += 1;
                    st.renderer.hide(minimised)
                });
                self.trim_drawables();
            }
        }

        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _n: &NSNotification) {
            self.shut_down();
        }
    }

    unsafe impl NSTextInputClient for TerminalView {
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, string: &AnyObject, _range: NSRange) {
            let text = string_of(string);
            self.with_state(|st| st.marked.clear());
            if text.is_empty() {
                self.request_redraw();
            } else {
                // Drawn with the echo (see `send_input`).
                self.send_input(text.into_bytes());
            }
        }

        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, _selector: Sel) {
            // Keys with commands (arrows, return, …) are encoded in keyDown.
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(&self, string: &AnyObject, _selected: NSRange, _replacement: NSRange) {
            let text = string_of(string);
            self.with_state(|st| st.marked = text);
            self.request_redraw();
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            self.with_state(|st| st.marked.clear());
            self.request_redraw();
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            NSRange::new(NSNotFound as usize, 0)
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            let len = self.with_state(|st| st.marked.encode_utf16().count()).unwrap_or(0);
            if len == 0 { NSRange::new(NSNotFound as usize, 0) } else { NSRange::new(0, len) }
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            self.with_state(|st| !st.marked.is_empty()).unwrap_or(false)
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        fn attributed_substring(&self, _range: NSRange, _actual: *mut NSRange) -> Option<Retained<NSAttributedString>> {
            None
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn first_rect(&self, _range: NSRange, _actual: *mut NSRange) -> NSRect {
            // Where the input method should put its candidate window: the cursor.
            let Some(window) = self.window() else { return NSRect::ZERO };
            let rect = self.with_state(|st| {
                let m = st.renderer.metrics();
                let pad = self.padding(st);
                let t = st.session.lock_term();
                let (x, y) = layout::cell_origin(t.state.cursor().row(), t.state.cursor().col(), m.into(), pad);
                let (x, y) = (x as f64 / st.scale, y as f64 / st.scale);
                NSRect::new(NSPoint::new(x, y), NSSize::new(m.cell_width as f64 / st.scale, m.cell_height as f64 / st.scale))
            });
            let Some(rect) = rect else { return NSRect::ZERO };
            let in_window = self.convertRect_toView(rect, None);
            window.convertRectToScreen(in_window)
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index(&self, _point: NSPoint) -> usize {
            NSNotFound as usize
        }
    }
);

fn string_of(obj: &AnyObject) -> String {
    if let Some(s) = obj.downcast_ref::<NSAttributedString>() {
        return s.string().to_string();
    }
    obj.downcast_ref::<NSString>().map(|s| s.to_string()).unwrap_or_default()
}

pub struct Launch {
    pub command: Command,
    pub fonts: Fonts,
    pub scale: f64,
    /// Shown at the top of the window, before the program's output.
    pub notices: Rc<[String]>,
}

impl TerminalView {
    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<TerminalView> {
        let id = NEXT_ID.with(|n| {
            let id = n.get();
            n.set(id + 1);
            id
        });
        let this = Self::alloc(mtm).set_ivars(Ivars {
            id,
            layer: CAMetalLayer::new(),
            state: RefCell::new(None),
            observed: RefCell::new(None),
        });
        let view: Retained<TerminalView> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setWantsLayer(true);
        VIEWS.with(|v| v.borrow_mut().insert(id, Weak::from_retained(&view)));
        view
    }

    /// Start the program and drawing. The view must be in its window.
    pub fn start(&self, config: Rc<Config>, launch: Launch) -> std::io::Result<()> {
        let id = self.ivars().id;
        let layer = &self.ivars().layer;
        let renderer = Renderer::new(launch.fonts);
        let opaque = config.opacity >= 1.0;
        term_render::configure_layer(layer, renderer.device(), opaque);
        layer.setContentsScale(launch.scale);

        let theme = config.theme();
        let m = renderer.metrics();
        let size = self.frame().size;
        let pad = padding_for(&config, self.titlebar_height(), launch.scale);
        let (cols, lines) = layout::grid_size((size.width, size.height), launch.scale, m.into(), pad);

        let mut t = Term::new(cols, lines, config.scrollback);
        t.state.set_default_colors(crate::config::query_colors(&theme));
        for n in launch.notices.iter() {
            crate::session::write_notice(&mut t, n);
        }
        let mut hooks = Hooks::new(
            Arc::new(move || wake(id)),
            Box::new(move |outcome| DispatchQueue::main().exec_async(move || with_view(id, |v| v.program_exited(outcome)))),
        );
        let test = std::env::var_os(control::ENV).filter(|p| !p.is_empty()).map(|p| TestControl {
            path: PathBuf::from(p),
            queue: VecDeque::new(),
            dump: DumpState::Idle,
            seq: 0,
        });
        if test.is_some() {
            hooks.on_control = Some(Box::new(move |cmd| {
                DispatchQueue::main().exec_async(move || with_view(id, |v| v.control(&cmd)));
            }));
        }
        if let Some(lat) = latency::get() {
            hooks.on_parsed = Some(Box::new(move || lat.parsed(id)));
        }
        hooks.on_events = Some(Box::new(move |events| {
            DispatchQueue::main().exec_async(move || with_view(id, |v| v.events(events)));
        }));
        let session = Session::spawn(&launch.command, pty_size(cols, lines, m), t, hooks)?;

        let link = unsafe { self.displayLinkWithTarget_selector(self, sel!(tick:)) };
        unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes) };
        link.setPaused(false);

        *self.ivars().state.borrow_mut() = Some(State {
            session,
            renderer,
            link,
            font_size: config.font_size,
            config,
            theme,
            scale: launch.scale,
            focused: true,
            cursor_on: true,
            blink_scheduled: false,
            idle_ticks: 0,
            frames: 0,
            rate: 0,
            last_input: Instant::now(),
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
            test,
            key_t0: None,
            awaiting_echo: None,
            top_inset: self.titlebar_height(),
            hidden: 0,
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

    /// What the program asked for, taken as it was parsed (see
    /// [`crate::session::Events`]): drawn or not, a tab's title is shown
    /// in the tab bar, a bell bounces the Dock icon, a notification is
    /// shown if this window or tab isn't the one in front, and progress
    /// goes on the Dock icon.
    fn events(&self, events: Events) {
        let mtm = MainThreadMarker::from(self);
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        let fixed_title = self.with_state(|st| st.config.title.is_some()).unwrap_or(true);
        let window = self.window();
        if let Some(window) = &window
            && !fixed_title
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
                    .unwrap_or_else(|| "decline".into());
                crate::notify::post(mtm, self.ivars().id, &title, &n.body);
            }
        }
        if let Some(p) = events.progress {
            crate::notify::set_progress(mtm, self.ivars().id, p);
        }
    }

    fn with_state<R>(&self, f: impl FnOnce(&mut State) -> R) -> Option<R> {
        let mut st = self.ivars().state.borrow_mut();
        st.as_mut().map(f)
    }

    pub fn foreground_cwd(&self) -> Option<String> {
        self.with_state(|st| {
            st.session
                .lock_term()
                .state
                .cwd()
                .map(str::to_owned)
                .or_else(|| st.session.foreground_cwd())
        })
        .flatten()
    }

    /// The title bar's height, plus the tab bar's while one shows.
    fn titlebar_height(&self) -> f64 {
        match self.window() {
            Some(w) => layout::top_inset(w.frame().size.height, w.contentLayoutRect().size.height),
            None => 0.0,
        }
    }

    fn padding(&self, st: &State) -> Padding {
        padding_for(&st.config, self.titlebar_height(), st.scale)
    }

    fn unpause(&self) {
        let was_paused = self.with_state(|st| {
            st.idle_ticks = 0;
            let paused = st.link.isPaused();
            st.link.setPaused(false);
            paused
        });
        if was_paused == Some(true) {
            // Nothing drawn recently: draw now instead of waiting a refresh.
            self.draw();
        }
    }

    /// The program wrote something (the dirty flag was clear). If it may be
    /// the echo of input, draw now rather than on the next tick.
    fn output_arrived(&self) {
        let echo = self.with_state(|st| st.awaiting_echo.take().is_some());
        if echo == Some(true) {
            self.with_state(|st| {
                st.idle_ticks = 0;
                st.link.setPaused(false);
            });
            self.draw();
        } else {
            self.unpause();
        }
    }

    fn request_redraw(&self) {
        self.with_state(|st| st.session.mark_dirty());
        self.unpause();
    }

    fn visible(&self) -> bool {
        self.window()
            .is_some_and(|w| w.occlusionState().contains(objc2_app_kit::NSWindowOcclusionState::Visible))
    }

    fn draw(&self) {
        let titlebar = self.titlebar_height();
        let visible = self.visible();
        let minimised = self.window().is_some_and(|w| w.isMiniaturized());
        let mut guard = self.ivars().state.borrow_mut();
        let Some(st) = guard.as_mut() else { return };
        // Under test control, draw even when covered by other windows, so a
        // dump never waits forever for a frame.
        let visible = visible || st.test.is_some();
        let term = st.session.term().clone();
        let mut dirty = st.session.take_dirty();
        if let Some(sent) = st.awaiting_echo {
            if sent.elapsed() >= ECHO_WAIT {
                // No echo: draw what the input changed (cursor, scroll).
                st.awaiting_echo = None;
                dirty = true;
            } else if !dirty {
                // Keep ticking until the echo comes or the wait is over.
                st.idle_ticks = 0;
                return;
            }
        }
        // Only a frame that may be drawn needs the terminal's lock.
        let mut t = (dirty && visible).then(|| crate::session::lock(&term));
        let synchronized = t.as_ref().is_some_and(|t| t.state.modes().synchronized);
        let sync_elapsed = if synchronized {
            st.sync_since.get_or_insert_with(Instant::now).elapsed()
        } else {
            if t.is_some() {
                st.sync_since = None;
            }
            Duration::ZERO
        };
        let decision = frame_policy::decide(
            dirty,
            visible,
            synchronized,
            sync_elapsed,
            st.idle_ticks,
            minimised,
            st.last_input.elapsed(),
            st.focused,
        );
        let rate = match decision {
            Decision::Skip => {
                st.idle_ticks = st.idle_ticks.saturating_add(1);
                return;
            }
            Decision::Pause { trim_after } => {
                st.idle_ticks = st.idle_ticks.saturating_add(1);
                st.link.setPaused(true);
                let token = st.frames;
                drop(t);
                drop(guard);
                self.schedule_trim(token, trim_after);
                return;
            }
            Decision::Hide { keep_surface } => {
                // Hidden or fully covered: keep the change and draw when shown.
                st.idle_ticks = 0;
                st.session.mark_dirty();
                st.link.setPaused(true);
                st.renderer.hide(keep_surface);
                return;
            }
            Decision::Hold => {
                // Keep ticking so the timeout is noticed.
                st.idle_ticks = 0;
                st.session.mark_dirty();
                st.link.setPaused(false);
                return;
            }
            Decision::Draw { rate } => latency::get().and_then(|l| l.hz()).unwrap_or(rate),
        };
        st.idle_ticks = 0;
        st.renderer.show();
        if rate != st.rate {
            st.rate = rate;
            let r = rate as f32;
            st.link.setPreferredFrameRateRange(objc2_quartz_core::CAFrameRateRange {
                minimum: r / 2.0,
                maximum: r,
                preferred: r,
            });
        }
        let pad = padding_for(&st.config, titlebar, st.scale);
        let blink;
        let clipboard;
        {
            let Some(t) = t.as_mut() else { return };
            clipboard = t.state.take_clipboard();
            blink = t.state.cursor_blink();
            let input = FrameInput {
                term: &t.state,
                theme: &st.theme,
                metrics: st.renderer.metrics(),
                padding: (pad.x, pad.top),
                cursor_on: st.cursor_on,
                focused: st.focused,
                preedit: Some(st.marked.as_str()),
            };
            st.renderer.prepare(&input);
        }
        // With the terminal still locked: the keys whose output this frame shows.
        let timed = latency::get().map(|l| (l, l.take_for_frame(self.ivars().id, rate)));
        drop(t);
        match timed {
            Some((lat, keys)) if !keys.is_empty() => present_timed(&mut st.renderer, &self.ivars().layer, lat, keys),
            _ => st.renderer.present(&self.ivars().layer),
        }
        st.frames += 1;
        let mut poll_dump = false;
        if let Some(test) = &mut st.test
            && test.dump == DumpState::Requested
        {
            test.dump = DumpState::Drawn(st.renderer.frames_submitted());
            poll_dump = true;
        }

        let need_blink = blink && st.focused && !st.blink_scheduled;
        if need_blink {
            st.blink_scheduled = true;
        }
        drop(guard);
        if need_blink {
            self.schedule_blink();
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
                let idle = v.with_state(|st| st.frames == token && st.link.isPaused() && !st.blink_scheduled);
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

    fn schedule_blink(&self) {
        let id = self.ivars().id;
        let _ = DispatchQueue::main().after(DispatchTime::NOW.time(BLINK_INTERVAL.as_nanos() as i64), move || {
            with_view(id, |v| {
                let again = v.with_state(|st| {
                    st.blink_scheduled = false;
                    let blinking = st.focused && st.session.lock_term().state.cursor_blink();
                    st.cursor_on = !st.cursor_on || !blinking;
                    blinking
                });
                if again == Some(true) {
                    v.request_redraw();
                }
            })
        });
    }

    fn resize(&self) {
        let Some(window) = self.window() else { return };
        let titlebar = self.titlebar_height();
        self.with_state(|st| {
            st.scale = window.backingScaleFactor();
            st.top_inset = titlebar;
            let size = self.frame().size;
            let layer = &self.ivars().layer;
            layer.setContentsScale(st.scale);
            layer.setDrawableSize(NSSize::new((size.width * st.scale).round(), (size.height * st.scale).round()));
            let m = st.renderer.metrics();
            let pad = padding_for(&st.config, titlebar, st.scale);
            let (cols, lines) = layout::grid_size((size.width, size.height), st.scale, m.into(), pad);
            let _ = st.session.resize(pty_size(cols, lines, m));
        });
        // Draw now so live resizing doesn't show a stretched old frame.
        self.draw();
    }

    fn reload_fonts(&self, size: Option<f64>) {
        let Some(window) = self.window() else { return };
        let scale = window.backingScaleFactor();
        let changed = self.with_state(|st| {
            let size = size.unwrap_or(st.font_size);
            if size == st.font_size && scale == st.scale {
                return false;
            }
            st.font_size = size;
            st.scale = scale;
            // Only the size changes: fallbacks, faces, line height and
            // cell width stay as configured.
            st.renderer.set_fonts(Fonts::with_config(&st.config.font_config(size), scale));
            true
        });
        if changed == Some(true) {
            self.resize();
        }
    }

    fn set_focus(&self, focused: bool) {
        let report = self.with_state(|st| {
            st.focused = focused;
            st.cursor_on = true;
            st.session.lock_term().state.modes().focus_events
        });
        if report == Some(true) {
            self.send_raw(if focused { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
        }
        self.request_redraw();
    }

    fn shut_down(&self) {
        let observed = self.ivars().observed.borrow_mut().take();
        if let Some(window) = observed {
            unsafe { window.removeObserver_forKeyPath(self, &NSString::from_str(LAYOUT_RECT)) };
        }
        let st = self.ivars().state.borrow_mut().take();
        if let Some(st) = st {
            st.link.invalidate();
            drop(st); // hangs up on the program
            crate::notify::set_progress(MainThreadMarker::from(self), self.ivars().id, term_core::Progress::Clear);
        }
        VIEWS.with(|v| v.borrow_mut().remove(&self.ivars().id));
    }

    fn program_exited(&self, outcome: Outcome) {
        match outcome {
            Outcome::Closed => self.close_window(),
            Outcome::Hold => {
                // The session wrote why into the terminal.
                let testing = self.with_state(|st| {
                    st.held = true;
                    st.session.mark_dirty();
                    st.test.is_some()
                });
                self.request_redraw();
                if testing == Some(true) {
                    // Smoke tests can't send a dump command once the
                    // program is gone: dump what the held window shows.
                    self.control("dump");
                }
            }
        }
    }

    fn close_window(&self) {
        if let Some(window) = self.window() {
            window.close();
        }
    }

    /// Input typed by the user: snaps the view back to the live screen.
    fn send_input(&self, bytes: Vec<u8>) {
        let len = bytes.len();
        let key_t0 = self.with_state(|st| {
            st.cursor_on = true;
            st.last_input = Instant::now();
            let mut t = st.session.lock_term();
            t.state.scroll_view_to_bottom();
            t.state.set_selection(None);
            drop(t);
            st.key_t0.take()
        });
        // Logged before the writer has the bytes, so the echo can't be
        // parsed before the key is waited for.
        if let (Some(lat), Some(Some((t0, key_down)))) = (latency::get(), key_t0) {
            lat.key_sent(self.ivars().id, t0, key_down, len);
        }
        self.send_raw(bytes);
        // Don't draw yet: the echo is usually well under a millisecond away.
        // A frame now would take the free drawable and clear the dirty flag,
        // so the echo would wait for the next tick. Instead the first output
        // after this is drawn as soon as it's parsed (`output_arrived`);
        // with none, a tick draws after `ECHO_WAIT`.
        self.with_state(|st| {
            st.awaiting_echo = Some(Instant::now());
            st.idle_ticks = 0;
            st.link.setPaused(false);
        });
    }

    /// Bytes typed for the latency harness (its `type` command).
    pub fn latency_type(&self, bytes: Vec<u8>) {
        let t = latency::now();
        self.with_state(|st| st.key_t0 = Some((t, t)));
        self.send_input(bytes);
        self.with_state(|st| st.key_t0 = None);
    }

    fn send_raw(&self, bytes: Vec<u8>) {
        self.with_state(|st| st.session.send(bytes));
    }

    fn handle_key(&self, event: &NSEvent) {
        if latency::get().is_some() {
            let times = (event.timestamp(), latency::now());
            self.with_state(|st| st.key_t0 = Some(times));
        }
        self.handle_key_event(event);
        self.with_state(|st| st.key_t0 = None);
    }

    fn handle_key_event(&self, event: &NSEvent) {
        let input = key_input(event);
        // A held window closes on any key; Cmd shortcuts (copy the error,
        // close, quit) still go to the menus.
        if input.flags & crate::keys::flags::COMMAND == 0 && self.with_state(|st| st.held) == Some(true) {
            self.close_window();
            return;
        }
        let modes = self.with_state(|st| {
            let t = st.session.lock_term();
            KeyModes {
                app_cursor: t.state.modes().app_cursor,
                app_keypad: t.state.modes().app_keypad,
                kitty_flags: t.state.kitty_keyboard_flags(),
                option_as_meta: st.config.option_as_meta,
                composing: !st.marked.is_empty(),
            }
        });
        let Some(modes) = modes else { return };
        let action = crate::keys::translate(&input, modes);
        if input.flags & crate::keys::flags::COMMAND == 0 {
            NSCursor::setHiddenUntilMouseMoves(true);
        }
        match action {
            KeyAction::Send(bytes) => self.send_input(bytes),
            KeyAction::ToMenu => {}
            KeyAction::ToTextInput => {
                // Text, dead keys and input methods.
                let events = NSArray::from_slice(&[event]);
                self.interpretKeyEvents(&events);
            }
        }
    }

    /// Visible line and column under the mouse, clamped to the grid.
    fn cell_at(&self, event: &NSEvent) -> Option<(usize, usize)> {
        let p = self.convertPoint_fromView(event.locationInWindow(), None);
        let titlebar = self.titlebar_height();
        self.with_state(|st| {
            let m = st.renderer.metrics();
            let pad = padding_for(&st.config, titlebar, st.scale);
            let t = st.session.lock_term();
            layout::cell_at((p.x, p.y), st.scale, m.into(), pad, t.state.cols(), t.state.lines())
        })
    }

    fn mods_of(event: &NSEvent) -> u8 {
        let f = event.modifierFlags();
        mouse::modifier_bits(
            f.contains(NSEventModifierFlags::Shift),
            f.contains(NSEventModifierFlags::Option),
            f.contains(NSEventModifierFlags::Control),
        )
    }

    /// Whether the program wants this mouse event (Shift forces selection).
    fn reporting(&self, event: &NSEvent) -> Option<(term_core::MouseMode, bool)> {
        let shift = event.modifierFlags().contains(NSEventModifierFlags::Shift);
        let (mode, sgr) = self.with_state(|st| {
            let t = st.session.lock_term();
            (t.state.modes().mouse, t.state.modes().mouse_sgr)
        })?;
        mouse::reports(mode, shift).then_some((mode, sgr))
    }

    fn send_mouse(&self, code: u8, line: usize, col: usize, press: bool, sgr: bool) {
        if let Some(bytes) = mouse::encode(code, line, col, press, sgr) {
            self.send_raw(bytes);
        }
    }

    fn mouse_button_event(&self, event: &NSEvent, button: u8, press: bool) {
        self.with_state(|st| st.last_input = Instant::now());
        let Some((line, col)) = self.cell_at(event) else { return };
        // A selection being made ends on its own button's release, even
        // if Shift (which kept the program from the mouse) is let go first.
        if button == mouse::LEFT && !press && self.with_state(|st| st.selecting.is_some()) == Some(true) {
            self.end_selection();
            return;
        }
        if let Some((_, sgr)) = self.reporting(event) {
            self.with_state(|st| st.mouse_button = press.then_some(button));
            self.send_mouse(button + Self::mods_of(event), line, col, press, sgr);
            return;
        }
        if button != mouse::LEFT || !press {
            return;
        }
        let flags = event.modifierFlags();
        let gesture = select::gesture(
            event.clickCount(),
            flags.contains(NSEventModifierFlags::Shift),
            flags.contains(NSEventModifierFlags::Option),
        );
        self.with_state(|st| {
            st.selecting = Some(gesture);
            st.dragged = false;
            st.autoscroll = None;
            let block = st.block.take();
            let mut t = st.session.lock_term();
            let s = &mut t.state;
            let point = s.point_at(line, col);
            match gesture {
                Gesture::Start => {
                    // Remember the anchor in an empty selection; dragging extends it.
                    s.set_selection(Some(Selection::new(point, point)));
                    st.scroll_accum = 0.0;
                }
                Gesture::Block => {
                    let b = Block::at(point);
                    s.set_selection(Some(b.stream()));
                    st.block = Some(b);
                }
                Gesture::Word => s.select_word(point),
                Gesture::Line => s.select_line(point),
                Gesture::Extend => {
                    // A rectangle still shown grows as a rectangle.
                    match block.filter(|b| s.selection() == Some(b.stream())) {
                        Some(mut b) => {
                            b.head = point;
                            s.set_selection(Some(b.stream()));
                            st.block = Some(b);
                        }
                        None => s.extend_selection(point),
                    }
                }
            }
        });
        self.request_redraw();
    }

    /// The left button was let go after making a selection: stop
    /// auto-scrolling, and copy it if `copy_on_select` is on.
    fn end_selection(&self) {
        let copy = self.with_state(|st| {
            let gesture = st.selecting.take();
            st.autoscroll = None;
            st.config.copy_on_select && gesture.is_some_and(|g| select::copies_on_release(g, st.dragged))
        });
        if copy == Some(true) {
            self.copy_selection();
        }
    }

    /// Put the selection (or the rectangle being shown) on the clipboard.
    fn copy_selection(&self) {
        let text = self
            .with_state(|st| {
                let mut t = st.session.lock_term();
                match st.block.filter(|b| t.state.selection() == Some(b.stream())) {
                    Some(b) => Some(b.text(&mut t.state)),
                    None => t.state.selection_text(),
                }
            })
            .flatten();
        if let Some(text) = text.filter(|t| !t.is_empty()) {
            let pb = NSPasteboard::generalPasteboard();
            pb.clearContents();
            pb.setString_forType(&NSString::from_str(&text), unsafe { NSPasteboardTypeString });
        }
    }

    /// Move the selection being made to the cell at view `line`, `col`.
    /// Whether it changed.
    fn extend_selection_to(st: &mut State, line: usize, col: usize) -> bool {
        let mut t = st.session.lock_term();
        let before = t.state.selection();
        let Some(sel) = before else { return false };
        let to = t.state.point_at(line, col);
        match st.block.as_mut().filter(|b| sel == b.stream()) {
            Some(b) => {
                b.head = to;
                t.state.set_selection(Some(b.stream()));
            }
            None => t.state.extend_selection(to),
        }
        let changed = t.state.selection() != before;
        st.dragged |= changed;
        changed
    }

    /// Auto-scroll speed for a drag at `event`'s height (lines a second;
    /// positive scrolls back), 0 while it is over the rows.
    fn autoscroll_speed(&self, event: &NSEvent) -> f64 {
        let p = self.convertPoint_fromView(event.locationInWindow(), None);
        let titlebar = self.titlebar_height();
        self.with_state(|st| {
            let m = st.renderer.metrics();
            let pad = padding_for(&st.config, titlebar, st.scale);
            let lines = st.session.lock_term().state.lines();
            let top = pad.top as f64;
            let bottom = top + (lines as u32 * m.cell_height) as f64;
            select::autoscroll_speed(p.y * st.scale, top, bottom, m.cell_height as f64)
        })
        .unwrap_or(0.0)
    }

    /// One auto-scroll step, then the next one while the mouse is still
    /// held past the edge. Nothing is scheduled once it is back over the
    /// rows or the button is let go.
    fn autoscroll_tick(&self) {
        let step = self.with_state(|st| {
            st.autoscroll_scheduled = false;
            if st.selecting.is_none() {
                st.autoscroll = None;
            }
            let a = st.autoscroll.as_mut()?;
            let now = Instant::now();
            let lines = select::autoscroll_step(a.speed, (now - a.last).as_secs_f64(), &mut a.carry);
            a.last = now;
            let (up, col) = (a.speed > 0.0, a.col);
            let mut changed = false;
            if lines != 0 {
                let mut t = st.session.lock_term();
                let before = t.state.view_top();
                t.state.scroll_view(lines);
                changed = t.state.view_top() != before;
                let row = if up { 0 } else { t.state.lines() - 1 };
                drop(t);
                changed |= Self::extend_selection_to(st, row, col);
            }
            st.autoscroll_scheduled = true;
            Some(changed)
        });
        let Some(Some(changed)) = step else { return };
        if changed {
            self.request_redraw();
        }
        self.schedule_autoscroll();
    }

    fn schedule_autoscroll(&self) {
        let id = self.ivars().id;
        let at = DispatchTime::NOW.time(select::INTERVAL.as_nanos() as i64);
        let _ = DispatchQueue::main().after(at, move || with_view(id, |v| v.autoscroll_tick()));
    }

    fn mouse_motion(&self, event: &NSEvent, button: Option<u8>) {
        let Some((line, col)) = self.cell_at(event) else { return };
        let selecting = self.with_state(|st| st.selecting.is_some()) == Some(true);
        if !selecting && let Some((mode, sgr)) = self.reporting(event) {
            let held = self.with_state(|st| st.mouse_button).flatten();
            if mouse::reports_motion(mode, held) {
                self.send_mouse(mouse::motion_code(held, button, Self::mods_of(event)), line, col, true, sgr);
            }
            return;
        }
        if button != Some(mouse::LEFT) || !selecting {
            return;
        }
        let speed = self.autoscroll_speed(event);
        let (changed, start_timer) = self
            .with_state(|st| {
                // Past the top or bottom edge, a timer keeps scrolling
                // while the mouse is held still; it stops once back over
                // the rows or the button is let go.
                st.autoscroll = (speed != 0.0).then(|| {
                    let (last, carry) = st.autoscroll.as_ref().map_or((Instant::now(), 0.0), |a| (a.last, a.carry));
                    AutoScroll { speed, col, last, carry }
                });
                let start = st.autoscroll.is_some() && !st.autoscroll_scheduled;
                st.autoscroll_scheduled |= start;
                (Self::extend_selection_to(st, line, col), start)
            })
            .unwrap_or((false, false));
        if changed {
            self.request_redraw();
        }
        if start_timer {
            self.schedule_autoscroll();
        }
    }

    fn handle_scroll(&self, event: &NSEvent) {
        self.with_state(|st| st.last_input = Instant::now());
        let precise = event.hasPreciseScrollingDeltas();
        let reporting = self.reporting(event);
        let Some((line, col)) = self.cell_at(event) else { return };
        let lines = self.with_state(|st| {
            let dy = event.scrollingDeltaY();
            let delta = if precise {
                dy * st.scale / st.renderer.metrics().cell_height as f64
            } else {
                dy
            };
            st.scroll_accum += delta;
            let whole = st.scroll_accum.trunc();
            st.scroll_accum -= whole;
            whole as isize
        });
        let Some(lines) = lines.filter(|&l| l != 0) else { return };
        if let Some((_, sgr)) = reporting {
            let code = mouse::wheel_code(lines, Self::mods_of(event));
            for _ in 0..lines.unsigned_abs().min(mouse::MAX_WHEEL_REPORTS) {
                self.send_mouse(code, line, col, true, sgr);
            }
            return;
        }
        let alt = self.with_state(|st| {
            let t = st.session.lock_term();
            t.state.alt_screen_active().then_some(t.state.modes().app_cursor)
        });
        if let Some(Some(app_cursor)) = alt {
            // Full-screen programs without mouse support get arrow keys.
            let key = if lines > 0 { crate::keys::Key::Up } else { crate::keys::Key::Down };
            let bytes = encode_key(key, Mods::default(), app_cursor);
            for _ in 0..lines.unsigned_abs() {
                self.send_raw(bytes.clone());
            }
            return;
        }
        self.with_state(|st| st.session.lock_term().state.scroll_view(lines));
        self.request_redraw();
    }
}

/// Present like [`Renderer::present`], with handlers that time the frame
/// for the keys it shows (latency harness only). The renderer doesn't see
/// this drawable, so a window hidden right after such a frame keeps its
/// previous surface instead of offering it for purging.
fn present_timed(renderer: &mut Renderer, layer: &CAMetalLayer, lat: &'static Latency, keys: Vec<latency::Key>) {
    let Some(drawable) = layer.nextDrawable() else {
        lat.write(&keys);
        return;
    };
    let cmd = renderer.encode(&drawable.texture());
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

fn padding_for(config: &Config, titlebar: f64, scale: f64) -> Padding {
    layout::padding(config.padding, titlebar, scale)
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

/// What `keys::translate` needs from a key event.
fn key_input(event: &NSEvent) -> KeyInput {
    let flags = event.modifierFlags();
    let plain = event.charactersIgnoringModifiers().map(|s| s.to_string()).unwrap_or_default();
    let shifted = if flags.contains(NSEventModifierFlags::Shift) {
        NSEventModifierFlags::Shift
    } else {
        NSEventModifierFlags::empty()
    };
    KeyInput {
        keycode: event.keyCode(),
        chars: event.characters().map(|s| s.to_string()).unwrap_or_default(),
        chars_shift_only: event
            .charactersByApplyingModifiers(shifted)
            .map(|s| s.to_string())
            .unwrap_or_else(|| plain.clone()),
        chars_ignoring_mods: plain,
        flags: flags.0,
        numpad: flags.contains(NSEventModifierFlags::NumericPad),
    }
}

pub fn delegate_of(view: &TerminalView) -> &ProtocolObject<dyn NSWindowDelegate> {
    ProtocolObject::from_ref(view)
}

/// Test-only control (`DECLINE_TEST_DUMP`); see [`crate::control`].
impl TerminalView {
    fn control(&self, cmd: &str) {
        let Some(c) = Control::parse(cmd) else {
            eprintln!("decline: unknown test control command {cmd:?}");
            return;
        };
        self.with_state(|st| {
            if let Some(test) = &mut st.test {
                test.queue.push_back(c);
            }
        });
        self.run_controls();
    }

    /// Run queued commands until one has to wait (a dump in progress).
    fn run_controls(&self) {
        loop {
            let next = self.with_state(|st| {
                let test = st.test.as_mut()?;
                if test.dump != DumpState::Idle {
                    return None;
                }
                test.queue.pop_front()
            });
            match next.flatten() {
                None => return,
                Some(Control::Dump) => {
                    self.with_state(|st| {
                        if let Some(test) = &mut st.test {
                            test.dump = DumpState::Requested;
                        }
                    });
                    self.request_redraw();
                }
                Some(Control::Resize { cols, lines }) => self.set_grid(cols as usize, lines as usize),
                Some(Control::Font(delta)) => {
                    let size = self.with_state(|st| (st.font_size + delta as f64).max(6.0));
                    self.reload_fonts(size);
                }
                Some(Control::Tab) => {
                    if let Some(window) = self.window() {
                        crate::app::open_tab_beside(&window);
                    }
                }
                Some(Control::Quit) => {
                    objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(self)).terminate(None);
                    return;
                }
            }
        }
    }

    /// Resize the window to show `cols` × `lines`.
    fn set_grid(&self, cols: usize, lines: usize) {
        let Some(window) = self.window() else { return };
        let titlebar = self.titlebar_height();
        let size = self.with_state(|st| {
            let pad = padding_for(&st.config, titlebar, st.scale);
            layout::content_size(cols, lines, st.scale, st.renderer.metrics().into(), pad)
        });
        if let Some((w, h)) = size {
            window.setContentSize(NSSize::new(w, h));
        }
    }

    /// Write the dump once the GPU has finished the frame drawn for it.
    fn poll_dump(&self) {
        let ready = self.with_state(|st| {
            let test = st.test.as_ref()?;
            let DumpState::Drawn(frame) = test.dump else { return None };
            Some(st.renderer.frames_completed() >= frame)
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
                    top_inset: self.titlebar_height(),
                    badge: objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(self))
                        .dockTile()
                        .badgeLabel()
                        .map(|b| b.to_string()),
                    active: objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(self)).isActive(),
                    secure_input: crate::app::secure_input_held(),
                };
                self.with_state(|st| write_dump(st, window));
                self.run_controls();
            }
        }
    }
}

/// What a dump says about the window, read from AppKit.
struct Window {
    id: usize,
    tabs: usize,
    tab: usize,
    visible: bool,
    top_inset: f64,
    badge: Option<String>,
    active: bool,
    secure_input: bool,
}

fn write_dump(st: &mut State, window: Window) {
    use std::io::Write;
    let pad_top = padding_for(&st.config, window.top_inset, st.scale).top;
    let Some(test) = &mut st.test else { return };
    test.seq += 1;
    test.dump = DumpState::Idle;
    let (screen, cols, lines) = {
        let t = st.session.lock_term();
        (t.state.dump(), t.state.cols(), t.state.lines())
    };
    let m = st.renderer.metrics();
    let (mask_texture, color_texture) = st.renderer.glyphs.texture_sizes();
    let dump = Dump {
        seq: test.seq,
        window: window.id,
        tabs: window.tabs,
        tab: window.tab,
        visible: window.visible,
        hidden: st.hidden,
        pad_top,
        notifications: crate::notify::stats(),
        badge: window.badge,
        active: window.active,
        secure_input: window.secure_input,
        screen,
        cols,
        lines,
        frames: st.renderer.frames_completed(),
        glyphs: st.renderer.frame.glyphs.len(),
        rects: st.renderer.frame.rects.len(),
        mask_texture,
        color_texture,
        cell_width: m.cell_width,
        cell_height: m.cell_height,
        font_size: st.font_size,
        scale: st.scale,
        phys_footprint: control::phys_footprint(),
    };
    let line = serde_json::to_string(&dump).expect("dump serialises");
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&test.path)
        .and_then(|mut f| writeln!(f, "{line}"));
    if let Err(e) = written {
        eprintln!("decline: writing {}: {e}", test.path.display());
    }
}
