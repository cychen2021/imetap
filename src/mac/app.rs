//! The resident macOS tool: status item, hotkeys, Ctrl double-tap, input-source watching and HUD.
//!
//! Everything runs on the main run loop. There are no extra threads. The keyboard and mouse are
//! observed with a Quartz event tap, which needs the Accessibility permission (the same as
//! Hammerspoon). Until it is granted the tap creation is retried every few seconds.

use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_void};
use std::ptr::{self, NonNull};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2::{MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBezierPath, NSColor,
    NSCompositingOperation, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
    NSGraphicsContext, NSImage, NSMenu, NSMenuItem, NSPanel, NSScreen, NSStatusBar, NSStatusItem,
    NSStringDrawing, NSTextAlignment, NSTextField, NSView, NSWindowCollectionBehavior,
    NSWindowStyleMask, NSWorkspace, NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::{
    MainThreadMarker, NSAttributedStringKey, NSDictionary, NSNotification, NSObject,
    NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, ns_string,
};

use super::ffi::{self, CFMachPortRef, CFRunLoopTimerRef, CGEventRef, CGEventTapProxy};
use super::{Engine, autostart_enabled, claim_pid, mingkuai, open, set_autostart};
use crate::config::{self, Config, Key as ShortcutKey, Shortcut};
use crate::debug;
use crate::fade::Fade;
use crate::mingkuai as mingkuai_wire;
use crate::reminder::{Reminder, now_ms};
use crate::state::{Color, Ime, Rgb, Snapshot};
use crate::tap::{Key, Tap, TapDetector};

/// How long input-source and focus notifications may settle before the state is read.
const DEBOUNCE_SECS: f64 = 0.25;
/// The HUD's size in points.
const HUD_WIDTH: f64 = 240.0;
/// Width of the stripe on the left that names the input method.
const HUD_STRIPE: f64 = 60.0;
const HUD_HEIGHT: f64 = 100.0;
const HUD_BOTTOM_MARGIN: f64 = 48.0;
/// `NSStatusWindowLevel`.
const STATUS_WINDOW_LEVEL: isize = 25;

/// Left inset of the menu header, lined up with the text of the other menu items.
const HEADER_INSET: f64 = 14.0;

const HOTKEY_IMES: [Ime; 4] = [Ime::Us, Ime::Shuangpin, Ime::Doubao, Ime::Mingkuai];

define_class!(
    /// Target of the status item's menu actions.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ImetapMenuTarget"]
    struct MenuTarget;

    unsafe impl NSObjectProtocol for MenuTarget {}

    impl MenuTarget {
        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }

        #[unsafe(method(toggleLogin:))]
        fn toggle_login(&self, _sender: Option<&AnyObject>) {
            if let Err(e) = set_autostart(!autostart_enabled(), debug::enabled()) {
                eprintln!("imetap: cannot change run at login: {e:#}");
            }
        }

        #[unsafe(method(openConfig:))]
        fn open_config(&self, _sender: Option<&AnyObject>) {
            if let Err(e) = open_config_file() {
                eprintln!("imetap: cannot open the config file: {e:#}");
            }
        }
    }
);

struct Hud {
    panel: Retained<NSPanel>,
    glyph: Retained<NSTextField>,
    /// The stripe on the left, in the input method's colour.
    stripe: Retained<NSView>,
    /// The input method's label on the stripe.
    stripe_label: Retained<NSTextField>,
}

/// One appearance of the HUD, stepped by timers.
#[derive(Clone, Copy)]
struct HudRun {
    fade: Fade,
    started: Instant,
    /// How far into the curve it began (non-zero when it took over from one still visible).
    offset_ms: u64,
}

/// How often the HUD's brightness is updated.
const FADE_STEP_SECS: f64 = 1.0 / 60.0;

