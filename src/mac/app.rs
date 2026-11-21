//! The resident macOS tool: status item, hotkeys, Ctrl double-tap, input-source watching and HUD.
//!
//! Everything runs on the main run loop; there are no extra threads. The keyboard and mouse are
//! observed with a Quartz event tap, which needs the Accessibility permission (the same as
//! Hammerspoon). Until it is granted the tap creation is retried every few seconds.

use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_void};
use std::ptr::{self, NonNull};

use anyhow::Result;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSFont, NSMenu,
    NSMenuItem, NSPanel, NSScreen, NSStatusBar, NSStatusItem, NSTextAlignment, NSTextField,
    NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
    NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
    NSString, ns_string,
};

use super::ffi::{self, CFMachPortRef, CFRunLoopTimerRef, CGEventRef, CGEventTapProxy};
use super::{Engine, autostart_enabled, claim_pid, open, set_autostart};
use crate::config::{self, Config, Key as ShortcutKey, Shortcut};
use crate::state::{Ime, Snapshot};
use crate::tap::{Key, TapDetector};

/// How long input-source and focus notifications may settle before the state is read.
const DEBOUNCE_SECS: f64 = 0.25;
/// The HUD's size in points.
const HUD_WIDTH: f64 = 220.0;
const HUD_HEIGHT: f64 = 116.0;
const HUD_BOTTOM_MARGIN: f64 = 48.0;
/// `NSStatusWindowLevel`.
const STATUS_WINDOW_LEVEL: isize = 25;

const HOTKEY_IMES: [Ime; 3] = [Ime::Us, Ime::Shuangpin, Ime::Doubao];

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
            if let Err(e) = set_autostart(!autostart_enabled()) {
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
    caption: Retained<NSTextField>,
}

struct State {
    mtm: MainThreadMarker,
    config: Config,
    engine: Engine,
    hud: Hud,
    status_item: Retained<NSStatusItem>,
    last: Option<Snapshot>,
    hotkeys: Vec<(u16, u64, Ime)>,
    hide_timer: Option<CFRunLoopTimerRef>,
    probe_timer: Option<CFRunLoopTimerRef>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    // The tap callback touches these two only, so it never collides with a borrow of `STATE`.
    static TAP: RefCell<TapDetector> = RefCell::new(TapDetector::default());
    static TAP_PORT: Cell<usize> = const { Cell::new(0) };
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
    TAP.with(|t| *t.borrow_mut() = TapDetector::new(config.tap.gap_ms, config.tap.hold_ms));

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
            hide_timer: None,
            probe_timer: None,
        })
    });

    start_event_tap();
    observe_changes();
    probe(false);
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
        // Not trusted yet: show the system prompt and try again in a few seconds.
        ffi::ensure_accessibility();
        schedule(3.0, retry_tap);
        return;
    }
    TAP_PORT.with(|p| p.set(port as usize));
    // SAFETY: attaches the tap's mach port to the main run loop.
    unsafe {
        let source = ffi::CFMachPortCreateRunLoopSource(ptr::null(), port, 0);
        ffi::CFRunLoopAddSource(ffi::CFRunLoopGetMain(), source, ffi::kCFRunLoopCommonModes);
        ffi::CGEventTapEnable(port, true);
    }
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
            // The system switches a slow tap off; switch it back on.
            let port = TAP_PORT.with(Cell::get) as CFMachPortRef;
            // SAFETY: the port was created by `start_event_tap`.
            unsafe { ffi::CGEventTapEnable(port, true) };
        }
        ffi::FLAGS_CHANGED => {
            if code == KEYCODE_CTRL_LEFT || code == KEYCODE_CTRL_RIGHT {
                let tapped = TAP.with(|t| {
                    let mut t = t.borrow_mut();
                    if flags & ffi::FLAG_CONTROL != 0 {
                        t.key_down(Key::Ctrl, now);
                        false
                    } else {
                        t.key_up(Key::Ctrl, now)
                    }
                });
                if tapped {
                    act(Action::Toggle);
                }
            } else {
                TAP.with(|t| t.borrow_mut().key_down(Key::Other, now));
            }
        }
        ffi::KEY_DOWN => {
            TAP.with(|t| t.borrow_mut().key_down(Key::Other, now));
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
                act(Action::Set(ime));
                // Swallow the shortcut so the frontmost app never sees it.
                return ptr::null_mut();
            }
        }
        ffi::LEFT_MOUSE_DOWN | ffi::RIGHT_MOUSE_DOWN | ffi::OTHER_MOUSE_DOWN => {
            TAP.with(|t| t.borrow_mut().pointer())
        }
        _ => {}
    }
    event
}

