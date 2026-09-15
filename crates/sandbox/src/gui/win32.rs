//! Win32 top-level windows (T-901, ADR-008 §7): an `HWND` whose **owner** is the editor's main
//! window (the handle travels over the control channel; cross-process ownership is allowed), so
//! the plugin window stays above it and minimizes with it. Compile-checked (`just check-cross`);
//! not run on the development machine.

use std::cell::Cell;
use std::ptr::null;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, BringWindowToTop, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW,
    DestroyWindow, DispatchMessageW, IDC_ARROW, LoadCursorW, MSG, PM_REMOVE, PeekMessageW,
    RegisterClassW, SW_SHOWNORMAL, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SetForegroundWindow,
    SetWindowPos, ShowWindow, TranslateMessage, WM_CLOSE, WM_SIZE, WNDCLASSW, WS_CAPTION,
    WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_THICKFRAME,
};

use super::{Backend, WindowEvent, WindowSpec};

thread_local! {
    static CLOSE: Cell<bool> = const { Cell::new(false) };
    static SIZE: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

const CLASS: &str = "PowerVoicePluginWindow";

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CLOSE => {
            CLOSE.with(|c| c.set(true));
            0
        }
        WM_SIZE => {
            let w = (lparam as usize & 0xffff) as u32;
            let h = ((lparam as usize >> 16) & 0xffff) as u32;
            SIZE.with(|s| s.set(Some((w, h))));
            0
        }
        // SAFETY: the default procedure for our own window's other messages.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub(super) struct Win32 {
    hwnd: HWND,
    style: u32,
    size: (u32, u32),
}

impl Win32 {
    pub(super) fn new() -> Self {
        Self {
            hwnd: std::ptr::null_mut(),
            style: 0,
            size: (0, 0),
        }
    }

    /// The outer size of a window of `style` with a `width`×`height` client area.
    fn outer(style: u32, width: u32, height: u32) -> (i32, i32) {
        let mut r = RECT {
            left: 0,
            top: 0,
            right: width.min(16_384) as i32,
            bottom: height.min(16_384) as i32,
        };
        // SAFETY: a valid RECT for the call.
        unsafe { AdjustWindowRectEx(&mut r, style, 0, 0) };
        (r.right - r.left, r.bottom - r.top)
    }
}

impl Backend for Win32 {
    fn create(&mut self, spec: &WindowSpec<'_>) -> Result<u64, String> {
        // SAFETY: Win32 calls on the main thread; every buffer outlives its call.
        unsafe {
            let instance = GetModuleHandleW(null());
            let class = wide(CLASS);
            if !REGISTERED.with(Cell::get) {
                let wc = WNDCLASSW {
                    style: 0,
                    lpfnWndProc: Some(wndproc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: instance,
                    hIcon: std::ptr::null_mut(),
                    hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                    hbrBackground: std::ptr::null_mut(),
                    lpszMenuName: null(),
                    lpszClassName: class.as_ptr(),
                };
                if RegisterClassW(&wc) == 0 {
                    return Err("couldn't register the plugin window class".into());
                }
                REGISTERED.with(|r| r.set(true));
            }
            let mut style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
            if spec.resizable {
                style |= WS_THICKFRAME | WS_MAXIMIZEBOX;
            }
            let (w, h) = Self::outer(style, spec.width, spec.height);
            let title = wide(spec.title);
            let owner = spec
                .transient_for
                .map_or(std::ptr::null_mut(), |p| p as HWND);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                w,
                h,
                owner,
                std::ptr::null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                return Err("couldn't create the plugin window".into());
            }
            CLOSE.with(|c| c.set(false));
            self.hwnd = hwnd;
            self.style = style;
            self.size = (spec.width, spec.height);
            Ok(hwnd as u64)
        }
    }

    fn show(&mut self) {
        if !self.hwnd.is_null() {
            // SAFETY: our live window.
            unsafe {
                ShowWindow(self.hwnd, SW_SHOWNORMAL);
                SetForegroundWindow(self.hwnd);
            }
        }
    }

    fn raise(&mut self) {
        if !self.hwnd.is_null() {
            // SAFETY: as above.
            unsafe {
                BringWindowToTop(self.hwnd);
                SetForegroundWindow(self.hwnd);
            }
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        if !self.hwnd.is_null() {
            self.size = (width, height);
            let (w, h) = Self::outer(self.style, width, height);
            // SAFETY: as above.
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    w,
                    h,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }

    fn destroy(&mut self) {
        if !self.hwnd.is_null() {
            // SAFETY: our live window, destroyed once.
            unsafe { DestroyWindow(self.hwnd) };
            self.hwnd = std::ptr::null_mut();
        }
    }

    fn fd(&self) -> Option<i32> {
        None
    }

    fn pump(&mut self, out: &mut Vec<WindowEvent>) {
        // SAFETY: the thread's message queue; `msg` is written by `PeekMessageW`.
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if CLOSE.with(|c| c.replace(false)) && !self.hwnd.is_null() {
            out.push(WindowEvent::CloseRequested);
        }
        if let Some(size) = SIZE.with(Cell::take)
            && size != self.size
            && size.0 > 0
            && size.1 > 0
        {
            self.size = size;
            out.push(WindowEvent::Resized(size.0, size.1));
        }
    }
}

impl Drop for Win32 {
    fn drop(&mut self) {
        self.destroy();
    }
}