struct State {
    mtm: MainThreadMarker,
    config: Config,
    engine: Engine,
    hud: Hud,
    status_item: Retained<NSStatusItem>,
    last: Option<Snapshot>,
    hotkeys: Vec<(u16, u64, Ime)>,
    /// The HUD's current appearance. `Some` while a chain of step timers runs.
    hud_run: Option<HudRun>,
    /// The HUD's last brightness, from 0.0 to 1.0.
    hud_level: f32,
    probe_timer: Option<CFRunLoopTimerRef>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    // The tap callback touches these two only, so it never collides with a borrow of `STATE`.
    static TAP: RefCell<TapDetector> = RefCell::new(TapDetector::default());
    static TAP_PORT: Cell<usize> = const { Cell::new(0) };
    /// The timer that ends the wait for a third Ctrl tap.
    static TRIPLE_TIMER: Cell<usize> = const { Cell::new(0) };
    /// The listen-only tap that sees mouse movement, for the reminder.
    static ACTIVITY_PORT: Cell<usize> = const { Cell::new(0) };
    /// `None` when the reminder is off. Kept out of `STATE` like `TAP`.
    static REMINDER: RefCell<Option<Reminder>> = const { RefCell::new(None) };
    /// The next probe shows the HUD even if nothing changed.
    static REMIND_PENDING: Cell<bool> = const { Cell::new(false) };
    /// The Accessibility prompt has been shown, so retries check silently.
    static PROMPTED: Cell<bool> = const { Cell::new(false) };
}

/// Runs the tool until the user quits from the status item menu.
pub fn run(config: Config) -> Result<()> {
    claim_pid()?;
    let mtm = MainThreadMarker::new().expect("run must be called on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let hotkeys = HOTKEY_IMES
        .into_iter()
        .filter_map(|ime| {
            config
                .shortcut(ime)
                .ok()
                .flatten()
                .map(|s| (keycode(s.key), flags_of(&s), ime))
        })
        .collect();
    let tap = TapDetector::new(config.tap.gap_ms, config.tap.hold_ms, config.tap.triple_ms);
    TAP.with(|t| *t.borrow_mut() = tap);
    let r = &config.reminder;
    if r.enabled {
        let reminder = Reminder::new(r.mode, r.idle_secs, r.dwell_secs, now_ms());
        REMINDER.with(|cell| *cell.borrow_mut() = Some(reminder));
    }

    let status_item = make_status_item(mtm);
    let hud = make_hud(mtm);
    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            mtm,
            config,
            engine: Engine::new(),
            hud,
            status_item,
            last: None,
            hotkeys,
            hud_run: None,
            hud_level: 0.0,
            probe_timer: None,
        })
    });

    start_event_tap();
    observe_changes();
    if mingkuai_wire::enabled() {
        spawn_mingkuai_watcher();
    }
    probe(false, false);
    app.run();
    Ok(())
}

fn open_config_file() -> Result<()> {
    let path =
        config::config_path().ok_or_else(|| anyhow::anyhow!("no configuration directory"))?;
    if !path.exists() {
        std::fs::create_dir_all(
            path.parent()
                .ok_or_else(|| anyhow::anyhow!("no parent directory"))?,
        )?;
        std::fs::write(&path, config::DEFAULT_TOML)?;
    }
    open(&path)
}

// ---- keys --------------------------------------------------------------------------------------

/// ANSI virtual key codes.
fn keycode(key: ShortcutKey) -> u16 {
    match key {
        ShortcutKey::Letter(c) => match c {
            'A' => 0,
            'S' => 1,
            'D' => 2,
            'F' => 3,
            'H' => 4,
            'G' => 5,
            'Z' => 6,
            'X' => 7,
            'C' => 8,
            'V' => 9,
            'B' => 11,
            'Q' => 12,
            'W' => 13,
            'E' => 14,
            'R' => 15,
            'Y' => 16,
            'T' => 17,
            'O' => 31,
            'U' => 32,
            'I' => 34,
            'P' => 35,
            'L' => 37,
            'J' => 38,
            'K' => 40,
            'N' => 45,
            _ => 46, // M
        },
        ShortcutKey::Digit(c) => match c {
            '1' => 18,
            '2' => 19,
            '3' => 20,
            '4' => 21,
            '5' => 23,
            '6' => 22,
            '7' => 26,
            '8' => 28,
            '9' => 25,
            _ => 29, // 0
        },
        ShortcutKey::Function(n) => match n {
            1 => 122,
            2 => 120,
            3 => 99,
            4 => 118,
            5 => 96,
            6 => 97,
            7 => 98,
            8 => 100,
            9 => 101,
            10 => 109,
            11 => 103,
            _ => 111, // F12
        },
    }
}

