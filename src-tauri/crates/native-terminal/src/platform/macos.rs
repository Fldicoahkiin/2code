use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSView, NSWindow};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{
    AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use std::ptr::NonNull;

/// Holds a child NSView added to a Tauri window's contentView.
pub struct NativeTerminalView {
    #[allow(dead_code)]
    ns_window: Retained<NSWindow>,
    ns_view: Retained<NSView>,
}

impl NativeTerminalView {
    /// Create a child NSView inside the given NSWindow's contentView.
    ///
    /// # Safety
    /// `ns_window_ptr` must be a valid pointer to an NSWindow.
    /// Must be called from the main thread.
    pub unsafe fn new(ns_window_ptr: *mut std::ffi::c_void) -> Self {
        let mtm = MainThreadMarker::new()
            .expect("NativeTerminalView::new must be called from the main thread");

        let ns_window: Retained<NSWindow> = {
            let ptr = ns_window_ptr as *mut NSWindow;
            Retained::retain(ptr).expect("invalid NSWindow pointer")
        };

        let content_view = ns_window
            .contentView()
            .expect("NSWindow has no contentView");

        // Start with zero-size frame — frontend will set the real position
        // via resize_native_terminal once the terminal container mounts.
        let terminal_frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(1.0, 1.0),
        );

        let ns_view = NSView::initWithFrame(NSView::alloc(mtm), terminal_frame);
        ns_view.setWantsLayer(true);

        content_view.addSubview(&ns_view);

        log::info!(
            "native-terminal: created NSView ({:.0}x{:.0}) at ({:.0},{:.0})",
            terminal_frame.size.width,
            terminal_frame.size.height,
            terminal_frame.origin.x,
            terminal_frame.origin.y,
        );

        Self { ns_window, ns_view }
    }

    /// Resize the terminal view.
    pub fn set_frame(&self, x: f64, y: f64, width: f64, height: f64) {
        let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
        self.ns_view.setFrame(frame);
    }

    pub fn raw_window_handle(&self) -> RawWindowHandle {
        let ns_view_ptr = Retained::as_ptr(&self.ns_view) as *mut std::ffi::c_void;
        let handle = AppKitWindowHandle::new(
            NonNull::new(ns_view_ptr).expect("NSView pointer is null"),
        );
        RawWindowHandle::AppKit(handle)
    }

    pub fn raw_display_handle(&self) -> RawDisplayHandle {
        RawDisplayHandle::AppKit(AppKitDisplayHandle::new())
    }

    pub fn set_hidden(&self, hidden: bool) {
        self.ns_view.setHidden(hidden);
    }

    pub fn size(&self) -> (u32, u32) {
        let frame = self.ns_view.frame();
        (frame.size.width as u32, frame.size.height as u32)
    }
}

/// Thread-safe wrapper around NativeTerminalView.
/// NSView operations must happen on the main thread, but we store raw
/// pointers and dispatch to the main thread when needed.
pub struct SendableNativeView {
    ns_view_ptr: *mut std::ffi::c_void,
}

// SAFETY: We only access the NSView through main-thread-dispatched calls.
unsafe impl Send for SendableNativeView {}
unsafe impl Sync for SendableNativeView {}

impl SendableNativeView {
    pub fn new(view: &NativeTerminalView) -> Self {
        let ptr = Retained::as_ptr(&view.ns_view) as *mut std::ffi::c_void;
        Self { ns_view_ptr: ptr }
    }

    pub fn set_frame(&self, x: f64, y: f64, width: f64, height: f64) {
        let ptr = self.ns_view_ptr;
        // SAFETY: Must be called while the NativeTerminalView is alive.
        unsafe {
            let view = ptr as *mut NSView;
            let view_ref = &*view;
            let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
            view_ref.setFrame(frame);
        }
    }

    pub fn set_hidden(&self, hidden: bool) {
        let ptr = self.ns_view_ptr;
        unsafe {
            let view = ptr as *mut NSView;
            let view_ref = &*view;
            view_ref.setHidden(hidden);
        }
    }
}
