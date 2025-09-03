use std::{
    cell::RefCell,
    mem::{self, size_of},
    sync::LazyLock,
    thread,
    time::Duration,
};

use bit_vec::BitVec;
use tokio::sync::broadcast::{self, Receiver, Sender};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{
            ClientToScreen, GetMonitorInfoW, IntersectRect, MONITOR_DEFAULTTONULL, MONITORINFO,
            MonitorFromWindow,
        },
        System::Threading::GetCurrentProcessId,
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS,
                KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC_EX,
                MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
                MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT,
                MapVirtualKeyW, SendInput, VIRTUAL_KEY, VK_0, VK_1, VK_2, VK_3, VK_4, VK_5, VK_6,
                VK_7, VK_8, VK_9, VK_A, VK_B, VK_C, VK_CONTROL, VK_D, VK_DELETE, VK_DOWN, VK_E,
                VK_END, VK_ESCAPE, VK_F, VK_F1, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8,
                VK_F9, VK_F10, VK_F11, VK_F12, VK_G, VK_H, VK_HOME, VK_I, VK_INSERT, VK_J, VK_K,
                VK_L, VK_LEFT, VK_M, VK_MENU, VK_N, VK_NEXT, VK_O, VK_OEM_1, VK_OEM_2, VK_OEM_3,
                VK_OEM_7, VK_OEM_COMMA, VK_OEM_PERIOD, VK_P, VK_PRIOR, VK_Q, VK_R, VK_RETURN,
                VK_RIGHT, VK_S, VK_SHIFT, VK_SPACE, VK_T, VK_U, VK_UP, VK_V, VK_W, VK_X, VK_Y,
                VK_Z,
            },
            WindowsAndMessaging::{
                CallNextHookEx, GetForegroundWindow, GetSystemMetrics, GetWindowRect,
                GetWindowThreadProcessId, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED,
                LLKHF_LOWER_IL_INJECTED, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
                SM_YVIRTUALSCREEN, SetWindowsHookExW, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
            },
        },
    },
    core::Owned,
};

use super::{HandleCell, handle::Handle};
use crate::{
    ConvertedCoordinates, Error, Result,
    input::{InputKind, KeyKind, KeyState, MouseKind},
};

static KEY_CHANNEL: LazyLock<Sender<KeyKind>> = LazyLock::new(|| broadcast::channel(1).0);
static PROCESS_ID: LazyLock<u32> = LazyLock::new(|| unsafe { GetCurrentProcessId() });

pub fn init() -> Owned<HHOOK> {
    unsafe extern "system" fn keyboard_ll(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let msg = wparam.0 as u32;
        if code as u32 == HC_ACTION && (msg == WM_KEYUP || msg == WM_KEYDOWN) {
            let lparam_ptr = lparam.0 as *mut KBDLLHOOKSTRUCT;
            let mut key = unsafe { lparam_ptr.read() };
            let vkey = unsafe { mem::transmute::<u16, VIRTUAL_KEY>(key.vkCode as u16) };
            let key_kind = KeyKind::try_from(vkey);
            let ignore = key.dwExtraInfo == *PROCESS_ID as usize;
            if !ignore
                && msg == WM_KEYUP
                && let Ok(key) = key_kind
            {
                let _ = KEY_CHANNEL.send(key);
            } else if ignore {
                // Won't work if the hook is not on the top of the chain
                key.flags &= !LLKHF_INJECTED;
                key.flags &= !LLKHF_LOWER_IL_INJECTED;
                unsafe {
                    *lparam_ptr = key;
                }
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }
    unsafe { Owned::new(SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_ll), None, 0).unwrap()) }
}

#[derive(Debug)]
pub struct WindowsInputReceiver {
    handle: HandleCell,
    input_kind: InputKind,
    rx: Receiver<KeyKind>,
}