fn flags_of(shortcut: &Shortcut) -> u64 {
    let m = &shortcut.mods;
    [
        (m.ctrl, ffi::FLAG_CONTROL),
        (m.shift, ffi::FLAG_SHIFT),
        (m.alt, ffi::FLAG_ALT),
        (m.meta, ffi::FLAG_COMMAND),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .fold(0, |acc, (_, flag)| acc | flag)
}

const MODIFIER_MASK: u64 = ffi::FLAG_CONTROL | ffi::FLAG_SHIFT | ffi::FLAG_ALT | ffi::FLAG_COMMAND;
const KEYCODE_CTRL_LEFT: i64 = 59;
const KEYCODE_CTRL_RIGHT: i64 = 62;

fn start_event_tap() {
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
            let hit = STATE.with(|s| {
                let s = s.try_borrow().ok()?;
                let state = s.as_ref()?;
                state
                    .hotkeys
                    .iter()
                    .find(|(k, f, _)| *k as i64 == code && flags & MODIFIER_MASK == *f)
                    .map(|(_, _, ime)| *ime)
            });
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

// ---- actions and state -------------------------------------------------------------------------

enum Action {
    Set(Ime),
    Toggle,
    /// Show the HUD without changing anything.
    Show,
}

/// Performs an action on the main queue, after the current callback returns. The event tap calls
/// this: every key waits while the tap callback runs, and an action can block for up to the
/// Mingkuai call timeout when its server is busy.
fn act_later(action: Action) {
    extern "C" fn run(context: *mut c_void) {
        // SAFETY: `context` is the box made below, and the queue runs this once.
        let action = unsafe { Box::from_raw(context.cast::<Action>()) };
        act(*action);
    }
    let context = Box::into_raw(Box::new(action)).cast::<c_void>();
    // SAFETY: the main queue takes the box and gives it back to `run`.
    unsafe { ffi::dispatch_async_f(&raw const ffi::_dispatch_main_q, context, run) };
}

/// Performs an action and always shows the result.
fn act(action: Action) {
    STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else {
            return;
        };
        let Some(state) = s.as_mut() else { return };
        let result = match action {
            Action::Set(ime) => state.engine.set(ime),
            Action::Toggle => state.engine.toggle(),
            Action::Show => Ok(()),
        };
        if let Err(e) = &result {
            eprintln!("imetap: {e:#}");
        }
        let snapshot = state.engine.snapshot();
        debug::log(|| {
            format!(
                "action {} error={:?} snapshot={snapshot:?}",
                match action {
                    Action::Set(ime) => format!("set {ime:?}"),
                    Action::Toggle => "toggle".to_string(),
                    Action::Show => "show".to_string(),
                },
                result.as_ref().err().map(|e| format!("{e:#}"))
            )
        });
        set_last(state, snapshot);
        show_hud(state, snapshot);
    });
}

/// Records the last state seen, and shows "E" in the menu bar icon while it is English.
fn set_last(state: &mut State, snapshot: Snapshot) {
    let english = is_english(snapshot);
    if state.last.map(is_english) != Some(english)
        && let Some(button) = state.status_item.button(state.mtm)
    {
        button.setImage(Some(&status_image(english)));
    }
    state.last = Some(snapshot);
}

