//! The keyboard, mouse and WinEvent hooks, and the thread-local state they touch.
//!
//! The hook procedures run on the message thread for every input event, so they stay trivial: they
//! update the state below and post messages to the main window. They never borrow `app::APP`, so a
//! hook can never collide with a borrow of it.

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, PostMessageW, SetTimer,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN,
};

use super::app::{TIMER_HOLD, WM_PASSIVE, WM_REMIND, WM_TAP, WM_TAP_PENDING, WM_TRIPLE_TAP};
use crate::config::Config;
use crate::hold::{HeldKey, KeyHold};
use crate::reminder::{Reminder, now_ms};
use crate::tap::{Key, Tap, TapDetector};

/// The longest keys are held back.
const HOLD_TIMEOUT_MS: u32 = 1000;

const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
const EVENT_OBJECT_FOCUS: u32 = 0x8005;
const EVENT_OBJECT_IME_CHANGE: u32 = 0x8029;
const WINEVENT_OUTOFCONTEXT: u32 = 0;
const LLKHF_EXTENDED: u32 = 0x01;
const LLKHF_INJECTED: u32 = 0x10;
const LLMHF_INJECTED: u32 = 0x01;

thread_local! {
    pub(super) static TAP: RefCell<TapDetector> = RefCell::new(TapDetector::default());
    /// The main window, as an integer handle.
    pub(super) static MAIN: Cell<isize> = const { Cell::new(0) };
    pub(super) static TAP_ENABLED: Cell<bool> = const { Cell::new(false) };
    /// Keys typed while a double tap is applied.
    pub(super) static HOLD: RefCell<KeyHold> = RefCell::new(KeyHold::default());
    pub(super) static HOLD_ENABLED: Cell<bool> = const { Cell::new(false) };
    /// `None` when the reminder is off.
    pub(super) static REMINDER: RefCell<Option<Reminder>> = const { RefCell::new(None) };
    /// The last check found focus outside a text field, so focus events, key presses and clicks
    /// trigger a new one.
    pub(super) static WAIT_FOR_TEXT: Cell<bool> = const { Cell::new(false) };
    /// The next debounced read shows the HUD even if nothing changed.
    pub(super) static REMIND_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// The low-level and WinEvent hooks. Removed on drop.
pub(super) struct Hooks {
    keyboard: Option<HHOOK>,
    mouse: Option<HHOOK>,
    events: Vec<HWINEVENTHOOK>,
}

impl Hooks {
    pub(super) fn install(config: &Config, instance: HINSTANCE) -> Hooks {
        // SAFETY: the hook procedures below are `extern "system"` functions that stay valid.
        unsafe {
            let (mut keyboard, mut mouse) = (None, None);
            // The reminder needs to see input too.
            if config.tap.enabled || config.reminder.enabled {
                keyboard =
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(instance), 0).ok();
                mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(instance), 0).ok();
            }
            let mut watched = vec![EVENT_SYSTEM_FOREGROUND, EVENT_OBJECT_IME_CHANGE];
            if config.hud.wait_for_text_field {
                watched.push(EVENT_OBJECT_FOCUS);
            }
            let events = watched
                .into_iter()
                .map(|event| {
                    SetWinEventHook(
                        event,
                        event,
                        None,
                        Some(win_event_proc),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT,
                    )
                })
                .collect();
            Hooks {
                keyboard,
                mouse,
                events,
            }
        }
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: unhooks handles obtained in `install`.
        unsafe {
            for hook in [self.keyboard, self.mouse].into_iter().flatten() {
                UnhookWindowsHookEx(hook).ok();
            }
            for hook in &self.events {
                let _ = UnhookWinEvent(*hook);
            }
        }
    }
}

/// Feeds key or mouse activity to the reminder. Cheap enough for every mouse move.
fn note_activity() {
    let resumed = REMINDER.with(|r| {
        r.borrow_mut()
            .as_mut()
            .is_some_and(|r| r.activity(now_ms()))
    });
    if resumed {
        post_main(WM_REMIND);
    }
}

