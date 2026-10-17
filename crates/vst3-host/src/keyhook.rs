#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadKey {
    Digit(u8),
    Dot,
    Plus,
    Minus,
    Star,
    Slash,
    Enter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Held {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    Start { vk: u16, held: Held },
    Pad { key: PadKey, down: bool, held: Held },
}

pub mod vk {
    pub const F1: u16 = 0x70;
    pub const F20: u16 = 0x83;
    pub const OEM_PLUS: u16 = 0xBB;
    pub const OEM_MINUS: u16 = 0xBD;
    pub const OEM_COMMA: u16 = 0xBC;
    pub const OEM_PERIOD: u16 = 0xBE;
    pub const OEM_1: u16 = 0xBA;
    pub const OEM_2: u16 = 0xBF;
    pub const OEM_4: u16 = 0xDB;
    pub const OEM_5: u16 = 0xDC;
    pub const OEM_6: u16 = 0xDD;
    pub const OEM_7: u16 = 0xDE;
    pub const SPACE: u16 = 0x20;
    pub const TAB: u16 = 0x09;
    pub const RETURN: u16 = 0x0D;
    pub const BACK: u16 = 0x08;
    pub const DELETE: u16 = 0x2E;
    pub const HOME: u16 = 0x24;
    pub const END: u16 = 0x23;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
}

#[cfg(windows)]
mod imp {
    use super::{Held, KeyEvent, PadKey};
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
    use std::sync::{Mutex, OnceLock};
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    static HOOK: AtomicIsize = AtomicIsize::new(0);
    static START_DOWN: AtomicBool = AtomicBool::new(false);
    static START_USED: AtomicBool = AtomicBool::new(false);
    static TEXT_FOCUS: AtomicBool = AtomicBool::new(false);
    static QUEUE: Mutex<Vec<KeyEvent>> = Mutex::new(Vec::new());
    static TAKEN: Mutex<[bool; 256]> = Mutex::new([false; 256]);
    static WAKER: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

    pub fn install() {
        if HOOK.load(Ordering::Relaxed) != 0 {
            return;
        }
        let h = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(proc), std::ptr::null_mut(), 0) };
        HOOK.store(h as isize, Ordering::Relaxed);
    }

    pub fn set_waker(wake: Box<dyn Fn() + Send + Sync>) {
        let _ = WAKER.set(wake);
    }

    pub fn set_text_focus(on: bool) {
        TEXT_FOCUS.store(on, Ordering::Relaxed);
    }

    pub fn drain() -> Vec<KeyEvent> {
        QUEUE.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
    }

    fn push(e: KeyEvent) {
        if let Ok(mut q) = QUEUE.lock() {
            q.push(e);
        }
        if let Some(w) = WAKER.get() {
            w();
        }
    }

    fn ours() -> bool {
        unsafe {
            let w = GetForegroundWindow();
            if w.is_null() {
                return false;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(w, &mut pid);
            if pid != GetCurrentProcessId() {
                return false;
            }
            let mut buf = [0u16; 64];
            let n = GetClassNameW(w, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
            String::from_utf16_lossy(&buf[..n]) == "Window Class"
        }
    }

    fn down(vk: u16) -> bool {
        (unsafe { GetAsyncKeyState(i32::from(vk)) } as u16) & 0x8000 != 0
    }

    fn held() -> Held {
        Held { ctrl: down(VK_CONTROL), shift: down(VK_SHIFT), alt: down(VK_MENU) }
    }

    fn pad_key(vk: u16, extended: bool) -> Option<PadKey> {
        Some(match vk {
            VK_NUMPAD0..=VK_NUMPAD9 => PadKey::Digit((vk - VK_NUMPAD0) as u8),
            VK_DECIMAL => PadKey::Dot,
            VK_ADD => PadKey::Plus,
            VK_SUBTRACT => PadKey::Minus,
            VK_MULTIPLY => PadKey::Star,
            VK_DIVIDE => PadKey::Slash,
            VK_RETURN if extended => PadKey::Enter,
            _ if extended => return None,
            VK_INSERT => PadKey::Digit(0),
            VK_END => PadKey::Digit(1),
            VK_DOWN => PadKey::Digit(2),
            VK_NEXT => PadKey::Digit(3),
            VK_LEFT => PadKey::Digit(4),
            VK_CLEAR => PadKey::Digit(5),
            VK_RIGHT => PadKey::Digit(6),
            VK_HOME => PadKey::Digit(7),
            VK_UP => PadKey::Digit(8),
            VK_PRIOR => PadKey::Digit(9),
            VK_DELETE => PadKey::Dot,
            _ => return None,
        })
    }

    fn taken(vk: u16, set: Option<bool>) -> bool {
        let Ok(mut t) = TAKEN.lock() else { return false };
        let i = usize::from(vk & 0xFF);
        let was = t[i];
        if let Some(v) = set {
            t[i] = v;
        }
        was
    }

    fn mask_start() {
        let key = |flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0xE8, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        };
        let inputs = [key(0), key(KEYEVENTF_KEYUP)];
        unsafe { SendInput(2, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    }

    fn key(vk: u16, flags: u32, pressed: bool) -> bool {
        if matches!(vk, VK_LWIN | VK_RWIN) {
            if pressed {
                if !START_DOWN.swap(true, Ordering::Relaxed) {
                    START_USED.store(false, Ordering::Relaxed);
                }
            } else {
                START_DOWN.store(false, Ordering::Relaxed);
                if START_USED.swap(false, Ordering::Relaxed) {
                    mask_start();
                }
            }
            return false;
        }
        if matches!(vk, VK_CONTROL | VK_LCONTROL | VK_RCONTROL | VK_SHIFT | VK_LSHIFT | VK_RSHIFT | VK_MENU | VK_LMENU | VK_RMENU) {
            return false;
        }
        let extended = flags & LLKHF_EXTENDED != 0;
        if !pressed {
            let was = taken(vk, Some(false));
            if was && let Some(p) = pad_key(vk, extended) {
                push(KeyEvent::Pad { key: p, down: false, held: held() });
            }
            return was;
        }
        if !ours() {
            return false;
        }
        if START_DOWN.load(Ordering::Relaxed) {
            if taken(vk, Some(true)) {
                return true;
            }
            START_USED.store(true, Ordering::Relaxed);
            let main = match pad_key(vk, extended) {
                Some(PadKey::Digit(d)) => 0x30 + u16::from(d),
                Some(PadKey::Plus) => super::vk::OEM_PLUS,
                Some(PadKey::Minus) => super::vk::OEM_MINUS,
                _ => vk,
            };
            push(KeyEvent::Start { vk: main, held: held() });
            return true;
        }
        if TEXT_FOCUS.load(Ordering::Relaxed) {
            return false;
        }
        let Some(p) = pad_key(vk, extended) else { return false };
        if !taken(vk, Some(true)) {
            push(KeyEvent::Pad { key: p, down: true, held: held() });
        }
        true
    }

    unsafe extern "system" fn proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            let msg = wparam as u32;
            let pressed = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let released = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            if kb.flags & LLKHF_INJECTED == 0 && (pressed || released) && key(kb.vkCode as u16, kb.flags, pressed) {
                return 1;
            }
        }
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::KeyEvent;
    pub fn install() {}
    pub fn set_waker(_: Box<dyn Fn() + Send + Sync>) {}
    pub fn set_text_focus(_: bool) {}
    pub fn drain() -> Vec<KeyEvent> {
        Vec::new()
    }
}

pub use imp::{drain, install, set_text_focus, set_waker};