/// True for the US layout and for the English mode of a Chinese input method.
fn is_english(snapshot: Snapshot) -> bool {
    snapshot.ime == Ime::Us || snapshot.chinese == Some(false)
}

/// Reads the state. Shows the HUD only if it differs from the last one seen, or if `force`.
fn probe(show_if_changed: bool, force: bool) {
    STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else {
            return;
        };
        let Some(state) = s.as_mut() else { return };
        let snapshot = state.engine.snapshot();
        let changed = state.last != Some(snapshot);
        let first = state.last.is_none();
        set_last(state, snapshot);
        let show =
            force || (changed && show_if_changed && !first && state.config.hud.on_window_switch);
        debug::log(|| {
            format!("probe force={force} changed={changed} snapshot={snapshot:?} -> show={show}")
        });
        if show {
            show_hud(state, snapshot);
        }
    });
}

/// Watches the mode of Mingkuai, which changes inside Mingkuai (for example with Shift) without any
/// system event. Each change wakes the main thread. When the server is not running, it tries again
/// every few seconds.
fn spawn_mingkuai_watcher() {
    thread::spawn(|| {
        loop {
            // The first message is the mode at the time of the connection, not a change.
            let mut first = true;
            let result = mingkuai::watch(|chinese| {
                debug::log(|| format!("mingkuai mode chinese={chinese} first={first}"));
                if !std::mem::replace(&mut first, false) {
                    // SAFETY: queues a plain function with no data on the main queue.
                    unsafe {
                        ffi::dispatch_async_f(
                            &raw const ffi::_dispatch_main_q,
                            ptr::null_mut(),
                            mingkuai_mode_changed,
                        )
                    };
                }
            });
            if let Err(e) = result {
                debug::log(|| format!("mingkuai watch: {e:#}"));
            }
            thread::sleep(Duration::from_secs(5));
        }
    });
}

/// Runs on the main thread after the mode of Mingkuai changed. Shows the HUD if Mingkuai is the
/// active input method.
extern "C" fn mingkuai_mode_changed(_: *mut c_void) {
    STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else {
            return;
        };
        let Some(state) = s.as_mut() else { return };
        let snapshot = state.engine.snapshot();
        let show = snapshot.ime == Ime::Mingkuai && state.last != Some(snapshot);
        debug::log(|| format!("mingkuai mode changed snapshot={snapshot:?} -> show={show}"));
        set_last(state, snapshot);
        if show {
            show_hud(state, snapshot);
        }
    });
}

extern "C" fn probe_timer_fired(_: CFRunLoopTimerRef, _: *mut c_void) {
    probe(true, REMIND_PENDING.replace(false));
}

/// Shows the HUD once the state has settled, whatever it is.
fn remind() {
    REMIND_PENDING.set(true);
    schedule_probe();
}

/// Feeds key or mouse activity to the reminder.
fn note_activity() {
    let resumed = REMINDER.with(|r| {
        r.borrow_mut()
            .as_mut()
            .is_some_and(|r| r.activity(now_ms()))
    });
    if resumed {
        remind();
    }
}

/// Feeds a change of the frontmost application to the reminder, then reads the state.
fn focus_changed() {
    let due = REMINDER.with(|r| {
        r.borrow_mut()
            .as_mut()
            .is_some_and(|r| r.focus_changed(now_ms()))
    });
    if due {
        remind();
    } else {
        schedule_probe();
    }
}

/// Debounces a burst of focus / input-source notifications into one probe.
fn schedule_probe() {
    STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else {
            return;
        };
        let Some(state) = s.as_mut() else { return };
        if let Some(old) = state.probe_timer.take() {
            // SAFETY: invalidating a timer we created. The run loop releases it.
            unsafe { ffi::CFRunLoopTimerInvalidate(old) };
        }
        state.probe_timer = Some(schedule(DEBOUNCE_SECS, probe_timer_fired));
    });
}

