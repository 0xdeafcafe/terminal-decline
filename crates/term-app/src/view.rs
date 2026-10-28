//! The terminal view: one per window. Owns the shell process, the terminal
//! state and the renderer, and turns keyboard/mouse input into bytes.

use std::cell::{Cell as StdCell, RefCell};
use std::collections::HashMap;

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dispatch2::{DispatchQueue, DispatchTime};
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSCursor, NSEvent, NSEventModifierFlags, NSPasteboard, NSPasteboardTypeString, NSResponder, NSTextInputClient, NSView, NSWindowDelegate,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSNotFound, NSNotification, NSObjectProtocol, NSPoint, NSRange, NSRect, NSRunLoop,
    NSRunLoopCommonModes, NSSize, NSString,
};
use objc2_quartz_core::{CADisplayLink, CALayer, CAMetalLayer};
use term_core::{MouseMode, Point, Selection, Term};
use term_font::Fonts;
use term_pty::{Command, Pty, Size};
use term_render::{FrameInput, Renderer, Theme};

use crate::config::{Config, OptionAsMeta};
use crate::keys::{Mods, ctrl_byte, encode_key, encode_kitty_char, encode_kitty_key, key_from_keycode, keypad_app_bytes, paste_bytes};

const BLINK_INTERVAL: Duration = Duration::from_millis(530);
/// Longest we hold drawing while a program is mid synchronized update.
const SYNC_TIMEOUT: Duration = Duration::from_millis(150);
/// How long after input the display runs at its full refresh rate.
const INPUT_BOOST: Duration = Duration::from_millis(250);
/// Idle time before the spare drawable is given back.
const IDLE_TRIM: Duration = Duration::from_secs(2);

/// Shared between the reader thread and the main thread.
struct Shared {
    /// Something changed since the last frame.
    dirty: AtomicBool,
}

struct State {
    term: Arc<Mutex<Term>>,
    shared: Arc<Shared>,
    input: Sender<Vec<u8>>,
    pty: Pty,
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
    selecting: bool,
}

pub struct Ivars {
    id: usize,
    layer: Retained<CAMetalLayer>,
    state: RefCell<Option<State>>,
}

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

