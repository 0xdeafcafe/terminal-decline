//! A pane's view: a plain `NSView` over its part of the window view, which
//! draws nothing (the window view draws every pane into its one layer).
//! It is there for what AppKit routes by view (splits design §1): it is
//! the first responder and the `NSTextInputClient`, and takes the mouse.
//! It turns those events into the inputs of the pure modules (`keys`,
//! `mouse`, `select`, `layout`) for its pane's session.

use std::cell::{Cell, RefCell};
use std::time::Instant;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSBeep, NSCursor, NSDragOperation, NSDraggingInfo, NSEvent, NSEventModifierFlags, NSEventType, NSMenu, NSMenuItem,
    NSPasteboard, NSPasteboardTypeFileURL, NSPasteboardTypeString, NSPasteboardTypeURL, NSResponder, NSTextInputClient, NSTrackingArea,
    NSTrackingAreaOptions, NSView, NSWorkspace,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSNotFound, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString, NSURL,
};
use term_core::Point;

use crate::find_bar::{Find, FindBar};
use crate::keybind;
use crate::keys::{KeyAction, KeyEventType, KeyInput, KeyModes, Mods, encode_key, paste_bytes};
use crate::latency;
use crate::layout;
use crate::mouse;
use crate::panes::PaneId;
use crate::search::Query;
use crate::select::{self, Gesture};
use crate::view::{AutoScroll, Pane, Win, WindowView, pane_padding, with_view};

pub struct Ivars {
    /// The window view this pane is laid over.
    window: Weak<WindowView>,
    pane: PaneId,
    /// The link the right-click menu was opened on, for Open Link and
    /// Copy Link.
    menu_link: RefCell<Option<String>>,
    /// Where the right-click menu was opened, for Select Command Output.
    menu_point: Cell<Option<Point>>,
    /// The pointer is the pointing hand, over a link with Cmd held.
    hand: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeclinePaneView"]
    #[ivars = Ivars]
    pub struct PaneView;