/// Fires `callback` once after `seconds` on the main run loop.
fn schedule(
    seconds: f64,
    callback: extern "C" fn(CFRunLoopTimerRef, *mut c_void),
) -> CFRunLoopTimerRef {
    // SAFETY: creates a one-shot timer on the main run loop.
    unsafe {
        let timer = ffi::CFRunLoopTimerCreate(
            ptr::null(),
            ffi::CFAbsoluteTimeGetCurrent() + seconds,
            0.0,
            0,
            0,
            callback,
            ptr::null_mut(),
        );
        ffi::CFRunLoopAddTimer(ffi::CFRunLoopGetMain(), timer, ffi::kCFRunLoopCommonModes);
        timer
    }
}

extern "C" fn input_source_changed(
    _: *const c_void,
    _: *mut c_void,
    _: ffi::CFStringRef,
    _: *const c_void,
    _: *const c_void,
) {
    schedule_probe();
}

fn observe_changes() {
    let mtm = MainThreadMarker::new().expect("main thread");
    // SAFETY: registers a C callback with the distributed notification center and a block with the
    // workspace. Both live for the whole process.
    unsafe {
        let name = ffi::cfstring(
            &CString::new("com.apple.Carbon.TISNotifySelectedKeyboardInputSourceChanged").unwrap(),
        );
        ffi::CFNotificationCenterAddObserver(
            ffi::CFNotificationCenterGetDistributedCenter(),
            ptr::null(),
            input_source_changed,
            name,
            ptr::null(),
            4,
        );

        let block = RcBlock::new(|_: NonNull<NSNotification>| focus_changed());
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        let observer = center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidActivateApplicationNotification),
            None,
            None,
            &block,
        );
        std::mem::forget(observer);
        let _ = mtm;
    }
}

// ---- status item -------------------------------------------------------------------------------

/// The menu bar icon: "中", or "E" while the state is English, knocked out of a rounded square,
/// like the Windows tray icon. It is a template image, so macOS tints it for light and dark menu
/// bars. It shows only Chinese or English: the system input menu shows the input method, but not
/// the mode of an input method such as Mingkuai.
fn status_image(english: bool) -> Retained<NSImage> {
    const SIZE: f64 = 18.0;
    let draw = RcBlock::new(move |_: NSRect| -> Bool {
        let square = NSRect::new(NSPoint::new(1.0, 1.0), NSSize::new(SIZE - 2.0, SIZE - 2.0));
        NSColor::blackColor().setFill();
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(square, 4.0, 4.0).fill();
        if let Some(context) = NSGraphicsContext::currentContext() {
            context.setCompositingOperation(NSCompositingOperation::DestinationOut);
        }
        let font = NSFont::boldSystemFontOfSize(12.5);
        let color = NSColor::blackColor();
        // SAFETY: the attribute keys are AppKit constants and the values have the types they expect.
        let attributes: Retained<NSDictionary<NSAttributedStringKey, AnyObject>> = unsafe {
            NSDictionary::from_slices(
                &[NSFontAttributeName, NSForegroundColorAttributeName],
                &[&**font as &AnyObject, &**color as &AnyObject],
            )
        };
        let text = if english {
            ns_string!("E")
        } else {
            ns_string!("中")
        };
        // SAFETY: drawing into the image's current graphics context, which AppKit set up.
        unsafe {
            let size = text.sizeWithAttributes(Some(&attributes));
            let origin = NSPoint::new((SIZE - size.width) / 2.0, (SIZE - size.height) / 2.0);
            text.drawAtPoint_withAttributes(origin, Some(&attributes));
        }
        Bool::YES
    });
    let image =
        NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(SIZE, SIZE), false, &draw);
    image.setTemplate(true);
    image
}

