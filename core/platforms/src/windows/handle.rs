use std::{cell::Cell, ffi::OsString, os::windows::ffi::OsStringExt, ptr, str};

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM},
        Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
        UI::WindowsAndMessaging::{
            EnumWindows, GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetWindowLongPtrW, GetWindowTextW,
            IsWindowVisible, WS_DISABLED, WS_EX_TOOLWINDOW,
        },
    },
    core::BOOL,
};

#[derive(Clone, Debug)]
pub struct HandleCell {
    inner: Handle,
    inner_cell: Cell<Option<HWND>>,
}

impl HandleCell {
    pub fn new(handle: Handle) -> Self {
        Self {
            inner: handle,
            inner_cell: Cell::new(None),
        }
    }

    #[inline]
    pub fn as_inner(&self) -> Option<HWND> {
        match self.inner.kind {
            HandleKind::Fixed(handle) => Some(handle),
            HandleKind::Dynamic(class) => {
                if self.inner_cell.get().is_none() {
                    self.inner_cell.set(query_handle(class));
                }

                let handle = self.inner_cell.get()?;
                if is_class_matched(handle, class) {
                    Some(handle)
                } else {
                    self.inner_cell.set(None);
                    None
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleKind {
    Fixed(HWND),
    Dynamic(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle {
    kind: HandleKind,
}

impl Handle {
    pub fn new(kind: HandleKind) -> Self {
        Self { kind }
    }

    pub fn as_inner(&self) -> Option<HWND> {
        match self.kind {
            HandleKind::Fixed(handle) => Some(handle),
            HandleKind::Dynamic(class) => query_handle(class),
        }
    }
}

pub fn query_capture_name_handle_pairs() -> Vec<(String, Handle)> {
    unsafe extern "system" fn callback(handle: HWND, params: LPARAM) -> BOOL {
        if !unsafe { IsWindowVisible(handle) }.as_bool() {
            return true.into();
        }

        let mut cloaked = 0u32;
        let _ = unsafe {
            DwmGetWindowAttribute(
                handle,
                DWMWA_CLOAKED,
                (&raw mut cloaked).cast(),
                std::mem::size_of::<u32>() as u32,
            )
        };
        if cloaked != 0 {
            return true.into();
        }

        let style = unsafe { GetWindowLongPtrW(handle, GWL_STYLE) } as u32;
        let ex_style = unsafe { GetWindowLongPtrW(handle, GWL_EXSTYLE) } as u32;
        if style & WS_DISABLED.0 != 0 || ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return true.into();
        }

        // TODO: Windows maximum title length is 256 but can this overflow?
        let mut buf = [0u16; 256];
        let count = unsafe { GetWindowTextW(handle, &mut buf) } as usize;
        if count == 0 {
            return true.into();
        }

        let vec = unsafe { &mut *(params.0 as *mut Vec<(String, Handle)>) };
        if let Some(name) = OsString::from_wide(&buf[..count]).to_str() {
            vec.push((name.to_string(), Handle::new(HandleKind::Fixed(handle))));
        }
        true.into()
    }

    let mut vec = Vec::new();
    let _ = unsafe { EnumWindows(Some(callback), LPARAM(&raw mut vec as isize)) };
    vec
}

#[inline]
fn query_handle(class: &'static str) -> Option<HWND> {
    struct Params {
        class: &'static str,
        handle_out: *mut HWND,
    }

    unsafe extern "system" fn callback(handle: HWND, params: LPARAM) -> BOOL {
        let params = unsafe { ptr::read::<Params>(params.0 as *const _) };
        if is_class_matched(handle, params.class) {
            unsafe { ptr::write(params.handle_out, handle) };
            false.into()
        } else {
            true.into()
        }
    }

    let mut handle = HWND::default();
    let params = Params {
        class,
        handle_out: &raw mut handle,
    };
    let _ = unsafe { EnumWindows(Some(callback), LPARAM(&raw const params as isize)) };

    if handle.is_invalid() {
        None
    } else {
        Some(handle)
    }
}

#[inline]
fn is_class_matched(handle: HWND, class: &'static str) -> bool {
    // TODO: Windows maximum title length is 256 but can this overflow?
    let mut buf = [0u16; 256];
    let count = unsafe { GetClassNameW(handle, &mut buf) as usize };
    if count == 0 {
        return false;
    }

    let class_name_string = OsString::from_wide(&buf[..count])
        .to_string_lossy()
        .into_owned();

    println!("Class name for handle {:?} is {}", handle, class_name_string);

    class_name_string.starts_with(class)
}