impl WindowsInputReceiver {
    pub fn new(handle: Handle, input_kind: InputKind) -> Self {
        Self {
            handle: HandleCell::new(handle),
            input_kind,
            rx: KEY_CHANNEL.subscribe(),
        }
    }

    pub fn try_recv(&mut self) -> Option<KeyKind> {
        self.rx
            .try_recv()
            .ok()
            .and_then(|key| self.can_process_key().then_some(key))
    }

    // TODO: Is this good?
    fn can_process_key(&self) -> bool {
        let fg = unsafe { GetForegroundWindow() };
        let mut fg_pid = 0;
        unsafe { GetWindowThreadProcessId(fg, Some(&raw mut fg_pid)) };
        if fg_pid == *PROCESS_ID {
            return true;
        }

        self.handle
            .as_inner()
            .map(|handle| is_foreground(handle, self.input_kind))
            .unwrap_or_default()
    }
}

#[derive(Debug)]
pub struct WindowsInput {
    handle: HandleCell,
    input_kind: InputKind,
    key_down: RefCell<BitVec>,
}

impl WindowsInput {
    pub fn new(handle: Handle, kind: InputKind) -> Self {
        Self {
            handle: HandleCell::new(handle),
            input_kind: kind,
            key_down: RefCell::new(BitVec::from_elem(256, false)),
        }
    }

    pub fn send_mouse(&self, x: i32, y: i32, kind: MouseKind) -> Result<()> {
        #[inline]
        fn mouse_input(dx: i32, dy: i32, flags: MOUSE_EVENT_FLAGS, data: i32) -> [INPUT; 1] {
            [INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx,
                        dy,
                        dwFlags: flags,
                        mouseData: data as u32,
                        ..MOUSEINPUT::default()
                    },
                },
            }]
        }

        let mut handle = self.get_handle()?;
        if !is_foreground(handle, self.input_kind) {
            return Err(Error::WindowNotFound);
        }
        if matches!(self.input_kind, InputKind::Foreground) {
            handle = unsafe { GetForegroundWindow() };
        }

        let (dx, dy) = client_to_absolute_coordinate_raw(handle, x, y)?;
        let base_flags = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE | MOUSEEVENTF_VIRTUALDESK;

        match kind {
            MouseKind::Move => send_input(mouse_input(dx, dy, base_flags, 0)),
            MouseKind::Click => {
                send_input(mouse_input(dx, dy, base_flags | MOUSEEVENTF_LEFTDOWN, 0))?;
                // TODO: Hack or double-click won't work...
                thread::sleep(Duration::from_millis(80));
                send_input(mouse_input(dx, dy, base_flags | MOUSEEVENTF_LEFTUP, 0))
            }
            MouseKind::Scroll => {
                send_input(mouse_input(dx, dy, base_flags | MOUSEEVENTF_WHEEL, -300))
            }
        }
    }

    pub fn key_state(&self, kind: KeyKind) -> Result<KeyState> {
        let result = unsafe { GetAsyncKeyState(VIRTUAL_KEY::from(kind).0 as i32) } as u16;
        let is_down = result & 0x8000 != 0;
        let state = if is_down {
            KeyState::Pressed
        } else {
            KeyState::Released
        };

        Ok(state)
    }

    pub fn send_key(&self, kind: KeyKind) -> Result<()> {
        self.send_key_down(kind)?;
        self.send_key_up(kind)?;
        Ok(())
    }

    pub fn send_key_up(&self, kind: KeyKind) -> Result<()> {
        self.send_input(kind, false)
    }

    pub fn send_key_down(&self, kind: KeyKind) -> Result<()> {
        self.send_input(kind, true)
    }

    #[inline]
    fn send_input(&self, kind: KeyKind, is_down: bool) -> Result<()> {
        let handle = self.get_handle()?;
        if is_down && !is_foreground(handle, self.input_kind) {
            return Err(Error::KeyNotSent);
        }
        let key = kind.into();
        let (scan_code, is_extended) = to_scan_code(key);
        let mut key_down = self.key_down.borrow_mut();
        // SAFETY: VIRTUAL_KEY is from range 0..254 (inclusive) and BitVec
        // was initialized with 256 elements
        let was_key_down = unsafe { key_down.get_unchecked(key.0 as usize) };
        match (is_down, was_key_down) {
            (true, true) | (false, false) => return Err(Error::KeyNotSent),
            _ => {
                key_down.set(key.0 as usize, is_down);
            }
        }
        send_input(to_input(key, scan_code, is_extended, is_down))
    }

    #[inline]
    fn get_handle(&self) -> Result<HWND> {
        self.handle.as_inner().ok_or(Error::WindowNotFound)
    }
}

