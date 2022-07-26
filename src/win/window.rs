use winapi::shared::guiddef::GUID;
use winapi::shared::minwindef::{ATOM, FALSE, LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HWND, RECT};
use winapi::um::combaseapi::CoCreateGuid;
use winapi::um::winuser::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetDpiForWindow, GetMessageW, GetWindowLongPtrW, LoadCursorW, PostMessageW, RegisterClassW,
    ReleaseCapture, SetCapture, SetProcessDpiAwarenessContext, SetTimer, SetWindowLongPtrW,
    SetWindowPos, TranslateMessage, UnregisterClassW, CS_OWNDC, GET_XBUTTON_WPARAM, GWLP_USERDATA,
    IDC_ARROW, MSG, SWP_NOMOVE, SWP_NOZORDER, WHEEL_DELTA, WM_CHAR, WM_CLOSE, WM_CREATE,
    WM_DPICHANGED, WM_INPUTLANGCHANGE, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_SHOWWINDOW, WM_SIZE, WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER,
    WM_USER, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSW, WS_CAPTION, WS_CHILD, WS_CLIPSIBLINGS,
    WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUPWINDOW, WS_SIZEBOX, WS_VISIBLE, XBUTTON1, XBUTTON2,
};

use std::cell::RefCell;
use std::ffi::{c_void, OsStr};
use std::marker::PhantomData;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use raw_window_handle::{HasRawWindowHandle, RawWindowHandle, Win32Handle};

const BV_WINDOW_MUST_CLOSE: UINT = WM_USER + 1;

use crate::{
    Event, MouseButton, MouseEvent, PhyPoint, PhySize, ScrollDelta, WindowEvent, WindowHandler,
    WindowInfo, WindowOpenOptions, WindowScalePolicy,
};

use super::keyboard::KeyboardState;

#[cfg(feature = "opengl")]
use crate::{gl::GlContext, window::RawWindowHandleWrapper};

unsafe fn generate_guid() -> String {
    log::warn!("generate_guid()");
    let mut guid: GUID = std::mem::zeroed();
    CoCreateGuid(&mut guid);
    format!(
        "{:0X}-{:0X}-{:0X}-{:0X}{:0X}-{:0X}{:0X}{:0X}{:0X}{:0X}{:0X}\0",
        guid.Data1,
        guid.Data2,
        guid.Data3,
        guid.Data4[0],
        guid.Data4[1],
        guid.Data4[2],
        guid.Data4[3],
        guid.Data4[4],
        guid.Data4[5],
        guid.Data4[6],
        guid.Data4[7]
    )
}

const WIN_FRAME_TIMER: usize = 4242;

pub struct WindowHandle {
    hwnd: Option<HWND>,
    is_open: Arc<AtomicBool>,

    // Ensure handle is !Send
    _phantom: PhantomData<*mut ()>,
}