fn make_status_item(mtm: MainThreadMarker) -> Retained<NSStatusItem> {
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0);
    if let Some(button) = item.button(mtm) {
        button.setImage(Some(&status_image(false)));
        button.setToolTip(Some(ns_string!("imetap")));
    }
    let target: Retained<MenuTarget> = unsafe { msg_send![mtm.alloc::<MenuTarget>(), init] };
    let menu = NSMenu::new(mtm);
    // A header that names the tool. AppKit greys out a menu item that has no action whatever its
    // title says, so the header is a plain label shown as the item's view, which also keeps it from
    // highlighting.
    let header = NSMenuItem::new(mtm);
    let name = NSTextField::labelWithString(ns_string!("imetap"), mtm);
    name.setFont(Some(
        &NSFont::boldSystemFontOfSize(NSFont::systemFontSize()),
    ));
    name.setTextColor(Some(&NSColor::labelColor()));
    name.sizeToFit();
    let size = name.frame().size;
    name.setFrameOrigin(NSPoint::new(HEADER_INSET, 3.0));
    let view = NSView::initWithFrame(
        mtm.alloc::<NSView>(),
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(size.width + 2.0 * HEADER_INSET, size.height + 6.0),
        ),
    );
    view.addSubview(&name);
    header.setView(Some(&view));
    menu.addItem(&header);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    for (title, action) in [
        ("Run at login", sel!(toggleLogin:)),
        ("Open config file", sel!(openConfig:)),
        ("Quit imetap", sel!(quit:)),
    ] {
        let entry = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc::<NSMenuItem>(),
                &NSString::from_str(title),
                Some(action),
                ns_string!(""),
            )
        };
        unsafe { entry.setTarget(Some(&target)) };
        menu.addItem(&entry);
    }
    item.setMenu(Some(&menu));
    // The menu items hold only weak references to their target.
    std::mem::forget(target);
    item
}

// ---- HUD ---------------------------------------------------------------------------------------

fn label(
    mtm: MainThreadMarker,
    frame: NSRect,
    size: f64,
    bold: bool,
    color: &NSColor,
) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(ns_string!(""), mtm);
    field.setFrame(frame);
    field.setAlignment(NSTextAlignment::Center);
    field.setTextColor(Some(color));
    let font = if bold {
        NSFont::boldSystemFontOfSize(size)
    } else {
        NSFont::systemFontOfSize(size)
    };
    field.setFont(Some(&font));
    field
}

fn make_hud(mtm: MainThreadMarker) -> Hud {
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(HUD_WIDTH, HUD_HEIGHT));
    let panel = {
        NSPanel::initWithContentRect_styleMask_backing_defer(
            mtm.alloc::<NSPanel>(),
            frame,
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    panel.setLevel(STATUS_WINDOW_LEVEL);
    panel.setOpaque(false);
    panel.setHasShadow(false);
    panel.setIgnoresMouseEvents(true);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    let glyph = label(
        mtm,
        NSRect::new(
            NSPoint::new(HUD_STRIPE, (HUD_HEIGHT - 66.0) / 2.0),
            NSSize::new(HUD_WIDTH - HUD_STRIPE, 66.0),
        ),
        52.0,
        true,
        &NSColor::whiteColor(),
    );
    let stripe = NSView::new(mtm);
    stripe.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(HUD_STRIPE, HUD_HEIGHT),
    ));
    stripe.setWantsLayer(true);
    let stripe_label = label(
        mtm,
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(HUD_STRIPE, 60.0)),
        40.0,
        true,
        &NSColor::blackColor(),
    );
    if let Some(view) = panel.contentView() {
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(24.0);
            // Clip the stripe to the rounded corners.
            layer.setMasksToBounds(true);
            layer.setBackgroundColor(Some(&ns_color(Color::Black.fill()).CGColor()));
        }
        view.addSubview(&stripe);
        view.addSubview(&stripe_label);
        view.addSubview(&glyph);
    }
    Hud {
        panel,
        glyph,
        stripe,
        stripe_label,
    }
}

fn ns_color([r, g, b]: Rgb) -> Retained<NSColor> {
    let channel = |v: u8| v as f64 / 255.0;
    NSColor::colorWithSRGBRed_green_blue_alpha(channel(r), channel(g), channel(b), 1.0)
}