/// Hands a recognised tap to the window procedure.
fn post_tap(tap: Tap) {
    match tap {
        Tap::None => {}
        Tap::Pending => {
            begin_hold();
            post_main(WM_TAP_PENDING);
        }
        Tap::Double => {
            begin_hold();
            post_main(WM_TAP);
        }
        Tap::Triple => post_main(WM_TRIPLE_TAP),
    }
}

/// Starts holding back keys until the double tap has taken effect.
fn begin_hold() {
    if !HOLD_ENABLED.with(Cell::get) {
        return;
    }
    HOLD.with(|h| h.borrow_mut().begin());
    let hwnd = MAIN.with(Cell::get);
    // SAFETY: arms a timer of the main window, from its own thread.
    unsafe {
        SetTimer(
            Some(HWND(hwnd as *mut _)),
            TIMER_HOLD,
            HOLD_TIMEOUT_MS,
            None,
        )
    };
}

fn post_main(message: u32) {
    let hwnd = MAIN.with(Cell::get);
    // SAFETY: posting to the main window. Failure just means it is gone.
    unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), message, WPARAM(0), LPARAM(0)).ok() };
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: for WH_KEYBOARD_LL, `lparam` points to a KBDLLHOOKSTRUCT.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // Ignore keys sent by other programs (including our own SendInput of the IME hotkey).
        let injected = info.flags.0 & LLKHF_INJECTED != 0;
        if !injected {
            note_activity();
            // Typing is a sign of a text field, for apps whose focus changes are not reported.
            if WAIT_FOR_TEXT.with(Cell::get)
                && matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN)
            {
                post_main(WM_PASSIVE);
            }
        }
        if !injected && TAP_ENABLED.with(Cell::get) {
            // `time` is the event's own timestamp in ms, so a delayed hook call does not skew taps.
            let key = if matches!(info.vkCode, 0xA2 | 0xA3) {
                Key::Ctrl
            } else {
                Key::Other
            };
            let now = info.time as u64;
            let tap = TAP.with(|t| match wparam.0 as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => t.borrow_mut().key_down(key, now),
                WM_KEYUP | WM_SYSKEYUP => t.borrow_mut().key_up(key, now),
                _ => Tap::None,
            });
            // A key typed while a double tap is being applied waits for it. This runs before
            // `post_tap`, so the Ctrl release that completes the double tap itself goes through.
            let held = HOLD.with(|h| {
                h.borrow_mut().capture(HeldKey {
                    vk: info.vkCode,
                    scan: info.scanCode,
                    extended: info.flags.0 & LLKHF_EXTENDED != 0,
                    up: matches!(wparam.0 as u32, WM_KEYUP | WM_SYSKEYUP),
                })
            });
            post_tap(tap);
            if held {
                return LRESULT(1);
            }
        }
    }
    // SAFETY: passing the event on, unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: for WH_MOUSE_LL, `lparam` points to an MSLLHOOKSTRUCT.
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        if info.flags & LLMHF_INJECTED == 0 {
            note_activity();
        }
        if matches!(
            wparam.0 as u32,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        ) {
            if WAIT_FOR_TEXT.with(Cell::get) {
                post_main(WM_PASSIVE);
            }
            post_tap(TAP.with(|t| t.borrow_mut().pointer()));
        }
    }
    // SAFETY: passing the event on, unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn win_event_proc(
    _: HWINEVENTHOOK,
    event: u32,
    _: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    if event == EVENT_OBJECT_FOCUS {
        // Only interesting while the HUD waits for a text field. Focus moves all the time.
        if WAIT_FOR_TEXT.with(Cell::get) {
            post_main(WM_PASSIVE);
        }
        return;
    }
    if event == EVENT_SYSTEM_FOREGROUND {
        let due = REMINDER.with(|r| {
            r.borrow_mut()
                .as_mut()
                .is_some_and(|r| r.focus_changed(now_ms()))
        });
        if due {
            REMIND_PENDING.with(|p| p.set(true));
        }
    }
    post_main(WM_PASSIVE);
}
