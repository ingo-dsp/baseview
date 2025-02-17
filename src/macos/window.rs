use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cocoa::appkit::{
    NSApp, NSApplication, NSApplicationActivationPolicyRegular, NSBackingStoreBuffered, NSWindow,
    NSWindowStyleMask,
};
use cocoa::base::{id, nil, YES, NO};
use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize, NSString};
use core_foundation::runloop::{
    CFRunLoop, CFRunLoopTimer, CFRunLoopTimerContext, __CFRunLoopTimer, kCFRunLoopDefaultMode,
};
use keyboard_types::KeyboardEvent;

use objc::{msg_send, runtime::Object, sel, sel_impl};

use raw_window_handle::{AppKitHandle, HasRawWindowHandle, RawWindowHandle};

use crate::{
    Event, EventStatus, MouseCursor, WindowEvent, WindowHandler, WindowInfo, WindowOpenOptions, Size,
};

use super::cursor::Cursor;
use super::keyboard::KeyboardState;
use super::view::{create_view, BASEVIEW_STATE_IVAR};

#[cfg(feature = "opengl")]
use crate::{
    gl::{GlConfig, GlContext},
    window::RawWindowHandleWrapper,
};

pub struct WindowHandle {
    raw_window_handle: Option<RawWindowHandle>,
    close_requested: Arc<AtomicBool>,
    is_open: Arc<AtomicBool>,

    // Ensure handle is !Send
    _phantom: PhantomData<*mut ()>,
}

impl WindowHandle {
    pub fn request_keyboard_focus(&mut self) {
        // TODO: not yet implemented
        //   - We can use this in the future to give the keyboard focus to a plugin's client window,
        //     so we can detect keyboard events at the client's window.
    }
    pub fn resize(&self, size: Size) {
        if let Some(window_handle) = &self.raw_window_handle {
            match window_handle {
                RawWindowHandle::AppKit(handle) => {
                    let scale_factor = unsafe {
                        let ns_window: *mut Object = msg_send![handle.ns_view as id, window];
                        let scale_factor: f64 = if ns_window.is_null() { 1.0 } else { NSWindow::backingScaleFactor(ns_window) as f64 };
                        scale_factor
                    };
                    
                    unsafe {

                        let state: &mut WindowState = WindowState::from_field(&*(handle.ns_view as *mut Object));                        

                        #[cfg(feature = "opengl")]
                        if let Some(handle) = state.window.gl_context() {
                            handle.resize(size.width, size.height);
                        }

                        let _: () = msg_send![handle.ns_view as *mut Object, setFrameSize: size];
                        let _: () = msg_send![handle.ns_view as *mut Object, setBoundsSize: size];


                        
                        let window_info = WindowInfo::from_logical_size(size, scale_factor);
                        state.trigger_event(Event::Window(WindowEvent::Resized(window_info)));
                        state.trigger_frame(); // timer events are not received during resize - so trigger frame manually
                    }
                }
                _ => { }
            }
        }
    }
    pub fn close(&mut self) {
        if let Some(window_handle) = self.raw_window_handle.take() {
            match window_handle {
                RawWindowHandle::AppKit(handle) => {
                    unsafe {
                        let ns_view = handle.ns_view as *mut Object;
                        WindowState::stop_and_free(&mut *ns_view);
                    }
                }
                _ => {}
            }
            self.close_requested.store(true, Ordering::Relaxed);
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_open.load(Ordering::Relaxed)
    }
}

unsafe impl HasRawWindowHandle for WindowHandle {
    fn raw_window_handle(&self) -> RawWindowHandle {
        if let Some(raw_window_handle) = self.raw_window_handle {
            if self.is_open.load(Ordering::Relaxed) {
                return raw_window_handle;
            }
        }

        RawWindowHandle::AppKit(AppKitHandle::empty())
    }
}

struct ParentHandle {
    _close_requested: Arc<AtomicBool>,
    is_open: Arc<AtomicBool>,
}

impl ParentHandle {
    pub fn new(raw_window_handle: RawWindowHandle) -> (Self, WindowHandle) {
        let close_requested = Arc::new(AtomicBool::new(false));
        let is_open = Arc::new(AtomicBool::new(true));

        let handle = WindowHandle {
            raw_window_handle: Some(raw_window_handle),
            close_requested: Arc::clone(&close_requested),
            is_open: Arc::clone(&is_open),
            _phantom: PhantomData::default(),
        };

        (Self { _close_requested: close_requested, is_open }, handle)
    }

