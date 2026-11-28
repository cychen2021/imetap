//! The Quartz event taps (the macOS counterpart of the Windows hooks), and the thread-local state
//! they touch.
//!
//! The main tap sees keys and clicks: it feeds the Ctrl tap detector and swallows hotkeys. A
//! listen-only tap sees mouse movement, for the reminder. Creating a tap needs the Accessibility
//! permission. Until it is granted the creation is retried every few seconds. The callbacks never
//! borrow `app::STATE`, so they can never collide with a borrow of it.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr;

use super::app::{Action, REMINDER, act_later, note_activity, schedule};
use super::ffi::{self, CFMachPortRef, CFRunLoopTimerRef, CGEventRef, CGEventTapProxy};
use super::hotkeys::{self, Hotkey};
use crate::tap::{Key, Tap, TapDetector};

const KEYCODE_CTRL_LEFT: i64 = 59;
const KEYCODE_CTRL_RIGHT: i64 = 62;

thread_local! {
    pub(super) static TAP: RefCell<TapDetector> = RefCell::new(TapDetector::default());
    /// The hotkeys of `config.toml`.
    pub(super) static HOTKEYS: RefCell<Vec<Hotkey>> = const { RefCell::new(Vec::new()) };
    static TAP_PORT: Cell<usize> = const { Cell::new(0) };
    /// The timer that ends the wait for a third Ctrl tap.
    static TRIPLE_TIMER: Cell<usize> = const { Cell::new(0) };
    /// The listen-only tap that sees mouse movement, for the reminder.
    static ACTIVITY_PORT: Cell<usize> = const { Cell::new(0) };
    /// The Accessibility prompt has been shown, so retries check silently.
    static PROMPTED: Cell<bool> = const { Cell::new(false) };
}

/// Creates the main tap, and the listen-only one when the reminder is on.
pub(super) fn start_event_tap() {
    let mask = [
        ffi::LEFT_MOUSE_DOWN,
        ffi::RIGHT_MOUSE_DOWN,
        ffi::OTHER_MOUSE_DOWN,
        ffi::KEY_DOWN,
        ffi::KEY_UP,
        ffi::FLAGS_CHANGED,
    ]
    .into_iter()
    .fold(0u64, |acc, t| acc | 1 << t);
    // SAFETY: creates a tap whose callback is a plain `extern "C"` function.
    let port = unsafe {
        ffi::CGEventTapCreate(
            0, /* session */
            0, /* head */
            0, /* default */
            mask,
            tap_callback,
            ptr::null_mut(),
        )
    };
    if port.is_null() {
        // Not trusted yet: show the system prompt once and try again in a few seconds. Prompting
        // on every retry would pop the dialog up every few seconds until the user grants access.
        ffi::ensure_accessibility(!PROMPTED.with(|p| p.replace(true)));
        schedule(3.0, retry_tap);
        return;
    }
    TAP_PORT.with(|p| p.set(port as usize));
    attach_tap(port);
    if REMINDER.with(|r| r.borrow().is_some()) {
        start_activity_tap();
    }
}

/// Attaches a tap's mach port to the main run loop and enables it.
fn attach_tap(port: CFMachPortRef) {
    // SAFETY: `port` is a live event tap.
    unsafe {
        let source = ffi::CFMachPortCreateRunLoopSource(ptr::null(), port, 0);
        ffi::CFRunLoopAddSource(ffi::CFRunLoopGetMain(), source, ffi::kCFRunLoopCommonModes);
        ffi::CGEventTapEnable(port, true);
    }
}

/// A listen-only tap for mouse movement, kept apart from the main tap so that the flood of moves
/// never waits on the hotkey logic. Keys and clicks reach the reminder through the main tap.
fn start_activity_tap() {
    let mask = [
        ffi::MOUSE_MOVED,
        ffi::LEFT_MOUSE_DRAGGED,
        ffi::RIGHT_MOUSE_DRAGGED,
        ffi::OTHER_MOUSE_DRAGGED,
        ffi::SCROLL_WHEEL,
    ]
    .into_iter()
    .fold(0u64, |acc, t| acc | 1 << t);
    // SAFETY: creates a tap whose callback is a plain `extern "C"` function.
    let port = unsafe {
        ffi::CGEventTapCreate(
            0, /* session */
            1, /* tail */
            1, /* listen only */
            mask,
            activity_callback,
            ptr::null_mut(),
        )
    };
    if port.is_null() {
        eprintln!("imetap: cannot watch the mouse for the reminder");
        return;
    }
    ACTIVITY_PORT.with(|p| p.set(port as usize));
    attach_tap(port);
}