impl TryFrom<VIRTUAL_KEY> for KeyKind {
    type Error = Error;

    fn try_from(value: VIRTUAL_KEY) -> Result<Self> {
        Ok(match value {
            VK_A => KeyKind::A,
            VK_B => KeyKind::B,
            VK_C => KeyKind::C,
            VK_D => KeyKind::D,
            VK_E => KeyKind::E,
            VK_F => KeyKind::F,
            VK_G => KeyKind::G,
            VK_H => KeyKind::H,
            VK_I => KeyKind::I,
            VK_J => KeyKind::J,
            VK_K => KeyKind::K,
            VK_L => KeyKind::L,
            VK_M => KeyKind::M,
            VK_N => KeyKind::N,
            VK_O => KeyKind::O,
            VK_P => KeyKind::P,
            VK_Q => KeyKind::Q,
            VK_R => KeyKind::R,
            VK_S => KeyKind::S,
            VK_T => KeyKind::T,
            VK_U => KeyKind::U,
            VK_V => KeyKind::V,
            VK_W => KeyKind::W,
            VK_X => KeyKind::X,
            VK_Y => KeyKind::Y,
            VK_Z => KeyKind::Z,
            VK_0 => KeyKind::Zero,
            VK_1 => KeyKind::One,
            VK_2 => KeyKind::Two,
            VK_3 => KeyKind::Three,
            VK_4 => KeyKind::Four,
            VK_5 => KeyKind::Five,
            VK_6 => KeyKind::Six,
            VK_7 => KeyKind::Seven,
            VK_8 => KeyKind::Eight,
            VK_9 => KeyKind::Nine,
            VK_F1 => KeyKind::F1,
            VK_F2 => KeyKind::F2,
            VK_F3 => KeyKind::F3,
            VK_F4 => KeyKind::F4,
            VK_F5 => KeyKind::F5,
            VK_F6 => KeyKind::F6,
            VK_F7 => KeyKind::F7,
            VK_F8 => KeyKind::F8,
            VK_F9 => KeyKind::F9,
            VK_F10 => KeyKind::F10,
            VK_F11 => KeyKind::F11,
            VK_F12 => KeyKind::F12,
            VK_UP => KeyKind::Up,
            VK_DOWN => KeyKind::Down,
            VK_LEFT => KeyKind::Left,
            VK_RIGHT => KeyKind::Right,
            VK_HOME => KeyKind::Home,
            VK_END => KeyKind::End,
            VK_PRIOR => KeyKind::PageUp,
            VK_NEXT => KeyKind::PageDown,
            VK_INSERT => KeyKind::Insert,
            VK_DELETE => KeyKind::Delete,
            VK_CONTROL => KeyKind::Ctrl,
            VK_RETURN => KeyKind::Enter,
            VK_SPACE => KeyKind::Space,
            VK_OEM_3 => KeyKind::Tilde,
            VK_OEM_7 => KeyKind::Quote,
            VK_OEM_1 => KeyKind::Semicolon,
            VK_OEM_COMMA => KeyKind::Comma,
            VK_OEM_PERIOD => KeyKind::Period,
            VK_OEM_2 => KeyKind::Slash,
            VK_ESCAPE => KeyKind::Esc,
            VK_SHIFT => KeyKind::Shift,
            VK_MENU => KeyKind::Alt,
            _ => return Err(Error::KeyNotFound),
        })
    }
}