    /*
    pub fn parent_did_drop(&self) -> bool {
        self.close_requested.load(Ordering::Relaxed)
    }
    */
}

impl Drop for ParentHandle {
    fn drop(&mut self) {
        self.is_open.store(false, Ordering::Relaxed);
    }
}

pub struct Window {
    /// Only set if we created the parent window, i.e. we are running in
    /// parentless mode
    ns_app: Option<id>,
    /// Only set if we created the parent window, i.e. we are running in
    /// parentless mode
    ns_window: Option<id>,
    /// Our subclassed NSView
    ns_view: id,
    close_requested: bool,

    #[cfg(feature = "opengl")]
    gl_context: Option<GlContext>,
}

impl Window {
    pub fn open_parented<P, H, B>(parent: &P, options: WindowOpenOptions, build: B) -> WindowHandle
    where
        P: HasRawWindowHandle,
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        let pool = unsafe { NSAutoreleasePool::new(nil) };

        let handle = if let RawWindowHandle::AppKit(handle) = parent.raw_window_handle() {
            handle
        } else {
            panic!("Not a macOS window");
        };

        let ns_view = unsafe { create_view(&options) };

        let window = Window {
            ns_app: None,
            ns_window: None,
            ns_view,
            close_requested: false,

            #[cfg(feature = "opengl")]
            gl_context: options
                .gl_config
                .map(|gl_config| Self::create_gl_context(None, ns_view, gl_config)),
        };

        let window_handle = Self::init(true, window, build);

        unsafe {
            let _: id = msg_send![handle.ns_view as *mut Object, addSubview: ns_view];
            let () = msg_send![ns_view as id, release];

            let () = msg_send![pool, drain];
        }

        window_handle
    }

    pub fn open_as_if_parented<H, B>(options: WindowOpenOptions, build: B) -> WindowHandle
    where
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        let pool = unsafe { NSAutoreleasePool::new(nil) };

        let ns_view = unsafe { create_view(&options) };

        let window = Window {
            ns_app: None,
            ns_window: None,
            ns_view,
            close_requested: false,

            #[cfg(feature = "opengl")]
            gl_context: options
                .gl_config
                .map(|gl_config| Self::create_gl_context(None, ns_view, gl_config)),
        };

        let window_handle = Self::init(true, window, build);

        unsafe {
            let () = msg_send![pool, drain];
        }

