//! A divider's view: a plain `NSView` over the grab zone of one divider
//! between panes (the 1 pt line and 3 pt either side, below the title bar),
//! which draws nothing (the window view draws the line). It is there to
//! take the mouse (splits design §1, §6): a drag moves its split, a
//! double-click evens out the panes it runs between. The window view lays
//! these out with the panes, keeps them above the pane views so they win
//! hit testing, and sets the resize cursor over them.

use objc2::rc::{Retained, Weak};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSResponder, NSView};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect};

use crate::view::WindowView;

pub struct Ivars {
    /// The window view this divider is laid over.
    window: Weak<WindowView>,
    /// Which divider it is, in the layout's order.
    index: usize,
}

define_class!(
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeclineDividerView"]
    #[ivars = Ivars]
    pub struct DividerView;

    impl DividerView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// A drag in an inactive window moves the divider too, as in
        /// Terminal and iTerm2.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if let Some(w) = self.ivars().window.load() {
                w.divider_down(self.ivars().index, self.location(&w, event), event.clickCount());
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            if let Some(w) = self.ivars().window.load() {
                w.divider_dragged(self.location(&w, event));
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            if let Some(w) = self.ivars().window.load() {
                w.divider_up();
            }
        }
    }

    unsafe impl NSObjectProtocol for DividerView {}
);

impl DividerView {
    pub fn new(mtm: MainThreadMarker, window: &WindowView, index: usize, frame: NSRect) -> Retained<DividerView> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            window: Weak::new(window),
            index,
        });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// Where `event` happened, in the window view's points (y down).
    fn location(&self, window: &WindowView, event: &NSEvent) -> NSPoint {
        window.convertPoint_fromView(event.locationInWindow(), None)
    }
}