impl WindowHandle {
    pub fn close(&mut self) {
        log::warn!("WindowHandle::close()");
        if let Some(hwnd) = self.hwnd.take() {
            unsafe {
                log::warn!("WindowHandle::close() -> PostMessageW");
                PostMessageW(hwnd, BV_WINDOW_MUST_CLOSE, 0, 0);
            }
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_open.load(Ordering::Relaxed)
    }
}

unsafe impl HasRawWindowHandle for WindowHandle {
    fn raw_window_handle(&self) -> RawWindowHandle {
        log::warn!("HasRawWindowHandle::raw_window_handle()");

        if let Some(hwnd) = self.hwnd {
            log::warn!("HasRawWindowHandle::raw_window_handle() -> take handle");
            let mut handle = Win32Handle::empty();
            handle.hwnd = hwnd as *mut c_void;

            RawWindowHandle::Win32(handle)
        } else {
            log::warn!("HasRawWindowHandle::raw_window_handle() -> empty handle");

            RawWindowHandle::Win32(Win32Handle::empty())
        }
    }
}

struct ParentHandle {
    is_open: Arc<AtomicBool>,
}

impl ParentHandle {
    pub fn new(hwnd: HWND) -> (Self, WindowHandle) {
        log::warn!("ParentHandle::new()");

        let is_open = Arc::new(AtomicBool::new(true));

        let handle = WindowHandle {
            hwnd: Some(hwnd),
            is_open: Arc::clone(&is_open),
            _phantom: PhantomData::default(),
        };

        (Self { is_open }, handle)
    }
}

impl Drop for ParentHandle {
    fn drop(&mut self) {
        log::warn!("ParentHandle::drop()");
        self.is_open.store(false, Ordering::Relaxed);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    log::warn!("wnd_proc()");

    if msg == WM_CREATE {
        log::warn!("wnd_proc() -> WM_CREATE -> PostMessageW");
        PostMessageW(hwnd, WM_SHOWWINDOW, 0, 0);
        return 0;
    }

    log::warn!("wnd_proc() -> GetWindowLongPtrW");
    let window_state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RefCell<WindowState>;
    if !window_state_ptr.is_null() {
        log::warn!("wnd_proc() -> window_state_ptr != null");
        match msg {
            WM_MOUSEMOVE => {
                log::warn!("WM_MOUSEMOVE: borrow_mut()");
                if let Ok(mut window_state) = (*window_state_ptr).try_borrow_mut() {
                    let mut window = window_state.create_window(hwnd);
                    let mut window = crate::Window::new(&mut window);

                    let x = (lparam & 0xFFFF) as i16 as i32;
                    let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;

                    let physical_pos = PhyPoint { x, y };

                    let logical_pos = physical_pos.to_logical(&window_state.window_info);

                    log::warn!("WM_MOUSEMOVE: enter on_event()");
                    window_state.handler.on_event(
                        &mut window,
                        Event::Mouse(MouseEvent::CursorMoved { position: logical_pos }),
                    );
                    log::warn!("WM_MOUSEMOVE: leave on_event()");
                    log::warn!("WM_MOUSEMOVE: return borrow_mut");
                } else {
                    log::warn!("WM_MOUSEWHEEL: try_bottor_mut() FAILED -> could not aquire borrow_mut!");
                }
               
                return 0;
            }
            WM_MOUSEWHEEL => {
                log::warn!("WM_MOUSEWHEEL: borrow_mut()");

                if let Ok(mut window_state) = (*window_state_ptr).try_borrow_mut() {
                    
                    let mut window = window_state.create_window(hwnd);
                    let mut window = crate::Window::new(&mut window);

                    let value = (wparam >> 16) as i16;
                    let value = value as i32;
                    let value = value as f32 / WHEEL_DELTA as f32;

                    log::warn!("WM_MOUSEWHEEL: enter on_event()");
                    window_state.handler.on_event(
                        &mut window,
                        Event::Mouse(MouseEvent::WheelScrolled(ScrollDelta::Lines {
                            x: 0.0,
                            y: value,
                        })),
                    );
                    log::warn!("WM_MOUSEWHEEL: leave on_event()");
                    log::warn!("WM_MOUSEWHEEL: return borrow_mut");

                } else {
                    log::warn!("WM_MOUSEWHEEL: try_bottor_mut() FAILED -> could not aquire borrow_mut!");
                }
               
                return 0;
            }
            WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_RBUTTONDOWN
            | WM_RBUTTONUP | WM_XBUTTONDOWN | WM_XBUTTONUP => {
                log::warn!("WM_XXBUTTONDOWN: borrow_mut()");


                if let Ok(mut window_state) = (*window_state_ptr).try_borrow_mut() {
                    
                    let mut window = window_state.create_window(hwnd);
                    let mut window = crate::Window::new(&mut window);

                    let mut mouse_button_counter = window_state.mouse_button_counter;

                    let button = match msg {
                        WM_LBUTTONDOWN | WM_LBUTTONUP => Some(MouseButton::Left),
                        WM_MBUTTONDOWN | WM_MBUTTONUP => Some(MouseButton::Middle),
                        WM_RBUTTONDOWN | WM_RBUTTONUP => Some(MouseButton::Right),
                        WM_XBUTTONDOWN | WM_XBUTTONUP => match GET_XBUTTON_WPARAM(wparam) {
                            XBUTTON1 => Some(MouseButton::Back),
                            XBUTTON2 => Some(MouseButton::Forward),
                            _ => None,
                        },
                        _ => None,
                    };

                    if let Some(button) = button {
                        let event = match msg {
                            WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN | WM_XBUTTONDOWN => {
                                // Capture the mouse cursor on button down
                                mouse_button_counter = mouse_button_counter.saturating_add(1);
                                SetCapture(hwnd);
                                MouseEvent::ButtonPressed(button)
                            }
                            WM_LBUTTONUP | WM_MBUTTONUP | WM_RBUTTONUP | WM_XBUTTONUP => {
                                // Release the mouse cursor capture when all buttons are released
                                mouse_button_counter = mouse_button_counter.saturating_sub(1);
                                if mouse_button_counter == 0 {
                                    ReleaseCapture();
                                }

                                MouseEvent::ButtonReleased(button)
                            }
                            _ => {
                                unreachable!()
                            }
                        };

                        window_state.mouse_button_counter = mouse_button_counter;

                        log::warn!("WM_XXBUTTONDOWN: enter on_event()");
                        window_state.handler.on_event(&mut window, Event::Mouse(event));
                        log::warn!("WM_XXBUTTONDOWN: leave on_event()");
                    }


                    log::warn!("WM_XXBUTTONDOWN: return borrow_mut");
                } else {
                    log::warn!("WM_XXBUTTONDOWN: try_bottor_mut() FAILED -> could not aquire borrow_mut!");
                }
               
            }
            WM_TIMER => {

                if let Ok(mut window_state) = (*window_state_ptr).try_borrow_mut() {
                    log::warn!("WM_TIMER: borrow_mut()");

                    let mut window_state = (*window_state_ptr).borrow_mut();
                    let mut window = window_state.create_window(hwnd);
                    let mut window = crate::Window::new(&mut window);

                    if wparam == WIN_FRAME_TIMER {

                        log::warn!("WM_TIMER: enter on_event()");
                        window_state.handler.on_frame(&mut window);
                        log::warn!("WM_TIMER: leave on_event()");
                    }
                    log::warn!("WM_TIMER: return borrow_mut");
                } else {
                    log::warn!("WM_TIMER: try_bottor_mut() FAILED -> could not aquire borrow_mut!");
                }

                return 0;
            }
            WM_CLOSE => {


                // Make sure to release the borrow before the DefWindowProc call
                {

                    log::warn!("WM_CLOSE: borrow_mut()");
                    if let Ok(mut window_state) = (*window_state_ptr).try_borrow_mut() {

                        let mut window = window_state.create_window(hwnd);
                        let mut window = crate::Window::new(&mut window);

                        log::warn!("WM_CLOSE: enter on_event()");
                        window_state
                            .handler
                            .on_event(&mut window, Event::Window(WindowEvent::WillClose));

                        log::warn!("WM_CLOSE: leave on_event()");
                        log::warn!("WM_CLOSE: return borrow_mut");
                    } else {
                        log::warn!("WM_CLOSE: try_bottor_mut() FAILED -> could not aquire borrow_mut!");
                    }
                }

                // DestroyWindow(hwnd);
                // return 0;
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            WM_CHAR | WM_SYSCHAR | WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP
            | WM_INPUTLANGCHANGE => {

                if let Ok(mut window_state) = (*window_state_ptr).try_borrow_mut() {

                    log::warn!("WM_CHAR_XX_KEY_UP_DOWN: borrow_mut()");


                    let mut window_state = (*window_state_ptr).borrow_mut();
                    let mut window = window_state.create_window(hwnd);
                    let mut window = crate::Window::new(&mut window);

                    log::warn!("WM_CHAR_XX_KEY_UP_DOWN: return borrow_mut");
                    log::warn!("WM_CHAR_XX_KEY_UP_DOWN: enter process_message()");
                    let opt_event =
                        window_state.keyboard_state.process_message(hwnd, msg, wparam, lparam);
                    log::warn!("WM_CHAR_XX_KEY_UP_DOWN: leave process_message()");


                    log::warn!("WM_CHAR_XX_KEY_UP_DOWN: enter on_event()");
                    if let Some(event) = opt_event {
                        window_state.handler.on_event(&mut window, Event::Keyboard(event));
                    }
                    log::warn!("WM_CHAR_XX_KEY_UP_DOWN: leave on_event()");
                    log::warn!("WM_CHAR_XX_KEY_UP_DOWN: return borrow_mut");

                    if msg != WM_SYSKEYDOWN {
                        log::warn!("WM_CHAR_XX_KEY_UP_DOWN: return 0 (skip default handler)");
                        return 0;
                    }
                } else {
                    log::warn!("WM_CHAR_XX_KEY_UP_DOWN: try_bottow_mut() FAILED -> could not aquire borrow_mut!");
                }

            }
            WM_SIZE => {
                log::warn!("WM_SIZE: borrow_mut()");


                
                if let Ok(mut window_state) = (*window_state_ptr).try_borrow_mut() {

                    let mut window = window_state.create_window(hwnd);
                    let mut window = crate::Window::new(&mut window);

                    let width = (lparam & 0xFFFF) as u16 as u32;
                    let height = ((lparam >> 16) & 0xFFFF) as u16 as u32;

                    window_state.window_info = WindowInfo::from_physical_size(
                        PhySize { width, height },
                        window_state.window_info.scale(),
                    );

                    let window_info = window_state.window_info;

                    log::warn!("WM_SIZE: enter on_event()");
                    window_state
                        .handler
                        .on_event(&mut window, Event::Window(WindowEvent::Resized(window_info)));

                    log::warn!("WM_SIZE: leave on_event()");
                    log::warn!("WM_SIZE: return borrow_mut");
                } else {
                    log::warn!("WM_SIZE: try_bottow_mut() FAILED -> could not aquire borrow_mut!");
                }

            }
            WM_DPICHANGED => {
                log::warn!("WM_DPICHANGED: borrow_mut()");


                
                if let Ok(mut window_state) = (*window_state_ptr).try_borrow_mut() {


                    // To avoid weirdness with the realtime borrow checker.
                    let new_rect = {
                        if let WindowScalePolicy::SystemScaleFactor = window_state.scale_policy {
                            let dpi = (wparam & 0xFFFF) as u16 as u32;
                            let scale_factor = dpi as f64 / 96.0;

                            window_state.window_info = WindowInfo::from_logical_size(
                                window_state.window_info.logical_size(),
                                scale_factor,
                            );

                            Some((
                                RECT {
                                    left: 0,
                                    top: 0,
                                    // todo: check if usize fits into i32
                                    right: window_state.window_info.physical_size().width as i32,
                                    bottom: window_state.window_info.physical_size().height as i32,
                                },
                                window_state.dw_style,
                            ))
                        } else {
                            None
                        }
                    };
                    if let Some((mut new_rect, dw_style)) = new_rect {

                        log::warn!("WM_DPICHANGED: enter AdjustWindowRectEx()");
                        // Convert this desired "client rectangle" size to the actual "window rectangle"
                        // size (Because of course you have to do that).
                        AdjustWindowRectEx(&mut new_rect, dw_style, 0, 0);

                        log::warn!("WM_DPICHANGED: leave AdjustWindowRectEx()");

                        log::warn!("WM_DPICHANGED: enter SetWindowPos()");
                        // Windows makes us resize the window manually. This will trigger another `WM_SIZE` event,
                        // which we can then send the user the new scale factor.
                        SetWindowPos(
                            hwnd,
                            hwnd,
                            new_rect.left as i32,
                            new_rect.top as i32,
                            new_rect.right - new_rect.left,
                            new_rect.bottom - new_rect.top,
                            SWP_NOZORDER | SWP_NOMOVE,
                        );
                        log::warn!("WM_DPICHANGED: leave SetWindowPos()");
                    }
                    log::warn!("WM_DPICHANGED: return borrow_mut");
                } else {
                    log::warn!("WM_DPICHANGED: try_bottow_mut() FAILED -> could not aquire borrow_mut!");
                }

            }
            WM_NCDESTROY => {
                log::warn!("WM_NCDESTROY: borrow_mut()");


                // let window_state = Box::from_raw(window_state_ptr);


                if let Ok(window_state) = (*window_state_ptr).try_borrow() {

                    log::warn!("WM_NCDESTROY: enter unregister_wnd_class()");
                    // unregister_wnd_class(window_state.borrow().window_class);
                    unregister_wnd_class(window_state.window_class);
                    log::warn!("WM_NCDESTROY: leave unregister_wnd_class()");


                    log::warn!("WM_NCDESTROY: enter on_event()");
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    log::warn!("WM_NCDESTROY: leave SetWindowLongPtrW()");

                    log::warn!("WM_NCDESTROY: return borrow_mut");
                } else {
                    log::warn!("WM_CHAR_XX_KEY_UP_DOWN: try_bottow_mut() FAILED -> could not aquire borrow_mut!");
                }


            }
            _ => {
                log::warn!("WC_??? another message {:?}", msg);

                if msg == BV_WINDOW_MUST_CLOSE {
                    log::warn!("ELSE BRANCH: BV_WINDOW_MUST_CLOSE -> enter DestroyWindow()");
                    DestroyWindow(hwnd);
                    log::warn!("ELSE BRANCH: BV_WINDOW_MUST_CLOSE -> leave DestroyWindow() -> skip default handler");
                    return 0;
                }
            }
        }
    }
    log::warn!("wnd_proc done -> Call DefWindowProcW (default handler)");

    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn register_wnd_class() -> ATOM {
    log::warn!("register_wnd_class");

    // We generate a unique name for the new window class to prevent name collisions
    let class_name_str = format!("Baseview-{}", generate_guid());
    let mut class_name: Vec<u16> = OsStr::new(&class_name_str).encode_wide().collect();
    class_name.push(0);

    let wnd_class = WNDCLASSW {
        style: CS_OWNDC,
        lpfnWndProc: Some(wnd_proc),
        hInstance: null_mut(),
        lpszClassName: class_name.as_ptr(),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hIcon: null_mut(),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: null_mut(),
        lpszMenuName: null_mut(),
    };

    RegisterClassW(&wnd_class)
}

unsafe fn unregister_wnd_class(wnd_class: ATOM) {
    log::warn!("unregister_wnd_class");
    UnregisterClassW(wnd_class as _, null_mut());
}

struct WindowState {
    window_class: ATOM,
    window_info: WindowInfo,
    _parent_handle: Option<ParentHandle>,
    keyboard_state: KeyboardState,
    mouse_button_counter: usize,
    handler: Box<dyn WindowHandler>,
    scale_policy: WindowScalePolicy,
    dw_style: u32,

    #[cfg(feature = "opengl")]
    gl_context: Arc<Option<GlContext>>,
}

impl WindowState {
    #[cfg(not(feature = "opengl"))]
    fn create_window(&self, hwnd: HWND) -> Window {
        Window { hwnd }
    }

    #[cfg(feature = "opengl")]
    fn create_window(&self, hwnd: HWND) -> Window {
        Window { hwnd, gl_context: self.gl_context.clone() }
    }
}

pub struct Window {
    hwnd: HWND,

    #[cfg(feature = "opengl")]
    gl_context: Arc<Option<GlContext>>,
}

impl Window {
    pub fn open_parented<P, H, B>(parent: &P, options: WindowOpenOptions, build: B) -> WindowHandle
    where
        P: HasRawWindowHandle,
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        log::warn!("open_parented");
        let parent = match parent.raw_window_handle() {
            RawWindowHandle::Win32(h) => h.hwnd as HWND,
            h => panic!("unsupported parent handle {:?}", h),
        };

        let (window_handle, _) = Self::open(true, parent, options, build);

        window_handle
    }

    pub fn open_as_if_parented<H, B>(options: WindowOpenOptions, build: B) -> WindowHandle
    where
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        log::warn!("open_as_if_parented");
        let (window_handle, _) = Self::open(true, null_mut(), options, build);

        window_handle
    }

    pub fn open_blocking<H, B>(options: WindowOpenOptions, build: B)
    where
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        log::warn!("open_blocking");

        let (_, hwnd) = Self::open(false, null_mut(), options, build);

        unsafe {
            let mut msg: MSG = std::mem::zeroed();

            loop {
                log::warn!("open_blocking: GetMessageW");

                let status = GetMessageW(&mut msg, hwnd, 0, 0);

                if status == -1 {
                    break;
                }
                log::warn!("open_blocking: TranslateMessage");

                TranslateMessage(&msg);
                log::warn!("open_blocking: DispatchMessageW");
                DispatchMessageW(&msg);
            }
        }
    }