        window_handle
    }

    pub fn open_blocking<H, B>(options: WindowOpenOptions, build: B)
    where
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        let pool = unsafe { NSAutoreleasePool::new(nil) };

        // It seems prudent to run NSApp() here before doing other
        // work. It runs [NSApplication sharedApplication], which is
        // what is run at the very start of the Xcode-generated main
        // function of a cocoa app according to:
        // https://developer.apple.com/documentation/appkit/nsapplication
        let app = unsafe { NSApp() };

        unsafe {
            app.setActivationPolicy_(NSApplicationActivationPolicyRegular);
        }

        let scaling = 1.0; // MacOS deals with scaling on its own.

        let window_info = WindowInfo::from_logical_size(options.size, scaling);

        let rect = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(
                window_info.logical_size().width as f64,
                window_info.logical_size().height as f64,
            ),
        );

        let ns_window = unsafe {
            let ns_window = NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
                rect,
                NSWindowStyleMask::NSTitledWindowMask
                    | NSWindowStyleMask::NSClosableWindowMask
                    | NSWindowStyleMask::NSMiniaturizableWindowMask
                    | NSWindowStyleMask::NSResizableWindowMask,
                NSBackingStoreBuffered,
                NO,
            );
            ns_window.center();

            let title = NSString::alloc(nil).init_str(&options.title).autorelease();
            ns_window.setTitle_(title);

            ns_window.makeKeyAndOrderFront_(nil);

            ns_window
        };

        let ns_view = unsafe { create_view(&options) };

        let window = Window {
            ns_app: Some(app),
            ns_window: Some(ns_window),
            ns_view,
            close_requested: false,

            #[cfg(feature = "opengl")]
            gl_context: options
                .gl_config
                .map(|gl_config| Self::create_gl_context(Some(ns_window), ns_view, gl_config)),
        };

        let _ = Self::init(false, window, build);

        unsafe {
            ns_window.setContentView_(ns_view);

            let () = msg_send![ns_view as id, release];
            let () = msg_send![pool, drain];

            app.run();
        }
    }

    fn init<H, B>(parented: bool, mut window: Window, build: B) -> WindowHandle
    where
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        let window_handler = Box::new(build(&mut crate::Window::new(&mut window)));

        let (parent_handle, window_handle) = ParentHandle::new(window.raw_window_handle());
        let parent_handle = if parented { Some(parent_handle) } else { None };

        let retain_count_after_build: usize = unsafe { msg_send![window.ns_view, retainCount] };

        let window_state_ptr = Box::into_raw(Box::new(WindowState {
            window,
            window_handler,
            keyboard_state: KeyboardState::new(),
            frame_timer: None,
            retain_count_after_build,
            _parent_handle: parent_handle,
        }));

        unsafe {
            (*(*window_state_ptr).window.ns_view)
                .set_ivar(BASEVIEW_STATE_IVAR, window_state_ptr as *mut c_void);

            WindowState::setup_timer(window_state_ptr);
        }

        window_handle
    }

    pub fn resize(&self, size: Size) {
        // TODO: Implement me!
 
    }

    pub fn set_mouse_cursor(&mut self, cursor: MouseCursor) {
        let native_cursor = Cursor::from(cursor);
        unsafe {
            let bounds: NSRect = msg_send![self.ns_view as id, bounds];
            let cursor = native_cursor.load();
            let _: () = msg_send![self.ns_view as id,
                addCursorRect:bounds
                cursor:cursor
            ];
        }
    }

    pub fn close(&mut self) {
        self.close_requested = true;
    }

    #[cfg(feature = "opengl")]
    pub fn gl_context(&self) -> Option<&GlContext> {
        self.gl_context.as_ref()
    }

    #[cfg(feature = "opengl")]
    fn create_gl_context(ns_window: Option<id>, ns_view: id, config: GlConfig) -> GlContext {
        let mut handle = AppKitHandle::empty();
        handle.ns_window = ns_window.unwrap_or(ptr::null_mut()) as *mut c_void;
        handle.ns_view = ns_view as *mut c_void;
        let handle = RawWindowHandleWrapper { handle: RawWindowHandle::AppKit(handle) };

        unsafe { GlContext::create(&handle, config).expect("Could not create OpenGL context") }
    }
}

pub(super) struct WindowState {
    window: Window,
    window_handler: Box<dyn WindowHandler>,
    keyboard_state: KeyboardState,
    frame_timer: Option<CFRunLoopTimer>,
    _parent_handle: Option<ParentHandle>,
    pub retain_count_after_build: usize,
}

impl WindowState {
    /// Returns a mutable reference to a WindowState from an Objective-C field
    ///
    /// Don't use this to create two simulataneous references to a single
    /// WindowState. Apparently, macOS blocks for the duration of an event,
    /// callback, meaning that this shouldn't be a problem in practice.
    pub(super) unsafe fn from_field(obj: &Object) -> &mut Self {
        let state_ptr: *mut c_void = *obj.get_ivar(BASEVIEW_STATE_IVAR);

        &mut *(state_ptr as *mut Self)
    }