/// Wake the main thread to draw (called from any thread).
fn wake(id: usize) {
    DispatchQueue::main().exec_async(move || with_view(id, |v| v.unpause()));
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
            let text = self.with_state(|st| st.term.lock().unwrap().state.selection_text()).flatten();
            if let Some(text) = text.filter(|t| !t.is_empty()) {
                let pb = NSPasteboard::generalPasteboard();
                pb.clearContents();
                pb.setString_forType(&NSString::from_str(&text), unsafe { NSPasteboardTypeString });
            }
        }

        #[unsafe(method(paste:))]
        fn paste(&self, _sender: Option<&AnyObject>) {
            let pb = NSPasteboard::generalPasteboard();
            let Some(text) = pb.stringForType(unsafe { NSPasteboardTypeString }) else { return };
            let bracketed = self.with_state(|st| st.term.lock().unwrap().state.modes.bracketed_paste).unwrap_or(false);
            self.send_input(paste_bytes(&text.to_string(), bracketed));
        }

        #[unsafe(method(selectAll:))]
        fn select_all(&self, _sender: Option<&AnyObject>) {
            self.with_state(|st| {
                let mut t = st.term.lock().unwrap();
                let s = &mut t.state;
                let first = s.visible_point(0, 0).line - (s.scrollback_len() - s.display_offset()) as u64;
                let last = s.visible_point(s.lines() - 1, s.cols() - 1);
                s.selection = Some(Selection { anchor: Point { line: first, col: 0 }, head: last });
            });
            self.request_redraw();
        }

        #[unsafe(method(clearBuffer:))]
        fn clear_buffer(&self, _sender: Option<&AnyObject>) {
            self.with_state(|st| st.term.lock().unwrap().advance(b"\x1b[H\x1b[2J\x1b[3J"));
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
                // Free what can be rebuilt while nothing can be seen.
                let minimised = self.window().is_some_and(|w| w.isMiniaturized());
                self.with_state(|st| st.renderer.hide(minimised));
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
            if !text.is_empty() {
                self.send_input(text.into_bytes());
            }
            self.request_redraw();
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
                let (px, py) = self.padding_px(st);
                let t = st.term.lock().unwrap();
                let x = (px + t.state.cursor.col as u32 * m.cell_width) as f64 / st.scale;
                let y = (py + t.state.cursor.row as u32 * m.cell_height) as f64 / st.scale;
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
        let (px, py, pb) = padding_for(&config, self.titlebar_height(), launch.scale);
        let (cols, lines) = grid_size(size, launch.scale, m, (px, py, pb));

        let mut t = Term::new(cols, lines, config.scrollback);
        t.state.default_colors =
            [theme.foreground, theme.background, theme.cursor].map(|c| (c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16) & 0xFF);
        let term = Arc::new(Mutex::new(t));
        let pty = Pty::spawn(
            &launch.command,
            Size {
                cols: cols as u16,
                lines: lines as u16,
                cell_width: m.cell_width as u16,
                cell_height: m.cell_height as u16,
            },
        )?;
        let shared = Arc::new(Shared {
            dirty: AtomicBool::new(true),
        });

        // Writer thread: input and query replies go out without ever blocking
        // the main thread on a full pipe.
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let mut writer = pty.writer()?;
        // Writes wait for room in the PTY on this thread, never the main one.
        std::thread::Builder::new().name("pty-writer".into()).spawn(move || {
            for bytes in rx {
                if writer.write_all(&bytes).is_err() {
                    break;
                }
            }
        })?;

        let mut reader = pty.reader()?;
        let (term2, shared2, tx2) = (term.clone(), shared.clone(), tx.clone());
        std::thread::Builder::new().name("pty-reader".into()).spawn(move || {
            // A macOS PTY hands out ~1 KB per read, so 64 KB is plenty.
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = reader.read_batch(&mut buf);
                if n == 0 {
                    break;
                }
                // Parse in slices, releasing the lock between them so a
                // flood of output never holds off drawing or typing for long.
                for chunk in buf[..n].chunks(64 * 1024) {
                    let replies = {
                        let mut t = term2.lock().unwrap();
                        t.advance(chunk);
                        std::mem::take(&mut t.state.replies)
                    };
                    if !replies.is_empty() {
                        let _ = tx2.send(replies);
                    }
                }
                if !shared2.dirty.swap(true, Ordering::AcqRel) {
                    wake(id);
                }
            }
            DispatchQueue::main().exec_async(move || with_view(id, |v| v.program_exited()));
        })?;

        let link = unsafe { self.displayLinkWithTarget_selector(self, sel!(tick:)) };
        unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes) };
        link.setPaused(false);

        *self.ivars().state.borrow_mut() = Some(State {
            term,
            shared,
            input: tx,
            pty,
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
            selecting: false,
        });
        self.resize();
        Ok(())
    }

    fn with_state<R>(&self, f: impl FnOnce(&mut State) -> R) -> Option<R> {
        let mut st = self.ivars().state.borrow_mut();
        st.as_mut().map(f)
    }

    pub fn foreground_cwd(&self) -> Option<String> {
        self.with_state(|st| st.term.lock().unwrap().state.cwd.clone().or_else(|| st.pty.foreground_cwd()))
            .flatten()
    }

    fn titlebar_height(&self) -> f64 {
        match self.window() {
            Some(w) => (w.frame().size.height - w.contentLayoutRect().size.height).max(0.0),
            None => 0.0,
        }
    }

    fn padding_px(&self, st: &State) -> (u32, u32) {
        let (px, py, _) = padding_for(&st.config, self.titlebar_height(), st.scale);
        (px, py)
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

    fn request_redraw(&self) {
        self.with_state(|st| st.shared.dirty.store(true, Ordering::Release));
        self.unpause();
    }

    fn visible(&self) -> bool {
        self.window()
            .is_some_and(|w| w.occlusionState().contains(objc2_app_kit::NSWindowOcclusionState::Visible))
    }

    fn draw(&self) {
        let mut guard = self.ivars().state.borrow_mut();
        let Some(st) = guard.as_mut() else { return };
        if !st.shared.dirty.swap(false, Ordering::AcqRel) {
            // Nothing new: stop the display link until something changes.
            st.idle_ticks += 1;
            if st.idle_ticks == 2 {
                st.link.setPaused(true);
                let token = st.frames;
                drop(guard);
                self.schedule_trim(token);
            }
            return;
        }
        st.idle_ticks = 0;
        if !self.visible() {
            // Hidden or fully covered: keep the change and draw when shown.
            st.shared.dirty.store(true, Ordering::Release);
            st.link.setPaused(true);
            let minimised = self.window().is_some_and(|w| w.isMiniaturized());
            st.renderer.hide(minimised);
            return;
        }
        st.renderer.show();
        // Full refresh rate right after input; 60 Hz while output streams
        // (no one reads text scrolling at 120 Hz, and the window server's
        // cost scales with frames); 30 Hz in a window that isn't focused.
        let rate = if st.last_input.elapsed() < INPUT_BOOST {
            120
        } else if !st.focused {
            30
        } else {
            60
        };
        if rate != st.rate {
            st.rate = rate;
            let r = rate as f32;
            st.link.setPreferredFrameRateRange(objc2_quartz_core::CAFrameRateRange {
                minimum: r / 2.0,
                maximum: r,
                preferred: r,
            });
        }
        let (px, py, _) = padding_for(&st.config, self.titlebar_height(), st.scale);
        let mut title = None;
        let blink;
        let bell;
        let clipboard;
        {
            let mut t = st.term.lock().unwrap();
            if t.state.modes.synchronized {
                let since = *st.sync_since.get_or_insert_with(Instant::now);
                if since.elapsed() < SYNC_TIMEOUT {
                    // Keep ticking so the timeout is noticed.
                    st.shared.dirty.store(true, Ordering::Release);
                    st.link.setPaused(false);
                    return;
                }
            } else {
                st.sync_since = None;
            }
            if t.state.title_changed {
                t.state.title_changed = false;
                title = t.state.title.clone();
            }
            bell = std::mem::take(&mut t.state.bell);
            clipboard = t.state.clipboard.take();
            blink = t.state.cursor_blink;
            let input = FrameInput {
                term: &t.state,
                theme: &st.theme,
                metrics: st.renderer.metrics(),
                padding: (px, py),
                cursor_on: st.cursor_on,
                focused: st.focused,
                preedit: Some(st.marked.as_str()),
            };
            st.renderer.prepare(&input);
        }
        st.renderer.present(&self.ivars().layer);
        st.frames += 1;

        let need_blink = blink && st.focused && !st.blink_scheduled;
        if need_blink {
            st.blink_scheduled = true;
        }
        let fixed_title = st.config.title.clone();
        drop(guard);
        if let Some(window) = self.window()
            && fixed_title.is_none()
            && let Some(title) = title
        {
            window.setTitle(&NSString::from_str(&title));
        }
        if need_blink {
            self.schedule_blink();
        }
        if let Some(text) = clipboard {
            let pb = NSPasteboard::generalPasteboard();
            pb.clearContents();
            pb.setString_forType(&NSString::from_str(&text), unsafe { NSPasteboardTypeString });
        }
        if bell {
            // Bounce the Dock icon if the app isn't in front.
            let app = objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(self));
            if !app.isActive() {
                app.requestUserAttention(objc2_app_kit::NSRequestUserAttentionType::InformationalRequest);
            }
        }
    }

    /// After a couple of seconds with nothing drawn, give back the spare
    /// drawable: resizing the layer's drawables to 1×1 and back releases the
    /// one not on screen (6.6 MB at 1624×1018) without changing what's shown.
    fn schedule_trim(&self, token: u64) {
        let id = self.ivars().id;
        let _ = DispatchQueue::main().after(DispatchTime::NOW.time(IDLE_TRIM.as_nanos() as i64), move || {
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
                    let blinking = st.focused && st.term.lock().unwrap().state.cursor_blink;
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
            let size = self.frame().size;
            let layer = &self.ivars().layer;
            layer.setContentsScale(st.scale);
            layer.setDrawableSize(NSSize::new((size.width * st.scale).round(), (size.height * st.scale).round()));
            let m = st.renderer.metrics();
            let (cols, lines) = grid_size(size, st.scale, m, padding_for(&st.config, titlebar, st.scale));
            {
                let mut t = st.term.lock().unwrap();
                t.state.resize(cols, lines);
                t.state.cell_pixels = (m.cell_width as u16, m.cell_height as u16);
            }
            let _ = st.pty.resize(Size {
                cols: cols as u16,
                lines: lines as u16,
                cell_width: m.cell_width as u16,
                cell_height: m.cell_height as u16,
            });
            st.shared.dirty.store(true, Ordering::Release);
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
            st.renderer.set_fonts(Fonts::new(&st.config.font_family, size * scale));
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
            st.term.lock().unwrap().state.modes.focus_events
        });
        if report == Some(true) {
            self.send_raw(if focused { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
        }
        self.request_redraw();
    }

    fn shut_down(&self) {
        let st = self.ivars().state.borrow_mut().take();
        if let Some(st) = st {
            st.link.invalidate();
            drop(st); // hangs up on the program
        }
        VIEWS.with(|v| v.borrow_mut().remove(&self.ivars().id));
    }

    fn program_exited(&self) {
        if let Some(window) = self.window() {
            window.close();
        }
    }

    /// Input typed by the user: snaps the view back to the live screen.
    fn send_input(&self, bytes: Vec<u8>) {
        self.with_state(|st| {
            st.cursor_on = true;
            st.last_input = Instant::now();
            let mut t = st.term.lock().unwrap();
            t.state.scroll_view_to_bottom();
            t.state.selection = None;
        });
        self.send_raw(bytes);
        self.request_redraw();
    }

    fn send_raw(&self, bytes: Vec<u8>) {
        self.with_state(|st| {
            let _ = st.input.send(bytes);
        });
    }

    fn handle_key(&self, event: &NSEvent) {
        let flags = event.modifierFlags();
        if flags.contains(NSEventModifierFlags::Command) {
            // Cmd shortcuts belong to the menus and are never sent to the
            // program, except the Mac line-editing ones.
            let bytes: Option<&[u8]> = match event.keyCode() {
                0x7B => Some(b"\x01"), // Cmd+Left: start of line
                0x7C => Some(b"\x05"), // Cmd+Right: end of line
                0x33 => Some(b"\x15"), // Cmd+Backspace: delete line
                _ => None,
            };
            if let Some(b) = bytes {
                self.send_input(b.to_vec());
            }
            return;
        }
        NSCursor::setHiddenUntilMouseMoves(true);
        let composing = self.with_state(|st| !st.marked.is_empty()).unwrap_or(false);
        if !composing && let Some(bytes) = self.key_bytes(event) {
            self.send_input(bytes);
            return;
        }
        // Text, dead keys and input methods.
        let events = NSArray::from_slice(&[event]);
        self.interpretKeyEvents(&events);
    }

    /// Bytes for keys that bypass text input: special keys, Ctrl combos and
    /// Option-as-Meta.
    fn key_bytes(&self, event: &NSEvent) -> Option<Vec<u8>> {
        let flags = event.modifierFlags();
        let (app_cursor, app_keypad, kitty, meta_side) = self.with_state(|st| {
            let t = st.term.lock().unwrap();
            (
                t.state.modes.app_cursor,
                t.state.modes.app_keypad,
                t.state.kitty_keyboard_flags(),
                st.config.option_as_meta,
            )
        })?;
        let raw = flags.0;
        // Device-dependent bits for which Option key is held.
        let (left_alt, right_alt) = (raw & 0x20 != 0, raw & 0x40 != 0);
        let meta = match meta_side {
            OptionAsMeta::Left => left_alt,
            OptionAsMeta::Right => right_alt,
            OptionAsMeta::Both => left_alt || right_alt,
            OptionAsMeta::None => false,
        };
        let mods = Mods {
            shift: flags.contains(NSEventModifierFlags::Shift),
            alt: flags.contains(NSEventModifierFlags::Option),
            ctrl: flags.contains(NSEventModifierFlags::Control),
        };
        let code = event.keyCode();
        let keypad = flags.contains(NSEventModifierFlags::NumericPad) && !(0x7B..=0x7E).contains(&code);
        if keypad
            && app_keypad
            && !mods.any()
            && let Some(b) = keypad_app_bytes(code)
        {
            return Some(b.to_vec());
        }
        if let Some(key) = key_from_keycode(code) {
            if kitty & 1 != 0
                && let Some(b) = encode_kitty_key(key, mods)
            {
                return Some(b);
            }
            return Some(encode_key(key, mods, app_cursor));
        }
        let plain = event.charactersIgnoringModifiers()?.to_string();
        let c = plain.chars().next()?;
        if kitty & 1 != 0 && (mods.ctrl || meta) && !c.is_control() {
            let mods = Mods { alt: meta, ..mods };
            return Some(encode_kitty_char(c, mods));
        }
        if mods.ctrl
            && let Some(b) = ctrl_byte(c)
        {
            let mut out = if meta { vec![0x1b] } else { Vec::new() };
            out.push(b);
            return Some(out);
        }
        if meta {
            // The character with Shift applied but not Option.
            let shifted = if mods.shift {
                NSEventModifierFlags::Shift
            } else {
                NSEventModifierFlags::empty()
            };
            let text = event.charactersByApplyingModifiers(shifted).map(|s| s.to_string()).unwrap_or(plain);
            let mut out = vec![0x1b];
            out.extend_from_slice(text.as_bytes());
            return Some(out);
        }
        None
    }

    /// Visible line and column under the mouse, clamped to the grid.
    fn cell_at(&self, event: &NSEvent) -> Option<(usize, usize)> {
        let p = self.convertPoint_fromView(event.locationInWindow(), None);
        let titlebar = self.titlebar_height();
        self.with_state(|st| {
            let m = st.renderer.metrics();
            let (px, py, _) = padding_for(&st.config, titlebar, st.scale);
            let t = st.term.lock().unwrap();
            let x = (p.x * st.scale - px as f64).max(0.0);
            let y = (p.y * st.scale - py as f64).max(0.0);
            let col = ((x / m.cell_width as f64) as usize).min(t.state.cols() - 1);
            let line = ((y / m.cell_height as f64) as usize).min(t.state.lines() - 1);
            (line, col)
        })
    }

    fn mods_of(event: &NSEvent) -> u8 {
        let f = event.modifierFlags();
        (f.contains(NSEventModifierFlags::Shift) as u8) * 4
            + (f.contains(NSEventModifierFlags::Option) as u8) * 8
            + (f.contains(NSEventModifierFlags::Control) as u8) * 16
    }

    /// Whether the program wants this mouse event (Shift forces selection).
    fn reporting(&self, event: &NSEvent) -> Option<(MouseMode, bool)> {
        if event.modifierFlags().contains(NSEventModifierFlags::Shift) {
            return None;
        }
        let (mode, sgr) = self.with_state(|st| {
            let t = st.term.lock().unwrap();
            (t.state.modes.mouse, t.state.modes.mouse_sgr)
        })?;
        (mode != MouseMode::Off).then_some((mode, sgr))
    }

    fn send_mouse(&self, code: u8, line: usize, col: usize, press: bool, sgr: bool) {
        let bytes = if sgr {
            format!("\x1b[<{code};{};{}{}", col + 1, line + 1, if press { 'M' } else { 'm' }).into_bytes()
        } else {
            if col > 222 || line > 222 {
                return;
            }
            let b = if press { code } else { 3 | (code & !3) };
            vec![0x1b, b'[', b'M', 32 + b, 33 + col as u8, 33 + line as u8]
        };
        self.send_raw(bytes);
    }

    fn mouse_button_event(&self, event: &NSEvent, button: u8, press: bool) {
        self.with_state(|st| st.last_input = Instant::now());
        let Some((line, col)) = self.cell_at(event) else { return };
        if let Some((_, sgr)) = self.reporting(event) {
            self.with_state(|st| st.mouse_button = press.then_some(button));
            self.send_mouse(button + Self::mods_of(event), line, col, press, sgr);
            return;
        }
        if button != 0 {
            return;
        }
        if !press {
            self.with_state(|st| st.selecting = false);
            return;
        }
        let clicks = event.clickCount();
        self.with_state(|st| {
            st.selecting = true;
            let mut t = st.term.lock().unwrap();
            let s = &mut t.state;
            let point = s.visible_point(line, col);
            s.selection = match clicks {
                1 => None,
                2 => {
                    let row = s.visible_row(line);
                    let is_word = |c: usize| {
                        let ch = row.get(c).ch();
                        !ch.is_whitespace() && !"()[]{}<>'\"`|,;".contains(ch)
                    };
                    if !is_word(col) {
                        Some(Selection {
                            anchor: point,
                            head: point,
                        })
                    } else {
                        let mut a = col;
                        while a > 0 && is_word(a - 1) {
                            a -= 1;
                        }
                        let mut b = col;
                        while b + 1 < s.cols() && is_word(b + 1) {
                            b += 1;
                        }
                        Some(Selection {
                            anchor: Point { col: a, ..point },
                            head: Point { col: b, ..point },
                        })
                    }
                }
                _ => Some(Selection {
                    anchor: Point { col: 0, ..point },
                    head: Point {
                        col: s.cols() - 1,
                        ..point
                    },
                }),
            };
            if clicks == 1 {
                // Remember the anchor in an empty selection; dragging extends it.
                s.selection = Some(Selection {
                    anchor: point,
                    head: point,
                });
                st.scroll_accum = 0.0;
            }
        });
        if clicks == 1 {
            // A plain click clears any visible selection until a drag starts.
            self.with_state(|st| {
                let mut t = st.term.lock().unwrap();
                if let Some(sel) = &mut t.state.selection {
                    sel.head = sel.anchor;
                }
            });
        }
        self.request_redraw();
    }

    fn mouse_motion(&self, event: &NSEvent, button: Option<u8>) {
        let Some((line, col)) = self.cell_at(event) else { return };
        if let Some((mode, sgr)) = self.reporting(event) {
            let held = self.with_state(|st| st.mouse_button).flatten();
            let send = match mode {
                MouseMode::Motion => true,
                MouseMode::Drag => held.is_some(),
                _ => false,
            };
            if send {
                let b = held.or(button).unwrap_or(3);
                self.send_mouse(32 + b + Self::mods_of(event), line, col, true, sgr);
            }
            return;
        }
        if button != Some(0) {
            return;
        }
        let changed = self.with_state(|st| {
            if !st.selecting {
                return false;
            }
            let mut t = st.term.lock().unwrap();
            let head = t.state.visible_point(line, col);
            match &mut t.state.selection {
                Some(sel) if sel.head != head => {
                    sel.head = head;
                    true
                }
                _ => false,
            }
        });
        if changed == Some(true) {
            self.request_redraw();
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
            let code = if lines > 0 { 64 } else { 65 } + Self::mods_of(event);
            for _ in 0..lines.unsigned_abs().min(10) {
                self.send_mouse(code, line, col, true, sgr);
            }
            return;
        }
        let alt = self.with_state(|st| {
            let t = st.term.lock().unwrap();
            t.state.alt_screen_active().then_some(t.state.modes.app_cursor)
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
        self.with_state(|st| st.term.lock().unwrap().state.scroll_view(lines));
        self.request_redraw();
    }
}

/// Padding in pixels: (left/right, top, bottom). The top includes the title
/// bar, which the content extends under.
fn padding_for(config: &Config, titlebar: f64, scale: f64) -> (u32, u32, u32) {
    let p = (config.padding * scale).round() as u32;
    (p, ((config.padding + titlebar) * scale).round() as u32, p)
}

fn grid_size(size: NSSize, scale: f64, m: term_font::Metrics, (px, py, pb): (u32, u32, u32)) -> (usize, usize) {
    let w = (size.width * scale) as u32;
    let h = (size.height * scale) as u32;
    // GPU positions are 16-bit; no display is near 32k pixels, but cap anyway.
    let (w, h) = (w.min(32_000), h.min(32_000));
    let cols = (w.saturating_sub(2 * px) / m.cell_width).clamp(2, 4000) as usize;
    let lines = (h.saturating_sub(py + pb) / m.cell_height).clamp(1, 2000) as usize;
    (cols, lines)
}

/// Window content size (points) for a grid.
pub fn content_size(config: &Config, m: term_font::Metrics, scale: f64, titlebar: f64) -> NSSize {
    let (px, py, pb) = padding_for(config, titlebar, scale);
    let w = config.columns as u32 * m.cell_width + 2 * px;
    let h = config.lines as u32 * m.cell_height + py + pb;
    NSSize::new(w as f64 / scale, h as f64 / scale)
}

pub fn delegate_of(view: &TerminalView) -> &ProtocolObject<dyn NSWindowDelegate> {
    ProtocolObject::from_ref(view)
}