    impl PaneView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        /// The focused pane works as before: the click that activates the
        /// window only activates it. Another pane takes that click, and
        /// with it focus (which swallows it).
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            self.window_view().and_then(|w| w.focused_pane()).is_some_and(|f| f != self.pane())
        }

        /// AppKit makes the pane first responder itself: before a click
        /// reaches it, and when focus moves with the keys.
        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            let ok: bool = unsafe { msg_send![super(self), becomeFirstResponder] };
            if ok && let Some(w) = self.window_view() {
                w.pane_focused(self.pane(), self.clicked_now());
            }
            ok
        }

        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> bool {
            let ok: bool = unsafe { msg_send![super(self), resignFirstResponder] };
            if ok {
                self.lose_focus();
            }
            ok
        }

        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            self.handle_key(event);
        }

        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            self.handle_key_up(event);
        }

        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            self.handle_flags_changed(event);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if !self.swallowing(false) {
                self.mouse_button_event(event, 0, true);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            if !self.swallowing(true) {
                self.mouse_button_event(event, 0, false);
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            if !self.swallowing(false) {
                self.mouse_motion(event, Some(0));
            }
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

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.set_hover(None);
        }

        #[unsafe(method(openLink:))]
        fn open_link_item(&self, _sender: Option<&AnyObject>) {
            if let Some(uri) = self.ivars().menu_link.take() {
                self.open_link(&uri);
            }
        }

        #[unsafe(method(copyLink:))]
        fn copy_link_item(&self, _sender: Option<&AnyObject>) {
            if let Some(uri) = self.ivars().menu_link.take() {
                let pb = NSPasteboard::generalPasteboard();
                pb.clearContents();
                pb.setString_forType(&NSString::from_str(&uri), unsafe { NSPasteboardTypeString });
            }
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
            self.paste_text(&text.to_string());
        }

        /// Files, URLs or text dragged onto the pane are typed in: see
        /// `performDragOperation:`.
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, _info: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            NSDragOperation::Copy
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, _info: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            NSDragOperation::Copy
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, info: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            self.drop_from(&info.draggingPasteboard())
        }

        #[unsafe(method(selectAll:))]
        fn select_all(&self, _sender: Option<&AnyObject>) {
            self.clear_other_selections();
            self.with(|_, p| p.session.lock_term().state.select_all());
            self.request_redraw();
        }

        #[unsafe(method(jumpToPreviousPrompt:))]
        fn jump_to_previous_prompt(&self, _sender: Option<&AnyObject>) {
            self.jump_to_prompt(true);
        }

        #[unsafe(method(jumpToNextPrompt:))]
        fn jump_to_next_prompt(&self, _sender: Option<&AnyObject>) {
            self.jump_to_prompt(false);
        }

        #[unsafe(method(selectCommandOutput:))]
        fn select_command_output(&self, _sender: Option<&AnyObject>) {
            self.select_output_at(None);
        }

        /// The right-click menu's Select Command Output: the command
        /// clicked.
        #[unsafe(method(selectClickedOutput:))]
        fn select_clicked_output(&self, _sender: Option<&AnyObject>) {
            if let Some(at) = self.ivars().menu_point.take() {
                self.select_output_at(Some(at));
            }
        }

        /// Edit > Find > Find… (Cmd+F): show the find bar, or give its
        /// field the keys again.
        #[unsafe(method(showFindBar:))]
        fn show_find_bar(&self, _sender: Option<&AnyObject>) {
            self.open_find();
        }

        #[unsafe(method(findNextMatch:))]
        fn find_next_match(&self, _sender: Option<&AnyObject>) {
            self.find_step(true);
        }

        #[unsafe(method(findPreviousMatch:))]
        fn find_previous_match(&self, _sender: Option<&AnyObject>) {
            self.find_step(false);
        }

        /// Edit > Find > Use Selection for Find (Cmd+E): search for the
        /// selected text.
        #[unsafe(method(useSelectionForFind:))]
        fn use_selection_for_find(&self, _sender: Option<&AnyObject>) {
            self.use_selection();
        }

        /// The standard NSTextFinderAction tags, for menus that send them
        /// (cells design §13.9).
        #[unsafe(method(performTextFinderAction:))]
        fn perform_text_finder_action(&self, sender: Option<&AnyObject>) {
            let tag = sender
                .and_then(|s| {
                    let item = s.downcast_ref::<NSMenuItem>().map(|m| m.tag());
                    item.or_else(|| s.downcast_ref::<objc2_app_kit::NSControl>().map(|c| c.tag()))
                })
                .unwrap_or(1);
            match tag {
                1 => self.open_find(),
                2 => self.find_step(true),
                3 => self.find_step(false),
                7 => self.use_selection(),
                11 => self.close_find(),
                _ => NSBeep(),
            }
        }

        #[unsafe(method(clearBuffer:))]
        fn clear_buffer(&self, _sender: Option<&AnyObject>) {
            self.with(|_, p| p.session.lock_term().advance(b"\x1b[H\x1b[2J\x1b[3J"));
            // Ask the shell to redraw its prompt.
            self.send_input(b"\x0c".to_vec());
        }
    }

    unsafe impl NSObjectProtocol for PaneView {}

    unsafe impl NSTextInputClient for PaneView {
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, string: &AnyObject, _range: NSRange) {
            let text = string_of(string);
            // The key being handled, unless the text ends a composition.
            let key = self.with(|_, p| {
                let composed = !p.marked.is_empty();
                p.marked.clear();
                p.text_key.take().filter(|_| !composed)
            });
            if text.is_empty() {
                self.request_redraw();
            } else {
                let modes = match &key {
                    Some(Some((_, modes))) => *modes,
                    _ => self.key_modes().unwrap_or_default(),
                };
                let input = key.flatten().map(|(input, _)| input);
                // Drawn with the echo (see `send_input`).
                self.send_input(crate::keys::committed_text(input.as_ref(), modes, &text));
            }
        }

        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, _selector: Sel) {
            // Keys with commands (arrows, return, …) are encoded in keyDown.
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(&self, string: &AnyObject, _selected: NSRange, _replacement: NSRange) {
            let text = string_of(string);
            self.with(|_, p| p.marked = text);
            self.request_redraw();
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            self.with(|_, p| p.marked.clear());
            self.request_redraw();
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            NSRange::new(NSNotFound as usize, 0)
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            let len = self.with(|_, p| p.marked.encode_utf16().count()).unwrap_or(0);
            if len == 0 { NSRange::new(NSNotFound as usize, 0) } else { NSRange::new(0, len) }
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            self.with(|_, p| !p.marked.is_empty()).unwrap_or(false)
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
            let titlebar = self.window_view().map_or(0.0, |w| w.titlebar_height());
            let rect = self.with(|win, p| {
                let m = win.renderer.metrics();
                let pad = pane_padding(&win.config, titlebar, win.scale, p.rect);
                let t = p.session.lock_term();
                let (x, y) = layout::cell_origin(t.state.cursor().row(), t.state.cursor().col(), m.into(), pad);
                let (x, y) = (x as f64 / win.scale, y as f64 / win.scale);
                NSRect::new(NSPoint::new(x, y), NSSize::new(m.cell_width as f64 / win.scale, m.cell_height as f64 / win.scale))
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

/// The files and folders on `pb`, as paths (Finder's file reference URLs
/// resolved).
pub(crate) fn file_paths(pb: &NSPasteboard) -> Vec<String> {
    let Some(items) = pb.pasteboardItems() else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| item.stringForType(unsafe { NSPasteboardTypeFileURL }))
        .filter_map(|url| NSURL::URLWithString(&url))
        .filter(|url| url.isFileURL())
        .filter_map(|url| url.filePathURL().and_then(|u| u.path()))
        .map(|p| p.to_string())
        .collect()
}

impl PaneView {
    /// The view of pane `pane` over `window`, at `frame` in its
    /// coordinates. It gets no autoresizing mask: the window view lays its
    /// panes out.
    pub(crate) fn new(mtm: MainThreadMarker, window: &WindowView, pane: PaneId, frame: NSRect) -> Retained<PaneView> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            window: Weak::new(window),
            pane,
            menu_link: RefCell::new(None),
            menu_point: Cell::new(None),
            hand: Cell::new(false),
        });
        let view: Retained<PaneView> = unsafe { msg_send![super(this), initWithFrame: frame] };
        let types = unsafe { [NSPasteboardTypeFileURL, NSPasteboardTypeURL, NSPasteboardTypeString] };
        view.registerForDraggedTypes(&NSArray::from_slice(&types));
        // Mouse moves over this pane, for mode 1003 and links under Cmd
        // (the window's `acceptsMouseMovedEvents` is off), and leaving it,
        // which ends a link's hover. Its rect follows the view.
        let options = NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::ActiveInKeyWindow
            | NSTrackingAreaOptions::InVisibleRect;
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(NSTrackingArea::alloc(), NSRect::ZERO, options, Some(&view), None)
        };
        view.addTrackingArea(&area);
        view
    }

    /// Whether the event being handled is a left click in this view.
    fn clicked_now(&self) -> bool {
        let mtm = MainThreadMarker::from(self);
        let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else {
            return false;
        };
        if event.r#type() != NSEventType::LeftMouseDown {
            return false;
        }
        let ours = event
            .window(mtm)
            .zip(self.window())
            .is_some_and(|(e, w)| std::ptr::eq::<objc2_app_kit::NSWindow>(&*e, &*w));
        let at = self.convertPoint_fromView(event.locationInWindow(), None);
        let b = self.bounds();
        ours && at.x >= b.origin.x && at.y >= b.origin.y && at.x < b.origin.x + b.size.width && at.y < b.origin.y + b.size.height
    }

    /// Whether the left button's events belong to the click that focused
    /// this pane; `up`: its release, which ends that.
    fn swallowing(&self, up: bool) -> bool {
        self.with(|_, p| {
            let swallow = p.swallow_click;
            if up {
                p.swallow_click = false;
            }
            swallow
        })
        .unwrap_or(false)
    }

    /// Another pane (or nothing, as the window closes) took the keys:
    /// text being composed is committed, as NSTextView does, the program
    /// is told if it asked for focus events, and keys still down are
    /// released for a program that asked for releases (kitty flag 2).
    fn lose_focus(&self) {
        let Some(modes) = self.key_modes() else { return };
        if modes.composing {
            // The input method may commit it itself (through `insertText`).
            if let Some(ctx) = self.inputContext() {
                ctx.discardMarkedText();
            }
            let marked = self.with(|_, p| std::mem::take(&mut p.marked)).unwrap_or_default();
            if !marked.is_empty() {
                let modes = KeyModes { composing: false, ..modes };
                self.send_input(crate::keys::committed_text(None, modes, &marked));
            }
        }
        let Some((report, down)) = self.with(|win, p| {
            let report = win.focused && p.session.lock_term().state.modes().focus_events;
            p.swallow_click = false;
            (report, p.keys_down.take())
        }) else {
            return;
        };
        for code in down {
            let plain = crate::ascii_layout::chars(code).0;
            if let Some(bytes) = crate::keys::focus_release(code, plain, modes) {
                self.send_raw(bytes);
            }
        }
        if report {
            self.send_raw(b"\x1b[O".to_vec());
        }
        self.request_redraw();
    }

    fn window_view(&self) -> Option<Retained<WindowView>> {
        self.ivars().window.load()
    }

    fn pane(&self) -> PaneId {
        self.ivars().pane
    }

    /// `f` on the window and this pane; `None` before the window view
    /// started or once the pane is gone.
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut Win, &mut Pane) -> R) -> Option<R> {
        self.window_view()?.with_pane(self.pane(), f)
    }

    fn request_redraw(&self) {
        if let Some(w) = self.window_view() {
            w.request_redraw(self.pane());
        }
    }

    /// Close this pane (the window with its last).
    fn close(&self) {
        if let Some(w) = self.window_view() {
            w.close_pane(self.pane());
        }
    }

    /// Input typed by the user: snaps the view back to the live screen.
    pub(crate) fn send_input(&self, bytes: Vec<u8>) {
        let len = bytes.len();
        let key_t0 = self.with(|win, p| {
            p.cursor_on = true;
            win.last_input = Instant::now();
            let mut t = p.session.lock_term();
            t.state.scroll_view_to_bottom();
            t.state.set_selection(None);
            drop(t);
            p.key_t0.take()
        });
        // Logged before the writer has the bytes, so the echo can't be
        // parsed before the key is waited for.
        if let (Some(lat), Some(Some((t0, key_down))), Some(w)) = (latency::get(), key_t0, self.window_view()) {
            lat.key_sent(w.id(), t0, key_down, len);
        }
        self.send_raw(bytes);
        // Don't draw yet: the echo is usually well under a millisecond away.
        // A frame now would take the free drawable and clear the dirty flag,
        // so the echo would wait for the next tick. Instead the first output
        // after this is drawn as soon as it's parsed (`output_arrived`);
        // with none, a tick draws after `ECHO_WAIT`.
        self.with(|win, p| {
            p.awaiting_echo = Some(Instant::now());
            win.idle_ticks = 0;
            win.link.setPaused(false);
        });
    }

    pub(crate) fn send_raw(&self, bytes: Vec<u8>) {
        self.with(|_, p| p.session.send(bytes));
    }

    /// Type `text` as a paste: bracketed when the program asked for it.
    pub(crate) fn paste_text(&self, text: &str) {
        let bracketed = self
            .with(|_, p| p.session.lock_term().state.modes().bracketed_paste)
            .unwrap_or(false);
        self.send_input(paste_bytes(text, bracketed));
    }

    /// Something dropped on the pane: files and folders as their paths,
    /// quoted for the shell and separated by spaces; else a URL or text as
    /// it is.
    fn drop_from(&self, pb: &NSPasteboard) -> bool {
        let paths = file_paths(pb);
        let text = if !paths.is_empty() {
            crate::desktop::dropped_text(&paths)
        } else {
            let text = unsafe { pb.stringForType(NSPasteboardTypeURL) }.or_else(|| unsafe { pb.stringForType(NSPasteboardTypeString) });
            match text {
                Some(t) => t.to_string(),
                None => return false,
            }
        };
        self.paste_text(&text);
        true
    }

    /// The right-click (or Control-click) menu: Open Link and Copy Link
    /// on a link, Copy, Paste, Select All and Clear Scrollback, for when
    /// the program isn't taking the mouse.
    fn show_context_menu(&self, event: &NSEvent) {
        let mtm = MainThreadMarker::from(self);
        let cell = self.cell_at(event);
        let link = cell.and_then(|(line, col)| self.link_at_cell(line, col));
        self.ivars().menu_link.replace(link.clone());
        let output = cell
            .and_then(|(line, col)| {
                self.with(|_, p| {
                    let t = p.session.lock_term();
                    let at = t.state.point_at(line, col);
                    crate::prompts::has_output(&t.state, at).then_some(at)
                })
            })
            .flatten();
        self.ivars().menu_point.set(output);
        let has_selection = self
            .with(|_, p| p.session.lock_term().state.selection_text().is_some_and(|t| !t.is_empty()))
            .unwrap_or(false);
        let can_paste = unsafe { NSPasteboard::generalPasteboard().stringForType(NSPasteboardTypeString) }.is_some();
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        let add = |title: &str, action: Sel, enabled: bool| {
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    Some(action),
                    &NSString::new(),
                )
            };
            unsafe { item.setTarget(Some(self)) };
            item.setEnabled(enabled);
            menu.addItem(&item);
        };
        if link.is_some() {
            add("Open Link", sel!(openLink:), true);
            add("Copy Link", sel!(copyLink:), true);
            menu.addItem(&NSMenuItem::separatorItem(mtm));
        }
        add("Copy", sel!(copy:), has_selection);
        add("Paste", sel!(paste:), can_paste);
        add("Select All", sel!(selectAll:), true);
        if output.is_some() {
            add("Select Command Output", sel!(selectClickedOutput:), true);
        }
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        add("Clear Scrollback", sel!(clearBuffer:), true);
        NSMenu::popUpContextMenu_withEvent_forView(&menu, event, self);
    }

    fn handle_key(&self, event: &NSEvent) {
        if latency::get().is_some() {
            let times = (event.timestamp(), latency::now());
            self.with(|_, p| p.key_t0 = Some(times));
        }
        self.handle_key_event(event);
        self.with(|_, p| p.key_t0 = None);
    }

    fn handle_key_event(&self, event: &NSEvent) {
        let input = key_input(event);
        // A held pane closes on any key; Cmd shortcuts (copy the error,
        // close, quit) still go to the menus.
        if input.flags & crate::keys::flags::COMMAND == 0 && self.with(|_, p| p.held) == Some(true) {
            self.close();
            return;
        }
        let Some(modes) = self.key_modes() else { return };
        // `[keybind]` first; an input method composing gets every key but
        // Cmd ones.
        let decision = self.with(|win, _| {
            if modes.composing && input.flags & crate::keys::flags::COMMAND == 0 {
                return None;
            }
            win.binds.decide(&input).map(|d| match d {
                keybind::Decision::Send(text) => Err(text.as_bytes().to_vec()),
                keybind::Decision::Perform(action) => Ok(action),
            })
        });
        match decision.flatten() {
            Some(Err(bytes)) => return self.send_input(bytes),
            Some(Ok(action)) => {
                let app = NSApplication::sharedApplication(MainThreadMarker::from(self));
                unsafe { app.sendAction_to_from(Sel::register(action.selector()), None, Some(self)) };
                return;
            }
            None => {}
        }
        let action = crate::keys::translate(&input, modes);
        if input.flags & crate::keys::flags::COMMAND == 0 {
            NSCursor::setHiddenUntilMouseMoves(true);
        }
        let went_to_program = matches!(action, KeyAction::Send(_)) || (action == KeyAction::ToTextInput && !modes.composing);
        if went_to_program {
            self.with(|_, p| p.keys_down.press(input.keycode));
        }
        match action {
            KeyAction::Send(bytes) => self.send_input(bytes),
            KeyAction::ToMenu | KeyAction::Ignore => {}
            KeyAction::ToTextInput => {
                // Text, dead keys and input methods.
                let key = (!modes.composing).then_some((input, modes));
                self.with(|_, p| p.text_key = key);
                let events = NSArray::from_slice(&[event]);
                self.interpretKeyEvents(&events);
                self.with(|_, p| p.text_key = None);
            }
        }
    }

    /// How keys are encoded right now.
    fn key_modes(&self) -> Option<KeyModes> {
        self.with(|win, p| {
            let t = p.session.lock_term();
            KeyModes {
                app_cursor: t.state.modes().app_cursor,
                app_keypad: t.state.modes().app_keypad,
                newline: t.state.modes().linefeed_newline,
                kitty_flags: t.state.kitty_keyboard_flags(),
                option_as_meta: win.config.option_as_meta,
                composing: !p.marked.is_empty(),
            }
        })
    }

    /// A key came up: a release event if the program asked for them
    /// (kitty flag 2) and the press went to it.
    fn handle_key_up(&self, event: &NSEvent) {
        let Some(modes) = self.key_modes() else { return };
        let input = KeyInput {
            event: KeyEventType::Release,
            ..key_input(event)
        };
        let was_down = self.with(|_, p| p.keys_down.release(input.keycode)).unwrap_or(false);
        if modes.kitty_flags & crate::keys::kitty::EVENT_TYPES == 0 || !was_down {
            return;
        }
        if let KeyAction::Send(bytes) = crate::keys::translate(&input, modes) {
            // Not typing: the view stays where it is.
            self.send_raw(bytes);
        }
    }

    /// A modifier key went down or up: reported only when the program
    /// asked for every key (kitty flag 8). A release is dropped if this
    /// pane didn't see the press (it went to another pane).
    fn handle_flags_changed(&self, event: &NSEvent) {
        self.hover_with_cmd(event.modifierFlags().contains(NSEventModifierFlags::Command));
        let Some(modes) = self.key_modes() else { return };
        // `flagsChanged` events have no characters.
        let input = KeyInput {
            keycode: event.keyCode(),
            flags: event.modifierFlags().0,
            ..KeyInput::default()
        };
        let seen = self.with(|_, p| match crate::keys::modifier_pressed(input.keycode, input.flags) {
            Some(true) => {
                p.keys_down.press(input.keycode);
                true
            }
            Some(false) => p.keys_down.release(input.keycode),
            None => true,
        });
        if seen != Some(true) || modes.kitty_flags & crate::keys::kitty::ALL_KEYS == 0 {
            return;
        }
        if let KeyAction::Send(bytes) = crate::keys::translate(&input, modes) {
            self.send_raw(bytes);
        }
    }

    /// Visible line and column under the mouse, clamped to the grid.
    fn cell_at(&self, event: &NSEvent) -> Option<(usize, usize)> {
        self.cell_at_point(self.convertPoint_fromView(event.locationInWindow(), None))
    }

    /// Visible line and column at `at` (in this view), clamped to the grid.
    fn cell_at_point(&self, at: NSPoint) -> Option<(usize, usize)> {
        let titlebar = self.window_view().map_or(0.0, |w| w.titlebar_height());
        self.with(|win, p| {
            let m = win.renderer.metrics();
            let pad = pane_padding(&win.config, titlebar, win.scale, p.rect);
            let t = p.session.lock_term();
            layout::cell_at((at.x, at.y), win.scale, m.into(), pad, t.state.cols(), t.state.lines())
        })
    }

    /// The link at visible `line`, `col`: OSC 8, else a plain URL.
    /// Option-click on view cell (`line`, `col`): if it is on the shell's
    /// command line ([`crate::prompts::input_steps`]), type the arrow keys
    /// that take the shell's cursor there. False if it isn't, and nothing
    /// was typed.
    pub(crate) fn move_shell_cursor(&self, line: usize, col: usize) -> bool {
        let Some((steps, app_cursor)) = self
            .with(|_, p| {
                let t = p.session.lock_term();
                crate::prompts::input_steps(&t.state, line, col).map(|s| (s, t.state.modes().app_cursor))
            })
            .flatten()
        else {
            return false;
        };
        if steps != 0 {
            self.send_input(crate::prompts::arrows(steps, app_cursor));
        }
        true
    }

    fn link_at_cell(&self, line: usize, col: usize) -> Option<String> {
        self.with(|_, p| {
            let t = p.session.lock_term();
            crate::links::under(&t.state, t.state.point_at(line, col))
        })
        .flatten()
    }

    /// The pointer is over visible `cell` with Cmd held, or `None`: the
    /// link there (every piece of it) is underlined, and the pointer is a
    /// pointing hand while it is over one.
    pub(crate) fn set_hover(&self, cell: Option<(usize, usize)>) {
        let Some((changed, on_link)) = self.with(|_, p| {
            let mut t = p.session.lock_term();
            let point = cell.map(|(line, col)| t.state.point_at(line, col));
            let changed = t.state.hover() != point;
            t.state.set_hover(point);
            (changed, point.is_some_and(|at| crate::links::under(&t.state, at).is_some()))
        }) else {
            return;
        };
        if changed {
            self.request_redraw();
        }
        if on_link != self.ivars().hand.replace(on_link) {
            if on_link {
                NSCursor::pointingHandCursor().set();
            } else {
                NSCursor::IBeamCursor().set();
            }
        }
    }

    /// Cmd went down or up: every pane of the window hovers what is under
    /// the pointer, or stops (the key goes to the focused pane only).
    fn hover_with_cmd(&self, cmd: bool) {
        let views = match self.window_view() {
            Some(w) => w.with_state(|st| st.panes.values().map(|p| p.view.clone()).collect::<Vec<_>>()),
            None => None,
        };
        let Some(loc) = self.window().map(|w| w.mouseLocationOutsideOfEventStream()) else {
            return;
        };
        for view in views.unwrap_or_default() {
            let at = view.convertPoint_fromView(loc, None);
            let b = view.bounds();
            let inside = at.x >= b.origin.x && at.y >= b.origin.y && at.x < b.origin.x + b.size.width && at.y < b.origin.y + b.size.height;
            view.set_hover(if cmd && inside { view.cell_at_point(at) } else { None });
        }
    }

    /// Jump to Previous / Next Prompt: scroll the view to it, or beep if
    /// there is none that way ([`crate::prompts::jump`]).
    fn jump_to_prompt(&self, up: bool) {
        match self.with(|_, p| crate::prompts::jump(&mut p.session.lock_term().state, up)) {
            Some(true) => self.request_redraw(),
            _ => NSBeep(),
        }
    }

    /// Select the output of the command at `at`, or with `None` the one
    /// at the top of the view or the last ([`crate::prompts::select_output`]);
    /// beep if there is none.
    fn select_output_at(&self, at: Option<Point>) {
        self.clear_other_selections();
        let selected = self.with(|_, p| {
            let mut t = p.session.lock_term();
            match at {
                Some(at) => t.state.select_output(at),
                None => crate::prompts::select_output(&mut t.state),
            }
        });
        match selected {
            Some(true) => self.request_redraw(),
            _ => NSBeep(),
        }
    }

    /// Show the find bar (with the text last searched for here, or the
    /// shared find text), give its field the keys and search.
    pub(crate) fn open_find(&self) {
        let mtm = MainThreadMarker::from(self);
        let Some((bar, query, opened)) = self.with(|_, p| {
            let opened = p.find.is_none();
            if opened
                && p.find_query.text.is_empty()
                && let Some(shared) = crate::find_bar::shared_query()
            {
                p.find_query = shared;
            }
            (p.find.as_ref().map(|f| f.bar.clone()), p.find_query.clone(), opened)
        }) else {
            return;
        };
        let bar = match bar {
            Some(bar) => bar,
            None => {
                let bar = FindBar::new(mtm, self, &query);
                self.addSubview(&bar);
                let search = self.new_search();
                self.with(|_, p| p.find = Some(Find::new(bar.clone(), search)));
                self.place_find_bar();
                bar
            }
        };
        bar.focus();
        if opened {
            self.find_query_changed(query);
        }
    }

    /// Put the find bar at the pane's top, below the title bar if the
    /// pane touches it.
    pub(crate) fn place_find_bar(&self) {
        let top = self.with(|_, p| p.rect.y == 0).unwrap_or(false);
        let inset = match (top, self.window_view()) {
            (true, Some(w)) => w.titlebar_height(),
            _ => 0.0,
        };
        if let Some(bar) = self.with(|_, p| p.find.as_ref().map(|f| f.bar.clone())).flatten() {
            bar.place(self.frame().size.width, inset);
        }
    }

    /// term-core's search, its batches added on the main thread.
    fn new_search(&self) -> Box<dyn crate::search::Search> {
        let window = self.window_view().map_or(0, |w| w.id());
        let pane = self.pane();
        let deliver: crate::search::Deliver = std::sync::Arc::new(move |batch| {
            dispatch2::DispatchQueue::main().exec_async(move || {
                crate::view::with_view(window, |v| v.search_batch(pane, batch));
            });
        });
        Box::new(crate::search::CoreSearch::new(deliver))
    }

    /// How many of the grid's top rows the find bar is over.
    fn rows_under_find_bar(&self) -> usize {
        let titlebar = self.window_view().map_or(0.0, |w| w.titlebar_height());
        self.with(|win, p| crate::view::rows_under_find_bar(win, p, titlebar)).unwrap_or(0)
    }

    /// Hide the find bar and drop its hits; the keys go back to the pane.
    pub(crate) fn close_find(&self) {
        let Some(Some(f)) = self.with(|_, p| {
            let f = p.find.take()?;
            let mut t = p.session.lock_term();
            let mut f = f;
            f.search.cancel(&mut t.state);
            Some(f)
        }) else {
            return;
        };
        let had_keys = f.bar.has_focus();
        f.bar.stop();
        f.bar.removeFromSuperview();
        if had_keys && let Some(w) = self.window() {
            w.makeFirstResponder(Some(self));
        }
        self.request_redraw();
    }

    /// A click landed in the find bar: this pane is the focused one, the
    /// keys staying in the bar's field.
    pub(crate) fn bar_clicked(&self) {
        if let Some(w) = self.window_view() {
            w.pane_focused(self.pane(), false);
        }
    }

    /// The test control's `find-click`: click the find bar's field.
    pub(crate) fn click_find_bar(&self) {
        match self.with(|_, p| p.find.as_ref().map(|f| f.bar.clone())).flatten() {
            Some(bar) => bar.post_click(),
            None => NSBeep(),
        }
    }

    /// Whether `q` isn't what was last searched for.
    pub(crate) fn find_query_differs(&self, q: &Query) -> bool {
        self.with(|_, p| p.find_query != *q).unwrap_or(false)
    }

    /// Search for `q`: the hits are drawn, the current one (the first at
    /// or after the view's top) scrolled into view, the count shown.
    pub(crate) fn find_query_changed(&self, q: Query) {
        let covered = self.rows_under_find_bar();
        let Some(Some((bar, status))) = self.with(|_, p| {
            let f = p.find.as_mut()?;
            let mut t = p.session.lock_term();
            f.search.start(&mut t.state, &q, covered);
            f.seen = Some((t.state.terminal_id(), t.state.alt_screen_active()));
            p.find_query = q.clone();
            Some((f.bar.clone(), f.search.status(&t.state)))
        }) else {
            return;
        };
        if !q.text.is_empty() {
            crate::find_bar::share_query(&q);
        }
        bar.show_status(status);
        self.request_redraw();
    }

    /// Find Next / Find Previous: the next or previous hit becomes
    /// current and is scrolled into view; with the bar closed, it opens.
    pub(crate) fn find_step(&self, forward: bool) {
        if self.with(|_, p| p.find.is_none()).unwrap_or(true) {
            self.open_find();
            return;
        }
        let covered = self.rows_under_find_bar();
        let stepped = self.with(|_, p| {
            let f = p.find.as_mut()?;
            let mut t = p.session.lock_term();
            let stepped = f.search.step(&mut t.state, forward, covered);
            let status = f.search.status(&t.state);
            // None yet, but some may come: it jumps when one arrives.
            (stepped || status.running).then(|| (f.bar.clone(), status))
        });
        match stepped.flatten() {
            Some((bar, status)) => {
                bar.show_status(status);
                self.request_redraw();
            }
            None => NSBeep(),
        }
    }

    /// The test control's `find TEXT`: open the bar, put TEXT in it and
    /// search.
    pub(crate) fn find_text(&self, text: &str) {
        self.open_find();
        let Some(Some(bar)) = self.with(|_, p| p.find.as_ref().map(|f| f.bar.clone())) else {
            return;
        };
        let q = Query {
            text: text.to_string(),
            ..bar.query()
        };
        bar.set_query(&q);
        self.find_query_changed(q);
    }

    /// The app became active: if another app changed the shared find text
    /// or options since, an open bar takes them and searches again.
    pub(crate) fn read_shared_find(&self) {
        let Some(shared) = crate::find_bar::shared_query().filter(|q| !q.text.is_empty()) else {
            return;
        };
        let bar = self.with(|_, p| {
            let f = p.find.as_ref()?;
            // Not while there is typing the search hasn't caught up with.
            (p.find_query != shared && f.bar.query() == p.find_query).then(|| f.bar.clone())
        });
        if let Some(Some(bar)) = bar {
            bar.set_query(&shared);
            self.find_query_changed(shared);
        }
    }

    /// Use Selection for Find: the selected text becomes the query.
    fn use_selection(&self) {
        let Some(Some(text)) = self.with(|_, p| p.session.lock_term().state.selection_text()) else {
            NSBeep();
            return;
        };
        let q = self
            .with(|_, p| Query {
                text: text.clone(),
                ..p.find_query.clone()
            })
            .unwrap_or_default();
        crate::find_bar::share_query(&q);
        let open = self.with(|_, p| p.find.as_ref().map(|f| f.bar.clone())).flatten();
        match open {
            Some(bar) => bar.set_query(&q),
            None => {
                self.with(|_, p| p.find_query = q.clone());
                self.open_find();
                return;
            }
        }
        self.find_query_changed(q);
    }

    /// Cmd-click or Open Link: open `uri` if it is a link that may open
    /// ([`crate::links::to_open`]); else say why not in the terminal.
    pub(crate) fn open_link(&self, uri: &str) {
        let url = match crate::links::to_open(uri, &crate::links::host_name()) {
            Ok(url) => url,
            Err(why) => return self.notice(&why.notice(uri)),
        };
        if self.window_view().is_some_and(|w| w.opened_in_test(&url)) {
            return;
        }
        let opened = NSURL::URLWithString(&NSString::from_str(&url)).is_some_and(|u| NSWorkspace::sharedWorkspace().openURL(&u));
        if !opened {
            self.notice(&format!("couldn't open {url}"));
        }
    }

    /// A notice from Terminal Decline in the terminal, on a line of its
    /// own; a beep instead over a full-screen program.
    fn notice(&self, text: &str) {
        let written = self.with(|_, p| {
            let mut t = p.session.lock_term();
            let full_screen = t.state.alt_screen_active();
            if !full_screen {
                crate::session::write_late_notice(&mut t, text);
            }
            !full_screen
        });
        match written {
            Some(true) => self.request_redraw(),
            _ => NSBeep(),
        }
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
        let (mode, sgr) = self.with(|_, p| {
            let t = p.session.lock_term();
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
        self.with(|win, _| win.last_input = Instant::now());
        let Some((line, col)) = self.cell_at(event) else { return };
        // Cmd-click on a link opens it, even while the program takes the
        // mouse; the rest of the click (drag, release) goes nowhere.
        if button == mouse::LEFT
            && press
            && event.modifierFlags().contains(NSEventModifierFlags::Command)
            && let Some(uri) = self.link_at_cell(line, col)
        {
            self.with(|_, p| p.swallow_click = true);
            self.open_link(&uri);
            return;
        }
        // A selection being made ends on its own button's release, even
        // if Shift (which kept the program from the mouse) is let go first.
        if button == mouse::LEFT && !press && self.with(|_, p| p.selecting.is_some()) == Some(true) {
            self.end_selection();
            return;
        }
        if let Some((_, sgr)) = self.reporting(event) {
            let cleared = self.with(|_, p| {
                p.mouse_button = press.then_some(button);
                let cleared = select::reported(&mut p.session.lock_term().state, press);
                if cleared {
                    p.block = None;
                }
                cleared
            });
            self.send_mouse(button + Self::mods_of(event), line, col, press, sgr);
            if cleared == Some(true) {
                self.request_redraw();
            }
            return;
        }
        // Option-click on the shell's command line moves its cursor there;
        // the rest of the click (drag, release) goes nowhere.
        let flags = event.modifierFlags();
        if button == mouse::LEFT
            && press
            && flags.contains(NSEventModifierFlags::Option)
            && !flags.intersects(NSEventModifierFlags::Shift | NSEventModifierFlags::Command | NSEventModifierFlags::Control)
            && self.move_shell_cursor(line, col)
        {
            self.with(|_, p| p.swallow_click = true);
            return;
        }
        if press && mouse::opens_menu(false, button, event.modifierFlags().contains(NSEventModifierFlags::Control)) {
            self.show_context_menu(event);
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
        let at = self.px_at(event);
        self.clear_other_selections();
        self.with(|_, p| {
            p.selecting = Some(gesture);
            p.dragged = false;
            p.autoscroll = None;
            p.pending = None;
            let block = p.block.take();
            let mut t = p.session.lock_term();
            let s = &mut t.state;
            let point = s.point_at(line, col);
            match gesture {
                // A click clears the selection; one starts at the cell
                // only once the mouse is dragged far enough from it.
                Gesture::Start | Gesture::Block => {
                    s.set_selection(None);
                    p.pending = Some(select::Pending {
                        anchor: point,
                        at,
                        block: gesture == Gesture::Block,
                    });
                    p.scroll_accum = 0.0;
                }
                Gesture::Word => s.select_word(point),
                Gesture::Line => s.select_line(point),
                Gesture::Extend => {
                    // A rectangle still shown grows as a rectangle.
                    match block.filter(|b| s.selection() == Some(b.stream())) {
                        Some(mut b) => {
                            b.head = point;
                            s.set_selection(Some(b.stream()));
                            p.block = Some(b);
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
        let copy = self.with(|win, p| {
            let gesture = p.selecting.take();
            p.autoscroll = None;
            p.pending = None;
            win.config.copy_on_select && gesture.is_some_and(|g| select::copies_on_release(g, p.dragged))
        });
        if copy == Some(true) {
            self.copy_selection();
        }
    }

    /// Put the selection (or the rectangle being shown) on the clipboard.
    fn copy_selection(&self) {
        let text = self
            .with(|_, p| {
                let mut t = p.session.lock_term();
                match p.block.filter(|b| t.state.selection() == Some(b.stream())) {
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

    /// One selection per window: this pane's is starting, so the other
    /// panes' go.
    fn clear_other_selections(&self) {
        if let Some(w) = self.window_view() {
            w.clear_other_selections(self.pane());
        }
    }

    /// Where `event` is, in this view's pixels.
    fn px_at(&self, event: &NSEvent) -> (f64, f64) {
        let at = self.convertPoint_fromView(event.locationInWindow(), None);
        let scale = self.with(|win, _| win.scale).unwrap_or(1.0);
        (at.x * scale, at.y * scale)
    }

    /// Start the selection of a click whose mouse has now moved to `at`
    /// (pixels), if that is far enough. Whether a selection is being
    /// made (it started now or before).
    fn start_pending(p: &mut Pane, at: (f64, f64), cell_width: f64) -> bool {
        let Some(pending) = p.pending else { return true };
        if !pending.passed(at, cell_width) {
            return false;
        }
        p.pending = None;
        p.block = pending.start(&mut p.session.lock_term().state);
        true
    }

    /// Move the selection being made to the cell at view `line`, `col`.
    /// Whether it changed.
    fn extend_selection_to(p: &mut Pane, line: usize, col: usize) -> bool {
        let mut t = p.session.lock_term();
        let before = t.state.selection();
        let Some(sel) = before else { return false };
        let to = t.state.point_at(line, col);
        match p.block.as_mut().filter(|b| sel == b.stream()) {
            Some(b) => {
                b.head = to;
                t.state.set_selection(Some(b.stream()));
            }
            None => t.state.extend_selection(to),
        }
        let changed = t.state.selection() != before;
        drop(t);
        p.dragged |= changed;
        changed
    }

    /// Auto-scroll speed for a drag at `event`'s height (lines a second;
    /// positive scrolls back), 0 while it is over the rows.
    fn autoscroll_speed(&self, event: &NSEvent) -> f64 {
        let at = self.convertPoint_fromView(event.locationInWindow(), None);
        let titlebar = self.window_view().map_or(0.0, |w| w.titlebar_height());
        self.with(|win, p| {
            let m = win.renderer.metrics();
            let pad = pane_padding(&win.config, titlebar, win.scale, p.rect);
            let lines = p.session.lock_term().state.lines();
            let top = pad.top as f64;
            let bottom = top + (lines as u32 * m.cell_height) as f64;
            select::autoscroll_speed(at.y * win.scale, top, bottom, m.cell_height as f64)
        })
        .unwrap_or(0.0)
    }

    /// One auto-scroll step, then the next one while the mouse is still
    /// held past the edge. Nothing is scheduled once it is back over the
    /// rows or the button is let go.
    fn autoscroll_tick(&self) {
        let step = self.with(|_, p| {
            p.autoscroll_scheduled = false;
            if p.selecting.is_none() {
                p.autoscroll = None;
            }
            let a = p.autoscroll.as_mut()?;
            let now = Instant::now();
            let lines = select::autoscroll_step(a.speed, (now - a.last).as_secs_f64(), &mut a.carry);
            a.last = now;
            let (up, col) = (a.speed > 0.0, a.col);
            let mut changed = false;
            if lines != 0 {
                let mut t = p.session.lock_term();
                let before = t.state.view_top();
                t.state.scroll_view(lines);
                changed = t.state.view_top() != before;
                let row = if up { 0 } else { t.state.lines() - 1 };
                drop(t);
                changed |= Self::extend_selection_to(p, row, col);
            }
            p.autoscroll_scheduled = true;
            Some(changed)
        });
        let Some(Some(changed)) = step else { return };
        if changed {
            self.request_redraw();
        }
        self.schedule_autoscroll();
    }

    fn schedule_autoscroll(&self) {
        let Some(window) = self.window_view() else { return };
        let (id, pane) = (window.id(), self.pane());
        let at = dispatch2::DispatchTime::NOW.time(select::INTERVAL.as_nanos() as i64);
        let _ = dispatch2::DispatchQueue::main().after(at, move || {
            with_view(id, |v| {
                if let Some(view) = v.with_pane(pane, |_, p| p.view.clone()) {
                    view.autoscroll_tick();
                }
            })
        });
    }

    fn mouse_motion(&self, event: &NSEvent, button: Option<u8>) {
        let Some((line, col)) = self.cell_at(event) else { return };
        let cmd = event.modifierFlags().contains(NSEventModifierFlags::Command);
        self.set_hover((cmd && button.is_none()).then_some((line, col)));
        let selecting = self.with(|_, p| p.selecting.is_some()) == Some(true);
        if !selecting && let Some((mode, sgr)) = self.reporting(event) {
            let held = self.with(|_, p| p.mouse_button).flatten();
            if mouse::reports_motion(mode, held) {
                self.send_mouse(mouse::motion_code(held, button, Self::mods_of(event)), line, col, true, sgr);
            }
            return;
        }
        if button != Some(mouse::LEFT) || !selecting {
            return;
        }
        let speed = self.autoscroll_speed(event);
        let at = self.px_at(event);
        let (changed, start_timer) = self
            .with(|win, p| {
                if !Self::start_pending(p, at, win.renderer.metrics().cell_width as f64) {
                    return (false, false);
                }
                // Past the top or bottom edge, a timer keeps scrolling
                // while the mouse is held still; it stops once back over
                // the rows or the button is let go.
                p.autoscroll = (speed != 0.0).then(|| {
                    let (last, carry) = p.autoscroll.as_ref().map_or((Instant::now(), 0.0), |a| (a.last, a.carry));
                    AutoScroll { speed, col, last, carry }
                });
                let start = p.autoscroll.is_some() && !p.autoscroll_scheduled;
                p.autoscroll_scheduled |= start;
                (Self::extend_selection_to(p, line, col), start)
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
        self.with(|win, _| win.last_input = Instant::now());
        let precise = event.hasPreciseScrollingDeltas();
        let reporting = self.reporting(event);
        let Some((line, col)) = self.cell_at(event) else { return };
        let lines = self.with(|win, p| {
            let dy = event.scrollingDeltaY();
            let delta = if precise {
                dy * win.scale / win.renderer.metrics().cell_height as f64
            } else {
                dy
            };
            p.scroll_accum += delta;
            let whole = p.scroll_accum.trunc();
            p.scroll_accum -= whole;
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
        let alt = self.with(|_, p| {
            let t = p.session.lock_term();
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
        self.with(|_, p| p.session.lock_term().state.scroll_view(lines));
        self.request_redraw();
    }
}

/// What `keys::translate` needs from a key event.
pub(crate) fn key_input(event: &NSEvent) -> KeyInput {
    let flags = event.modifierFlags();
    let (ascii, ascii_shifted) = crate::ascii_layout::chars(event.keyCode());
    let plain = event.charactersIgnoringModifiers().map(|s| s.to_string()).unwrap_or_default();
    let shifted = if flags.contains(NSEventModifierFlags::Shift) {
        NSEventModifierFlags::Shift
    } else {
        NSEventModifierFlags::empty()
    };
    KeyInput {
        keycode: event.keyCode(),
        chars: event.characters().map(|s| s.to_string()).unwrap_or_default(),
        chars_unshifted: event
            .charactersByApplyingModifiers(NSEventModifierFlags::empty())
            .map(|s| s.to_string())
            .unwrap_or_default(),
        event: if event.isARepeat() {
            KeyEventType::Repeat
        } else {
            KeyEventType::Press
        },
        chars_shift_only: event
            .charactersByApplyingModifiers(shifted)
            .map(|s| s.to_string())
            .unwrap_or_else(|| plain.clone()),
        chars_ignoring_mods: plain,
        ascii,
        ascii_shifted,
        flags: flags.0,
        numpad: flags.contains(NSEventModifierFlags::NumericPad),
    }
}