    pub(super) fn trigger_event(&mut self, event: Event) -> EventStatus {
        self.window_handler.on_event(&mut crate::Window::new(&mut self.window), event)
    }

    pub(super) fn trigger_frame(&mut self) {
        self.window_handler.on_frame(&mut crate::Window::new(&mut self.window));

        let mut do_close = false;

        /* FIXME: Is it even necessary to check if the parent dropped the handle
        // in MacOS?
        // Check if the parent handle was dropped
        if let Some(parent_handle) = &self.parent_handle {
            if parent_handle.parent_did_drop() {
                do_close = true;
                self.window.close_requested = false;
            }
        }
        */

        // Check if the user requested the window to close
        if self.window.close_requested {
            do_close = true;
            self.window.close_requested = false;
        }

        if do_close {
            unsafe {
                if let Some(ns_window) = self.window.ns_window.take() {
                    ns_window.close();
                } else {
                    // FIXME: How do we close a non-parented window? Is this even
                    // possible in a DAW host usecase?
                }
            }
        }
    }

    pub(super) fn process_native_key_event(&mut self, event: *mut Object) -> Option<KeyboardEvent> {
        self.keyboard_state.process_native_event(event)
    }

    /// Don't call until WindowState pointer is stored in view
    unsafe fn setup_timer(window_state_ptr: *mut WindowState) {
        extern "C" fn timer_callback(_: *mut __CFRunLoopTimer, window_state_ptr: *mut c_void) {
            unsafe {
                let window_state = &mut *(window_state_ptr as *mut WindowState);

                window_state.trigger_frame();
            }
        }

        let mut timer_context = CFRunLoopTimerContext {
            version: 0,
            info: window_state_ptr as *mut c_void,
            retain: None,
            release: None,
            copyDescription: None,
        };

        let timer = CFRunLoopTimer::new(0.0, 0.015, 0, 0, timer_callback, &mut timer_context);

        CFRunLoop::get_current().add_timer(&timer, kCFRunLoopDefaultMode);

        let window_state = &mut *(window_state_ptr);

        window_state.frame_timer = Some(timer);
    }

    /// Call when freeing view
    pub(super) unsafe fn stop_and_free(ns_view_obj: &mut Object) {
        let state_ptr: *mut c_void = *ns_view_obj.get_ivar(BASEVIEW_STATE_IVAR);

        // Take back ownership of Box<WindowState> so that it gets dropped
        // when it goes out of scope
        let mut window_state = Box::from_raw(state_ptr as *mut WindowState);

        if let Some(frame_timer) = window_state.frame_timer.take() {
            CFRunLoop::get_current().remove_timer(&frame_timer, kCFRunLoopDefaultMode);
        }

        // Clear ivar before triggering WindowEvent::WillClose. Otherwise, if the
        // handler of the event causes another call to release, this function could be
        // called again, leading to a double free.
        ns_view_obj.set_ivar(BASEVIEW_STATE_IVAR, ptr::null() as *const c_void);

        window_state.trigger_event(Event::Window(WindowEvent::WillClose));

        // If in non-parented mode, we want to also quit the app altogether
        if let Some(app) = window_state.window.ns_app.take() {
            app.stop_(app);
        }
    }
}

unsafe impl HasRawWindowHandle for Window {
    fn raw_window_handle(&self) -> RawWindowHandle {
        let ns_window = self.ns_window.unwrap_or(ptr::null_mut()) as *mut c_void;

        let mut handle = AppKitHandle::empty();
        handle.ns_window = ns_window;
        handle.ns_view = self.ns_view as *mut c_void;

        RawWindowHandle::AppKit(handle)
    }
}