    fn open<H, B>(
        parented: bool, parent: HWND, options: WindowOpenOptions, build: B,
    ) -> (WindowHandle, HWND)
    where
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        log::warn!("open()");

        unsafe {
            let mut title: Vec<u16> = OsStr::new(&options.title[..]).encode_wide().collect();
            title.push(0);
            log::warn!("open() -> register_wnd_class");

            let window_class = register_wnd_class();
            // todo: manage error ^

            let scaling = match options.scale {
                WindowScalePolicy::SystemScaleFactor => 1.0,
                WindowScalePolicy::ScaleFactor(scale) => scale,
            };

            log::warn!("open() -> WindowInfo::from_logical_size()");
            let window_info = WindowInfo::from_logical_size(options.size, scaling);

            let mut rect = RECT {
                left: 0,
                top: 0,
                // todo: check if usize fits into i32
                right: window_info.physical_size().width as i32,
                bottom: window_info.physical_size().height as i32,
            };

            let flags = if parented {
                WS_CHILD | WS_VISIBLE
            } else {
                WS_POPUPWINDOW
                    | WS_CAPTION
                    | WS_VISIBLE
                    | WS_SIZEBOX
                    | WS_MINIMIZEBOX
                    | WS_MAXIMIZEBOX
                    | WS_CLIPSIBLINGS
            };

            if !parented {
                log::warn!("open() -> AdjustWindowRectEx");

                AdjustWindowRectEx(&mut rect, flags, FALSE, 0);
            }

            log::warn!("open() -> CreateWindowExW");
            let hwnd = CreateWindowExW(
                0,
                window_class as _,
                title.as_ptr(),
                flags,
                0,
                0,
                rect.right - rect.left,
                rect.bottom - rect.top,
                parent as *mut _,
                null_mut(),
                null_mut(),
                null_mut(),
            );
            // todo: manage error ^

            #[cfg(feature = "opengl")]
            let gl_context: Arc<Option<GlContext>> = Arc::new(options.gl_config.map(|gl_config| {
                log::warn!("open() -> GL Context closure 1 -> GlContext::create()");
                let mut handle = Win32Handle::empty();
                handle.hwnd = hwnd as *mut c_void;
                let handle = RawWindowHandleWrapper { handle: RawWindowHandle::Win32(handle) };

                GlContext::create(&handle, gl_config).expect("Could not create OpenGL context")
            }));

            #[cfg(not(feature = "opengl"))]
            let handler = Box::new(build(&mut crate::Window::new(&mut Window { hwnd })));
            #[cfg(feature = "opengl")]
            let handler = Box::new(build(&mut crate::Window::new(&mut Window {
                hwnd,
                gl_context: gl_context.clone(),
            })));

            log::warn!("open() -> ParentHandle::new()");
            let (parent_handle, window_handle) = ParentHandle::new(hwnd);
            let parent_handle = if parented { Some(parent_handle) } else { None };

            let mut window_state = Box::new(RefCell::new(WindowState {
                window_class,
                window_info,
                _parent_handle: parent_handle,
                keyboard_state: KeyboardState::new(),
                mouse_button_counter: 0,
                handler,
                scale_policy: options.scale,
                dw_style: flags,

                #[cfg(feature = "opengl")]
                gl_context,
            }));

            log::warn!("open() -> SetProcessDpiAwarenessContext");
            // Only works on Windows 10 unfortunately.
            SetProcessDpiAwarenessContext(
                winapi::shared::windef::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE,
            );

            // Now we can get the actual dpi of the window.
            let new_rect = if let WindowScalePolicy::SystemScaleFactor = options.scale {
                // Only works on Windows 10 unfortunately.
                log::warn!("open() -> GetDpiForWindow");
                let dpi = GetDpiForWindow(hwnd);
                let scale_factor = dpi as f64 / 96.0;

                let mut window_state = window_state.get_mut();
                if window_state.window_info.scale() != scale_factor {
                    window_state.window_info = WindowInfo::from_logical_size(
                        window_state.window_info.logical_size(),
                        scale_factor,
                    );

                    Some(RECT {
                        left: 0,
                        top: 0,
                        // todo: check if usize fits into i32
                        right: window_state.window_info.physical_size().width as i32,
                        bottom: window_state.window_info.physical_size().height as i32,
                    })
                } else {
                    None
                }
            } else {
                None
            };

            log::warn!("open() -> SetWindowLongPtrW");
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(window_state) as *const _ as _);


