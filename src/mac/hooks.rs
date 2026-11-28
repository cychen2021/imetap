//! The Quartz event taps (the macOS counterpart of the Windows hooks), and the thread-local state
//! they touch.
//!
//! The main tap sees keys and clicks: it feeds the Ctrl tap detector and swallows hotkeys. A
//! listen-only tap sees mouse movement, for the reminder. Creating a tap needs the Accessibility
//! permission. Until it is granted the creation is retried every few seconds. The callbacks never
//! borrow `app::STATE`, so they can never collide with a borrow of it.

use std::cell::{Cell, RefCell};

use super::app::{Action, REMINDER, act_later, note_activity};
use super::ffi::{self, EventTap, TapEvent, Timer};
use super::hotkeys::{self, Hotkey};
use crate::tap::{Key, Tap, TapDetector};

const KEYCODE_CTRL_LEFT: i64 = 59;
const KEYCODE_CTRL_RIGHT: i64 = 62;

thread_local! {
    pub(super) static TAP: RefCell<TapDetector> = RefCell::new(TapDetector::default());
    /// The hotkeys of `config.toml`.
    pub(super) static HOTKEYS: RefCell<Vec<Hotkey>> = const { RefCell::new(Vec::new()) };
    static MAIN_TAP: Cell<Option<EventTap>> = const { Cell::new(None) };
    /// The timer that ends the wait for a third Ctrl tap.
    static TRIPLE_TIMER: Cell<Option<Timer>> = const { Cell::new(None) };
    /// The listen-only tap that sees mouse movement, for the reminder.
    static ACTIVITY_TAP: Cell<Option<EventTap>> = const { Cell::new(None) };
    /// The Accessibility prompt has been shown, so retries check silently.
    static PROMPTED: Cell<bool> = const { Cell::new(false) };
}

/// Creates the main tap, and the listen-only one when the reminder is on.
pub(super) fn start_event_tap() {
    let events = [
        ffi::LEFT_MOUSE_DOWN,
        ffi::RIGHT_MOUSE_DOWN,
        ffi::OTHER_MOUSE_DOWN,
        ffi::KEY_DOWN,
        ffi::KEY_UP,
        ffi::FLAGS_CHANGED,
    ];
    let Some(tap) = ffi::create_event_tap(false, false, &events, on_event) else {
        // Not trusted yet: show the system prompt once and try again in a few seconds. Prompting
        // on every retry would pop the dialog up every few seconds until the user grants access.
        ffi::ensure_accessibility(!PROMPTED.with(|p| p.replace(true)));
        ffi::schedule_once(3.0, start_event_tap);
        return;
    };
    MAIN_TAP.with(|t| t.set(Some(tap)));
    tap.attach();
    if REMINDER.with(|r| r.borrow().is_some()) {
        start_activity_tap();
    }
}

/// A listen-only tap for mouse movement, kept apart from the main tap so that the flood of moves
/// never waits on the hotkey logic. Keys and clicks reach the reminder through the main tap.
fn start_activity_tap() {
    let events = [
        ffi::MOUSE_MOVED,
        ffi::LEFT_MOUSE_DRAGGED,
        ffi::RIGHT_MOUSE_DRAGGED,
        ffi::OTHER_MOUSE_DRAGGED,
        ffi::SCROLL_WHEEL,
    ];
    let Some(tap) = ffi::create_event_tap(true, true, &events, on_activity) else {
        eprintln!("imetap: cannot watch the mouse for the reminder");
        return;
    };
    ACTIVITY_TAP.with(|t| t.set(Some(tap)));
    tap.attach();
}

fn on_activity(event: &TapEvent) -> bool {
    if event.kind == TapEvent::DISABLED {
        if let Some(tap) = ACTIVITY_TAP.with(Cell::get) {
            tap.enable();
        }
    } else {
        note_activity();
    }
    true
}

/// Handles an event of the main tap. Returns false to swallow it.
fn on_event(event: &TapEvent) -> bool {
    if event.kind == TapEvent::DISABLED {
        // The system switches a slow tap off. Switch it back on.
        if let Some(tap) = MAIN_TAP.with(Cell::get) {
            tap.enable();
        }
        return true;
    }
    note_activity();
    let (flags, code, now) = (event.flags, event.keycode, event.time_ms);
    match event.kind {
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
                return false;
            }
        }
        ffi::LEFT_MOUSE_DOWN | ffi::RIGHT_MOUSE_DOWN | ffi::OTHER_MOUSE_DOWN => {
            on_tap(TAP.with(|t| t.borrow_mut().pointer()))
        }
        _ => {}
    }
    true
}

/// Acts on a recognised tap: a double tap toggles, a triple tap shows the HUD, and a pending
/// double tap arms the timer that waits for a third.
fn on_tap(tap: Tap) {
    match tap {
        Tap::None => {}
        Tap::Pending => {
            if let Some(old) = TRIPLE_TIMER.with(Cell::take) {
                old.cancel();
            }
            let wait = TAP.with(|t| t.borrow().triple_ms()) as f64 / 1000.0;
            let timer = ffi::schedule(wait, triple_timer_fired);
            TRIPLE_TIMER.with(|t| t.set(Some(timer)));
        }
        Tap::Double => act_later(Action::Toggle),
        Tap::Triple => act_later(Action::Show),
    }
}

fn triple_timer_fired() {
    TRIPLE_TIMER.with(|t| t.set(None));
    on_tap(TAP.with(|t| t.borrow_mut().expire()));
}