/// One step of the fade: sets the HUD's brightness and arms the next step, until it is over. There
/// is at most one chain of steps. Showing the HUD again only replaces its `HudRun`.
extern "C" fn fade_step(timer: CFRunLoopTimerRef, _: *mut c_void) {
    // SAFETY: this one-shot timer was created by `schedule`, which hands its reference to the one
    // callback. Nothing else uses it.
    unsafe { ffi::CFRelease(timer) };
    STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else {
            return;
        };
        let Some(state) = s.as_mut() else { return };
        let Some(run) = state.hud_run else { return };
        let elapsed = run.started.elapsed().as_millis() as u64 + run.offset_ms;
        if run.fade.finished(elapsed) {
            state.hud.panel.orderOut(None);
            state.hud_level = 0.0;
            state.hud_run = None;
            return;
        }
        state.hud_level = run.fade.level(elapsed);
        state
            .hud
            .panel
            .setAlphaValue(state.config.hud.opacity as f64 * state.hud_level as f64);
        schedule(FADE_STEP_SECS, fade_step);
    });
}

fn show_hud(state: &mut State, snapshot: Snapshot) {
    if !state.config.hud.enabled {
        return;
    }
    REMINDER.with(|r| {
        if let Some(r) = r.borrow_mut().as_mut() {
            r.hud_shown(now_ms());
        }
    });
    let hud = &state.hud;
    let style = state.config.hud.style(snapshot);
    let colors = style.colors;
    if let Some(layer) = hud.panel.contentView().and_then(|view| view.layer()) {
        layer.setBackgroundColor(Some(&ns_color(colors.background).CGColor()));
    }
    if let Some(layer) = hud.stripe.layer() {
        layer.setBackgroundColor(Some(&ns_color(colors.stripe).CGColor()));
    }
    hud.glyph.setTextColor(Some(&ns_color(colors.glyph)));
    hud.glyph
        .setStringValue(&NSString::from_str(snapshot.glyph()));
    // A one-character label is bigger than a two-character one. The field is centred on the stripe.
    let size = if style.label.chars().count() > 1 {
        24.0
    } else {
        34.0
    };
    let height = size * 1.4;
    hud.stripe_label
        .setFont(Some(&NSFont::boldSystemFontOfSize(size)));
    hud.stripe_label.setFrame(NSRect::new(
        NSPoint::new(0.0, (HUD_HEIGHT - height) / 2.0),
        NSSize::new(HUD_STRIPE, height),
    ));
    hud.stripe_label
        .setTextColor(Some(&ns_color(colors.stripe_text)));
    hud.stripe_label
        .setStringValue(&NSString::from_str(&style.label));
    // The screen of the window with keyboard focus. The first (primary) screen if there is none.
    let screen =
        NSScreen::mainScreen(state.mtm).or_else(|| NSScreen::screens(state.mtm).firstObject());
    if let Some(screen) = screen {
        let area = screen.visibleFrame();
        let origin = NSPoint::new(
            area.origin.x + (area.size.width - HUD_WIDTH) / 2.0,
            area.origin.y + HUD_BOTTOM_MARGIN,
        );
        hud.panel.setFrameOrigin(origin);
    }
    // Showing again while still visible carries on from the current brightness.
    let fade = state.config.hud.fade();
    let offset_ms = if hud.panel.isVisible() {
        fade.start_for(state.hud_level)
    } else {
        0
    };
    state.hud_level = fade.level(offset_ms);
    hud.panel
        .setAlphaValue(state.config.hud.opacity as f64 * state.hud_level as f64);
    hud.panel.orderFrontRegardless();
    let running = state.hud_run.is_some();
    state.hud_run = Some(HudRun {
        fade,
        started: Instant::now(),
        offset_ms,
    });
    if !running {
        schedule(FADE_STEP_SECS, fade_step);
    }
}