            log::warn!("open() -> SetTimer");
            SetTimer(hwnd, WIN_FRAME_TIMER, 15, None);

            if let Some(mut new_rect) = new_rect {
                // Convert this desired"client rectangle" size to the actual "window rectangle"
                // size (Because of course you have to do that).
                log::warn!("open() -> AdjustWindowRectEx");
                AdjustWindowRectEx(&mut new_rect, flags, 0, 0);

                // Windows makes us resize the window manually. This will trigger another `WM_SIZE` event,
                // which we can then send the user the new scale factor.
                log::warn!("open() -> SetWindowPos");
                SetWindowPos(
                    hwnd,
                    hwnd,
                    new_rect.left as i32,
                    new_rect.top as i32,
                    new_rect.right - new_rect.left,
                    new_rect.bottom - new_rect.top,
                    SWP_NOZORDER | SWP_NOMOVE,
                );
            }

            (window_handle, hwnd)
        }
    }

    pub fn close(&mut self) {
        unsafe {
            log::warn!("close() -> PostMessageW");
            PostMessageW(self.hwnd, BV_WINDOW_MUST_CLOSE, 0, 0);
        }
    }

    #[cfg(feature = "opengl")]
    pub fn gl_context(&self) -> Option<&GlContext> {
        self.gl_context.as_ref().as_ref()
    }
}

unsafe impl HasRawWindowHandle for Window {
    fn raw_window_handle(&self) -> RawWindowHandle {
        log::warn!("HasRawWindowHandle::raw_window_handle()");
        let mut handle = Win32Handle::empty();
        handle.hwnd = self.hwnd as *mut c_void;
        RawWindowHandle::Win32(handle)
    }
}