impl From<KeyKind> for VIRTUAL_KEY {
    fn from(value: KeyKind) -> Self {
        match value {
            KeyKind::A => VK_A,
            KeyKind::B => VK_B,
            KeyKind::C => VK_C,
            KeyKind::D => VK_D,
            KeyKind::E => VK_E,
            KeyKind::F => VK_F,
            KeyKind::G => VK_G,
            KeyKind::H => VK_H,
            KeyKind::I => VK_I,
            KeyKind::J => VK_J,
            KeyKind::K => VK_K,
            KeyKind::L => VK_L,
            KeyKind::M => VK_M,
            KeyKind::N => VK_N,
            KeyKind::O => VK_O,
            KeyKind::P => VK_P,
            KeyKind::Q => VK_Q,
            KeyKind::R => VK_R,
            KeyKind::S => VK_S,
            KeyKind::T => VK_T,
            KeyKind::U => VK_U,
            KeyKind::V => VK_V,
            KeyKind::W => VK_W,
            KeyKind::X => VK_X,
            KeyKind::Y => VK_Y,
            KeyKind::Z => VK_Z,
            KeyKind::Zero => VK_0,
            KeyKind::One => VK_1,
            KeyKind::Two => VK_2,
            KeyKind::Three => VK_3,
            KeyKind::Four => VK_4,
            KeyKind::Five => VK_5,
            KeyKind::Six => VK_6,
            KeyKind::Seven => VK_7,
            KeyKind::Eight => VK_8,
            KeyKind::Nine => VK_9,
            KeyKind::F1 => VK_F1,
            KeyKind::F2 => VK_F2,
            KeyKind::F3 => VK_F3,
            KeyKind::F4 => VK_F4,
            KeyKind::F5 => VK_F5,
            KeyKind::F6 => VK_F6,
            KeyKind::F7 => VK_F7,
            KeyKind::F8 => VK_F8,
            KeyKind::F9 => VK_F9,
            KeyKind::F10 => VK_F10,
            KeyKind::F11 => VK_F11,
            KeyKind::F12 => VK_F12,
            KeyKind::Up => VK_UP,
            KeyKind::Down => VK_DOWN,
            KeyKind::Left => VK_LEFT,
            KeyKind::Right => VK_RIGHT,
            KeyKind::Home => VK_HOME,
            KeyKind::End => VK_END,
            KeyKind::PageUp => VK_PRIOR,
            KeyKind::PageDown => VK_NEXT,
            KeyKind::Insert => VK_INSERT,
            KeyKind::Delete => VK_DELETE,
            KeyKind::Ctrl => VK_CONTROL,
            KeyKind::Enter => VK_RETURN,
            KeyKind::Space => VK_SPACE,
            KeyKind::Tilde => VK_OEM_3,
            KeyKind::Quote => VK_OEM_7,
            KeyKind::Semicolon => VK_OEM_1,
            KeyKind::Comma => VK_OEM_COMMA,
            KeyKind::Period => VK_OEM_PERIOD,
            KeyKind::Slash => VK_OEM_2,
            KeyKind::Esc => VK_ESCAPE,
            KeyKind::Shift => VK_SHIFT,
            KeyKind::Alt => VK_MENU,
        }
    }
}