// ---- actions and state -------------------------------------------------------------------------

enum Action {
    Set(Ime),
    Toggle,
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
        };
        if let Err(e) = result {
            eprintln!("imetap: {e:#}");
        }
        let snapshot = state.engine.snapshot();
        state.last = Some(snapshot);
        update_status_item(state, snapshot);
        show_hud(state, snapshot);
    });
}

/// Reads the state; shows the HUD only if it differs from the last one seen.
fn probe(show_if_changed: bool) {
    STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else {
            return;
        };
        let Some(state) = s.as_mut() else { return };
        let snapshot = state.engine.snapshot();
        let changed = state.last != Some(snapshot);
        let first = state.last.is_none();
        state.last = Some(snapshot);
        if changed {
            update_status_item(state, snapshot);
        }
        if changed && show_if_changed && !first && state.config.hud.on_window_switch {
            show_hud(state, snapshot);
        }
    });
}

extern "C" fn probe_timer_fired(_: CFRunLoopTimerRef, _: *mut c_void) {
    probe(true);
}

/// Debounces a burst of focus / input-source notifications into one probe.
fn schedule_probe() {
    STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else {
            return;
        };
        let Some(state) = s.as_mut() else { return };
        if let Some(old) = state.probe_timer.take() {
            // SAFETY: invalidating a timer we created; the run loop releases it.
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
    // workspace; both live for the whole process.
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

        let block = RcBlock::new(|_: NonNull<NSNotification>| schedule_probe());
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

fn make_status_item(mtm: MainThreadMarker) -> Retained<NSStatusItem> {
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0);
    if let Some(button) = item.button(mtm) {
        button.setTitle(ns_string!("?"));
    }
    let target: Retained<MenuTarget> = unsafe { msg_send![mtm.alloc::<MenuTarget>(), init] };
    let menu = NSMenu::new(mtm);
    for (title, action) in [
        ("Run at login", sel!(toggleLogin:)),
        ("Open config file", sel!(openConfig:)),
        ("Quit", sel!(quit:)),
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

fn update_status_item(state: &State, snapshot: Snapshot) {
    if let Some(button) = state.status_item.button(state.mtm) {
        button.setTitle(&NSString::from_str(snapshot.glyph()));
        button.setToolTip(Some(ns_string!("imetap")));
    }
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
        NSRect::new(NSPoint::new(0.0, 44.0), NSSize::new(HUD_WIDTH, 66.0)),
        52.0,
        true,
        &NSColor::whiteColor(),
    );
    let caption = label(
        mtm,
        NSRect::new(NSPoint::new(0.0, 12.0), NSSize::new(HUD_WIDTH, 22.0)),
        14.0,
        false,
        &NSColor::lightGrayColor(),
    );
    if let Some(view) = panel.contentView() {
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(24.0);
            layer.setBackgroundColor(Some(
                &NSColor::colorWithSRGBRed_green_blue_alpha(0.125, 0.13, 0.14, 1.0).CGColor(),
            ));
        }
        view.addSubview(&glyph);
        view.addSubview(&caption);
    }
    Hud {
        panel,
        glyph,
        caption,
    }
}

extern "C" fn hide_hud(_: CFRunLoopTimerRef, _: *mut c_void) {
    STATE.with(|s| {
        if let Some(state) = s.try_borrow().ok().as_deref().and_then(Option::as_ref) {
            state.hud.panel.orderOut(None);
        }
    });
}

fn show_hud(state: &mut State, snapshot: Snapshot) {
    if !state.config.hud.enabled {
        return;
    }
    let hud = &state.hud;
    hud.glyph
        .setStringValue(&NSString::from_str(snapshot.glyph()));
    hud.caption
        .setStringValue(&NSString::from_str(&snapshot.caption()));
    // The screen of the window with keyboard focus; the first (primary) screen if there is none.
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
    hud.panel.setAlphaValue(state.config.hud.opacity as f64);
    hud.panel.orderFrontRegardless();
    if let Some(old) = state.hide_timer.take() {
        // SAFETY: invalidating a timer we created.
        unsafe { ffi::CFRunLoopTimerInvalidate(old) };
    }
    state.hide_timer = Some(schedule(
        state.config.hud.duration_ms as f64 / 1000.0,
        hide_hud,
    ));
}