extern "C" fn activity_callback(
    _: CGEventTapProxy,
    kind: u32,
    event: CGEventRef,
    _: *mut c_void,
) -> CGEventRef {
    match kind {
        ffi::TAP_DISABLED_BY_TIMEOUT | ffi::TAP_DISABLED_BY_USER_INPUT => {
            let port = ACTIVITY_PORT.with(Cell::get) as CFMachPortRef;
            // SAFETY: the port was created by `start_activity_tap`.
            unsafe { ffi::CGEventTapEnable(port, true) };
        }
        _ => note_activity(),
    }
    event
}

extern "C" fn retry_tap(_: CFRunLoopTimerRef, _: *mut c_void) {
    start_event_tap();
}

extern "C" fn tap_callback(
    _: CGEventTapProxy,
    kind: u32,
    event: CGEventRef,
    _: *mut c_void,
) -> CGEventRef {
    // SAFETY: `event` is valid for the duration of the callback.
    let (flags, code, now) = unsafe {
        (
            ffi::CGEventGetFlags(event),
            ffi::CGEventGetIntegerValueField(event, ffi::FIELD_KEYCODE),
            ffi::CGEventGetTimestamp(event) / 1_000_000,
        )
    };
    match kind {
        ffi::TAP_DISABLED_BY_TIMEOUT | ffi::TAP_DISABLED_BY_USER_INPUT => {
            // The system switches a slow tap off. Switch it back on.
            let port = TAP_PORT.with(Cell::get) as CFMachPortRef;
            // SAFETY: the port was created by `start_event_tap`.
            unsafe { ffi::CGEventTapEnable(port, true) };
            return event;
        }
        _ => note_activity(),
    }
    match kind {
        ffi::FLAGS_CHANGED => {
            if code == KEYCODE_CTRL_LEFT || code == KEYCODE_CTRL_RIGHT {
                let tap = TAP.with(|t| {
                    let mut t = t.borrow_mut();
                    if flags & ffi::FLAG_CONTROL != 0 {
                        t.key_down(Key::Ctrl, now)
                    } else {
                        t.key_up(Key::Ctrl, now)
                    }
                });
                on_tap(tap);
            } else {
                on_tap(TAP.with(|t| t.borrow_mut().key_down(Key::Other, now)));
            }
        }
        ffi::KEY_DOWN => {
            on_tap(TAP.with(|t| t.borrow_mut().key_down(Key::Other, now)));
            let hit = HOTKEYS.with(|h| hotkeys::find(&h.borrow(), code, flags));
            if let Some(ime) = hit {
                act_later(Action::Set(ime));
                // Swallow the shortcut so the frontmost app never sees it.
                return ptr::null_mut();
            }
        }
        ffi::LEFT_MOUSE_DOWN | ffi::RIGHT_MOUSE_DOWN | ffi::OTHER_MOUSE_DOWN => {
            on_tap(TAP.with(|t| t.borrow_mut().pointer()))
        }
        _ => {}
    }
    event
}

/// Acts on a recognised tap: a double tap toggles, a triple tap shows the HUD, and a pending
/// double tap arms the timer that waits for a third.
fn on_tap(tap: Tap) {
    match tap {
        Tap::None => {}
        Tap::Pending => {
            let old = TRIPLE_TIMER.with(Cell::get);
            if old != 0 {
                // SAFETY: invalidating a timer we created. Harmless if it has fired already.
                unsafe { ffi::CFRunLoopTimerInvalidate(old as CFRunLoopTimerRef) };
            }
            let wait = TAP.with(|t| t.borrow().triple_ms()) as f64 / 1000.0;
            let timer = schedule(wait, triple_timer_fired);
            TRIPLE_TIMER.with(|t| t.set(timer as usize));
        }
        Tap::Double => act_later(Action::Toggle),
        Tap::Triple => act_later(Action::Show),
    }
}

extern "C" fn triple_timer_fired(_: CFRunLoopTimerRef, _: *mut c_void) {
    TRIPLE_TIMER.with(|t| t.set(0));
    on_tap(TAP.with(|t| t.borrow_mut().expire()));
}