pub fn client_to_monitor_or_frame(
    handle: Handle,
    x: i32,
    y: i32,
    monitor_coordinate: bool,
) -> Result<ConvertedCoordinates> {
    let handle = handle.as_inner().ok_or(Error::WindowNotFound)?;
    let mut point = POINT { x, y };
    unsafe { ClientToScreen(handle, &raw mut point).ok()? };

    if !monitor_coordinate {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(handle, &raw mut rect)? };

        let x = point.x - rect.left;
        let y = point.y - rect.top;
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;

        return Ok(ConvertedCoordinates {
            width,
            height,
            x,
            y,
        });
    }

    // Get monitor from window
    let monitor = unsafe { MonitorFromWindow(handle, MONITOR_DEFAULTTONULL) };
    if monitor.is_invalid() {
        return Err(Error::WindowNotFound);
    }

    let mut mi = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..MONITORINFO::default()
    };
    unsafe { GetMonitorInfoW(monitor, &mut mi).ok()? };
    let width = mi.rcMonitor.right - mi.rcMonitor.left;
    let height = mi.rcMonitor.bottom - mi.rcMonitor.top;

    let x = point.x - mi.rcMonitor.left;
    let y = point.y - mi.rcMonitor.top;

    Ok(ConvertedCoordinates {
        width,
        height,
        x,
        y,
    })
}

fn client_to_absolute_coordinate_raw(handle: HWND, x: i32, y: i32) -> Result<(i32, i32)> {
    let mut point = POINT { x, y };
    unsafe { ClientToScreen(handle, &raw mut point).ok()? };

    let virtual_left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let virtual_top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let virtual_width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let virtual_height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    if virtual_width == 0 || virtual_height == 0 {
        return Err(Error::WindowInvalidSize);
    }

    let dx = (point.x - virtual_left) * 65536 / virtual_width;
    let dy = (point.y - virtual_top) * 65536 / virtual_height;
    Ok((dx, dy))
}

// TODO: Is this good?
#[inline]
fn is_foreground(handle: HWND, kind: InputKind) -> bool {
    let handle_fg = unsafe { GetForegroundWindow() };
    if handle_fg.is_invalid() {
        return false;
    }
    match kind {
        InputKind::Focused => handle_fg == handle,
        InputKind::Foreground => {
            if handle_fg == handle {
                return false;
            }
            // Null != Null?
            if unsafe {
                MonitorFromWindow(handle_fg, MONITOR_DEFAULTTONULL)
                    != MonitorFromWindow(handle, MONITOR_DEFAULTTONULL)
            } {
                return false;
            }
            let mut rect_fg = RECT::default();
            let mut rect_handle = RECT::default();
            let mut rect_intersect = RECT::default();
            unsafe {
                if GetWindowRect(handle_fg, &mut rect_fg).is_err()
                    || GetWindowRect(handle, &mut rect_handle).is_err()
                {
                    return false;
                }
                IntersectRect(
                    &raw mut rect_intersect,
                    &raw const rect_fg,
                    &raw const rect_handle,
                )
                .as_bool()
            }
        }
    }
}

#[inline]
fn send_input(input: [INPUT; 1]) -> Result<()> {
    let result = unsafe { SendInput(&input, size_of::<INPUT>() as i32) };
    // could be UIPI
    if result == 0 {
        Err(Error::from_last_win_error())
    } else {
        Ok(())
    }
}

#[inline]
fn to_scan_code(key: VIRTUAL_KEY) -> (u16, bool) {
    let scan_code = unsafe { MapVirtualKeyW(key.0 as u32, MAPVK_VK_TO_VSC_EX) } as u16;
    let code = scan_code & 0xFF;
    let is_extended = if VK_INSERT == key {
        true
    } else {
        (scan_code & 0xFF00) != 0
    };
    (code, is_extended)
}

#[inline]
fn to_input(key: VIRTUAL_KEY, scan_code: u16, is_extended: bool, is_down: bool) -> [INPUT; 1] {
    let is_extended = if is_extended {
        KEYEVENTF_EXTENDEDKEY
    } else {
        KEYBD_EVENT_FLAGS::default()
    };
    let is_up = if is_down {
        KEYBD_EVENT_FLAGS::default()
    } else {
        KEYEVENTF_KEYUP
    };
    [INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: scan_code,
                dwFlags: is_extended | is_up,
                dwExtraInfo: *PROCESS_ID as usize,
                ..KEYBDINPUT::default()
            },
        },
    }]
}
