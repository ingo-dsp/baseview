use std::marker::PhantomData;

use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

use crate::event::{Event, EventStatus};
use crate::window_open_options::WindowOpenOptions;
use crate::Size;
use crate::MouseCursor;

#[cfg(target_os = "macos")]
use crate::macos as platform;
#[cfg(target_os = "windows")]
use crate::win as platform;
#[cfg(target_os = "linux")]
use crate::x11 as platform;

pub struct WindowHandle {
    window_handle: platform::WindowHandle,
    // so that WindowHandle is !Send on all platforms
    phantom: PhantomData<*mut ()>,
}

/// Quick wrapper to satisfy [HasRawWindowHandle], because of course a raw window handle wouldn't
/// have a raw window handle, that would be silly.
pub(crate) struct RawWindowHandleWrapper {
    pub handle: RawWindowHandle,
}

impl WindowHandle {
    fn new(window_handle: platform::WindowHandle) -> Self {
        Self { window_handle, phantom: PhantomData::default() }
    }

    pub fn request_keyboard_focus(&mut self) {
        self.window_handle.request_keyboard_focus();
    }

    pub fn resize(&self, size: Size) {
        self.window_handle.resize(size);
    }

    /// Close the window
    pub fn close(&mut self) {
        self.window_handle.close();
    }

    /// Returns `true` if the window is still open, and returns `false`
    /// if the window was closed/dropped.
    pub fn is_open(&self) -> bool {
        self.window_handle.is_open()
    }
}

unsafe impl HasRawWindowHandle for WindowHandle {
    fn raw_window_handle(&self) -> RawWindowHandle {
        self.window_handle.raw_window_handle()
    }
}

pub trait WindowHandler {
    fn on_frame(&mut self, window: &mut Window);
    fn on_event(&mut self, window: &mut Window, event: Event) -> EventStatus;
}

pub struct Window<'a> {
    window: &'a mut platform::Window,
    // so that Window is !Send on all platforms
    phantom: PhantomData<*mut ()>,
}

impl<'a> Window<'a> {
    pub(crate) fn new(window: &mut platform::Window) -> Window {
        Window { window, phantom: PhantomData }
    }

    pub fn open_parented<P, H, B>(parent: &P, options: WindowOpenOptions, build: B) -> WindowHandle
    where
        P: HasRawWindowHandle,
        H: WindowHandler + 'static,
        B: FnOnce(&mut Window) -> H,
        B: Send + 'static,
    {
        let window_handle = platform::Window::open_parented::<P, H, B>(parent, options, build);
        WindowHandle::new(window_handle)
    }

    pub fn open_as_if_parented<H, B>(options: WindowOpenOptions, build: B) -> WindowHandle
    where
        H: WindowHandler + 'static,
        B: FnOnce(&mut Window) -> H,
        B: Send + 'static,
    {
        let window_handle = platform::Window::open_as_if_parented::<H, B>(options, build);
        WindowHandle::new(window_handle)
    }

    pub fn open_blocking<H, B>(options: WindowOpenOptions, build: B)
    where
        H: WindowHandler + 'static,
        B: FnOnce(&mut Window) -> H,
        B: Send + 'static,
    {
        platform::Window::open_blocking::<H, B>(options, build)
    }

    /// Close the window
    pub fn close(&mut self) {
        self.window.close();
    }

    /// Resize the window to the given size.
    ///
    /// # TODO
    ///
    /// This is currently only supported on Linux.
    #[cfg(target_os = "linux")]
    pub fn resize(&mut self, size: Size) {
        self.window.resize(size);
    }

    /// Set the cursor to the given cursor type
    pub fn set_mouse_cursor(&mut self, cursor: MouseCursor) {
        self.window.set_mouse_cursor(cursor);
    }


    /// If provided, then an OpenGL context will be created for this window. You'll be able to
    /// access this context through [crate::Window::gl_context].
    #[cfg(feature = "opengl")]
    pub fn gl_context(&self) -> Option<&crate::gl::GlContext> {
        self.window.gl_context()
    }
}

unsafe impl<'a> HasRawWindowHandle for Window<'a> {
    fn raw_window_handle(&self) -> RawWindowHandle {
        self.window.raw_window_handle()
    }
}

unsafe impl HasRawWindowHandle for RawWindowHandleWrapper {
    fn raw_window_handle(&self) -> RawWindowHandle {
        self.handle
    }
}
