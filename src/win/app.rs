//! The resident tool: tray icon, hotkeys, Ctrl double-tap, window-change watching and the HUD.
//!
//! Everything runs on one message-loop thread, driven by events; nothing polls. Work that can
//! block (UI Automation, the Doubao pipe, `SendInput`) goes to a worker thread, which reports back
//! with a posted message.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fs;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::{
    COLORREF, ERROR_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT,
    WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC,
    DeleteObject, FillRect, GetDC, HGDIOBJ, ReleaseDC, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CallNextHookEx, CreateIconIndirect, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyIcon, DestroyMenu, DispatchMessageW, FindWindowW, GetCursorPos,
    GetMessageW, HHOOK, HICON, ICONINFO, KBDLLHOOKSTRUCT, KillTimer, MF_CHECKED, MF_SEPARATOR,
    MF_STRING, MSG, MSLLHOOKSTRUCT, PostMessageW, PostQuitMessage, RegisterClassExW,
    RegisterWindowMessageW, SW_SHOWNORMAL, SetForegroundWindow, SetTimer, SetWindowsHookExW,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_DESTROY, WM_HOTKEY,
    WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_NULL, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER, WM_XBUTTONDOWN, WNDCLASSEXW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR, w};

use super::hud::{Hud, draw_text};
use super::{Engine, autostart, init_com};
use crate::config::{self, Config, Key as ShortcutKey, Shortcut};
use crate::reminder::{Reminder, now_ms};
use crate::state::{Ime, Snapshot};
use crate::tap::{Key, TapDetector};

const WM_TRAY: u32 = WM_APP + 1;
/// The worker finished a job and queued an outcome.
const WM_OUTCOME: u32 = WM_APP + 2;
/// Something happened that may have changed the input method (focus moved, IME notification).
const WM_PASSIVE: u32 = WM_APP + 3;
/// The Ctrl double-tap was recognised.
const WM_TAP: u32 = WM_APP + 4;
/// The reminder wants the HUD shown.
const WM_REMIND: u32 = WM_APP + 5;

const TIMER_DEBOUNCE: usize = 1;
/// How long focus and IME notifications are allowed to settle before the state is read.
const DEBOUNCE_MS: u32 = 250;
/// How long the worker waits after an action before reading the state back.
const SETTLE: Duration = Duration::from_millis(150);

const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
const EVENT_OBJECT_IME_CHANGE: u32 = 0x8029;
const WINEVENT_OUTOFCONTEXT: u32 = 0;
const LLKHF_INJECTED: u32 = 0x10;
const LLMHF_INJECTED: u32 = 0x01;

const CMD_AUTOSTART: usize = 1;
const CMD_CONFIG: usize = 2;
const CMD_QUIT: usize = 3;

/// Work for the worker thread.
enum Job {
    Set(Ime),
    Toggle,
    /// Just read the state.
    Probe,
    /// Read the state and always show it.
    Show,
}

struct Outcome {
    snapshot: Snapshot,
    /// Show the HUD even if the state did not change.
    force: bool,
    error: Option<String>,
}

struct App {
    config: Config,
    hud: Hud,
    jobs: Sender<Job>,
    outcomes: Arc<Mutex<VecDeque<Outcome>>>,
    last: Option<Snapshot>,
    tray_icon: Option<HICON>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    // The hooks touch only these two, so a hook can never collide with a borrow of `APP`.
    static TAP: RefCell<TapDetector> = RefCell::new(TapDetector::default());
    static MAIN: Cell<isize> = const { Cell::new(0) };
    static TAP_ENABLED: Cell<bool> = const { Cell::new(false) };
    /// `None` when the reminder is off. Touched by the hooks, so kept out of `APP` like `TAP`.
    static REMINDER: RefCell<Option<Reminder>> = const { RefCell::new(None) };
    /// The next debounced read shows the HUD even if nothing changed.
    static REMIND_PENDING: Cell<bool> = const { Cell::new(false) };
    /// The id of the `TaskbarCreated` broadcast, sent when Explorer (re)starts its taskbar.
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

/// Runs the tool until the user quits from the tray menu.
pub fn run(config: Config) -> Result<()> {
    init_com();
    // SAFETY: a named mutex whose handle lives as long as the process.
    let _single = unsafe {
        let mutex = CreateMutexW(None, true, w!("Local\\imetap-single-instance"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            bail!("imetap is already running");
        }
        mutex
    };

    let hud = Hud::new()?;
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None)? }.into();
    let hwnd = create_window(instance)?;
    MAIN.with(|m| m.set(hwnd.0 as isize));

    let (jobs, rx) = channel();
    let outcomes = Arc::new(Mutex::new(VecDeque::new()));
    spawn_worker(rx, outcomes.clone(), hwnd.0 as isize);

    TAP.with(|t| *t.borrow_mut() = TapDetector::new(config.tap.gap_ms, config.tap.hold_ms));
    TAP_ENABLED.with(|t| t.set(config.tap.enabled));
    let r = &config.reminder;
    if r.enabled {
        let reminder = Reminder::new(r.mode, r.idle_secs, r.dwell_secs, now_ms());
        REMINDER.with(|cell| *cell.borrow_mut() = Some(reminder));
    }
    let hooks = Hooks::install(&config, instance);
    let registered = register_hotkeys(hwnd, &config);
    APP.with(|a| {
        *a.borrow_mut() = Some(App {
            config,
            hud,
            jobs,
            outcomes,
            last: None,
            tray_icon: None,
        })
    });
    // SAFETY: registers a window message by name.
    TASKBAR_CREATED.with(|m| m.set(unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }));
    add_tray(hwnd);
    send(Job::Probe);

    // SAFETY: the standard message loop on the thread that owns the windows.
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    for id in registered {
        // SAFETY: unregisters hotkeys registered above.
        unsafe { UnregisterHotKey(Some(hwnd), id).ok() };
    }
    drop(hooks);
    remove_tray(hwnd);
    Ok(())
}

fn send(job: Job) {
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            let _ = app.jobs.send(job);
        }
    });
}

fn spawn_worker(rx: Receiver<Job>, outcomes: Arc<Mutex<VecDeque<Outcome>>>, main: isize) {
    thread::spawn(move || {
        init_com();
        let engine = Engine::new();
        for job in rx {
            let (action, force) = match job {
                Job::Set(ime) => (Some(engine.set(ime)), true),
                Job::Toggle => (Some(engine.toggle()), true),
                Job::Show => (None, true),
                Job::Probe => (None, false),
            };
            let error = action.and_then(Result::err).map(|e| format!("{e:#}"));
            if force {
                // Let the input method apply the change before reading it back.
                thread::sleep(SETTLE);
            }
            let outcome = Outcome {
                snapshot: engine.snapshot(),
                force,
                error,
            };
            outcomes.lock().unwrap().push_back(outcome);
            // SAFETY: posting to the main window handle, which outlives the worker.
            unsafe {
                PostMessageW(Some(HWND(main as *mut _)), WM_OUTCOME, WPARAM(0), LPARAM(0)).ok()
            };
        }
    });
}

/// The window of a running instance, if any.
fn find_running() -> Option<HWND> {
    // SAFETY: looks a window up by class name.
    unsafe { FindWindowW(w!("imetap_main"), PCWSTR::null()).ok() }
}

/// Whether a resident instance is running.
pub fn is_running() -> bool {
    find_running().is_some()
}

/// Asks the running instance to quit. Returns false if none was running.
pub fn stop() -> Result<bool> {
    let Some(hwnd) = find_running() else {
        return Ok(false);
    };
    // SAFETY: WM_CLOSE makes the window destroy itself, which ends its message loop.
    unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0))? };
    Ok(true)
}

fn create_window(instance: HINSTANCE) -> Result<HWND> {
    // SAFETY: registers a class and creates a hidden top-level window for the tray and hotkeys.
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: w!("imetap_main"),
            ..Default::default()
        };
        RegisterClassExW(&class);
        Ok(CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("imetap_main"),
            w!("imetap"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )?)
    }
}

// ---- hooks -------------------------------------------------------------------------------------

/// The low-level and WinEvent hooks; removed on drop.
struct Hooks {
    keyboard: Option<HHOOK>,
    mouse: Option<HHOOK>,
    events: Vec<HWINEVENTHOOK>,
}

impl Hooks {
    fn install(config: &Config, instance: HINSTANCE) -> Hooks {
        // SAFETY: the hook procedures below are `extern "system"` functions that stay valid.
        unsafe {
            let (mut keyboard, mut mouse) = (None, None);
            // The reminder needs to see input too.
            if config.tap.enabled || config.reminder.enabled {
                keyboard =
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(instance), 0).ok();
                mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(instance), 0).ok();
            }
            let events = [EVENT_SYSTEM_FOREGROUND, EVENT_OBJECT_IME_CHANGE]
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

/// Feeds key or mouse activity to the reminder; cheap enough for every mouse move.
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

fn post_main(message: u32) {
    let hwnd = MAIN.with(Cell::get);
    // SAFETY: posting to the main window; failure just means it is gone.
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
        }
        if !injected && TAP_ENABLED.with(Cell::get) {
            // `time` is the event's own timestamp in ms, so a delayed hook call does not skew taps.
            let key = if matches!(info.vkCode, 0xA2 | 0xA3) {
                Key::Ctrl
            } else {
                Key::Other
            };
            let now = info.time as u64;
            let tapped = TAP.with(|t| match wparam.0 as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    t.borrow_mut().key_down(key, now);
                    false
                }
                WM_KEYUP | WM_SYSKEYUP => t.borrow_mut().key_up(key, now),
                _ => false,
            });
            if tapped {
                post_main(WM_TAP);
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
            TAP.with(|t| t.borrow_mut().pointer());
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

// ---- hotkeys -----------------------------------------------------------------------------------

fn virtual_key(key: ShortcutKey) -> u32 {
    match key {
        ShortcutKey::Letter(c) | ShortcutKey::Digit(c) => c as u32,
        ShortcutKey::Function(n) => 0x70 + n as u32 - 1,
    }
}

fn modifiers(shortcut: &Shortcut) -> HOT_KEY_MODIFIERS {
    let m = &shortcut.mods;
    let mut flags = MOD_NOREPEAT;
    for (on, flag) in [
        (m.ctrl, MOD_CONTROL),
        (m.shift, MOD_SHIFT),
        (m.alt, MOD_ALT),
        (m.meta, MOD_WIN),
    ] {
        if on {
            flags |= flag;
        }
    }
    flags
}

const HOTKEY_IMES: [Ime; 3] = [Ime::Us, Ime::Pinyin, Ime::Doubao];

/// Registers the configured hotkeys; returns the ids that succeeded. The id is the index in
/// `HOTKEY_IMES`.
fn register_hotkeys(hwnd: HWND, config: &Config) -> Vec<i32> {
    let mut registered = Vec::new();
    for (id, ime) in HOTKEY_IMES.into_iter().enumerate() {
        let Ok(Some(shortcut)) = config.shortcut(ime) else {
            continue;
        };
        // SAFETY: registers a hotkey for the main window.
        let result = unsafe {
            RegisterHotKey(
                Some(hwnd),
                id as i32,
                modifiers(&shortcut),
                virtual_key(shortcut.key),
            )
        };
        match result {
            Ok(()) => registered.push(id as i32),
            Err(e) => eprintln!("imetap: cannot register the hotkey for {}: {e}", ime.name()),
        }
    }
    registered
}

// ---- tray --------------------------------------------------------------------------------------

const TRAY_ID: u32 = 1;

fn tray_data(hwnd: HWND, icon: HICON, tip: &str) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    for (slot, unit) in data.szTip.iter_mut().zip(tip.encode_utf16().take(127)) {
        *slot = unit;
    }
    data
}

fn add_tray(hwnd: HWND) {
    update_tray(hwnd, None, true);
}

fn update_tray(hwnd: HWND, snapshot: Option<Snapshot>, add: bool) {
    // The tooltip is just the tool name: the state is already on the icon and in the input bar.
    let glyph = snapshot.map_or("?", |s| s.glyph());
    let icon = make_icon(glyph, snapshot);
    let data = tray_data(hwnd, icon, "imetap");
    // SAFETY: `data` is fully initialised for the call.
    unsafe {
        let _ = Shell_NotifyIconW(if add { NIM_ADD } else { NIM_MODIFY }, &data);
    }
    let old = APP.with(|a| {
        a.borrow_mut()
            .as_mut()
            .and_then(|app| app.tray_icon.replace(icon))
    });
    if let Some(old) = old {
        // SAFETY: the shell has switched to the new icon; the old handle is ours to free.
        unsafe { DestroyIcon(old).ok() };
    }
}

fn remove_tray(hwnd: HWND) {
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    };
    // SAFETY: removes the icon added by `add_tray`.
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

fn icon_color(snapshot: Option<Snapshot>) -> COLORREF {
    let rgb = |r: u32, g: u32, b: u32| COLORREF(r | g << 8 | b << 16);
    match snapshot {
        Some(s) if s.ime == Ime::Us => rgb(70, 74, 82),
        Some(s) if s.chinese == Some(true) => rgb(196, 48, 43),
        Some(_) => rgb(37, 99, 170),
        None => rgb(90, 90, 90),
    }
}

/// A 32 x 32 icon: the mode glyph in white on a colour that tells the mode at a glance.
fn make_icon(glyph: &str, snapshot: Option<Snapshot>) -> HICON {
    const SIZE: i32 = 32;
    // SAFETY: GDI objects created here are released before returning; the icon owns copies.
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let color = CreateCompatibleBitmap(screen, SIZE, SIZE);
        let mask = CreateBitmap(SIZE, SIZE, 1, 1, None);
        let old = SelectObject(mem, HGDIOBJ(color.0));
        let rect = RECT {
            left: 0,
            top: 0,
            right: SIZE,
            bottom: SIZE,
        };
        let brush = CreateSolidBrush(icon_color(snapshot));
        FillRect(mem, &rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
        SetBkMode(mem, TRANSPARENT);
        SetTextColor(mem, COLORREF(0x00FF_FFFF));
        draw_text(
            mem,
            glyph,
            rect,
            if glyph.is_ascii() { 17 } else { 26 },
            700,
        );
        SelectObject(mem, old);
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        icon
    }
}

fn show_menu(hwnd: HWND) {
    // SAFETY: builds, shows and destroys a popup menu on this thread. No `APP` borrow is held, since
    // the menu runs a nested message loop.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let autostart_flags = if autostart::is_enabled() {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        let _ = AppendMenuW(menu, autostart_flags, CMD_AUTOSTART, w!("Run at login"));
        let _ = AppendMenuW(menu, MF_STRING, CMD_CONFIG, w!("Open config file"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, w!("Quit"));
        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        // The menu closes on an outside click only if its owner is the foreground window.
        let _ = SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            at.x,
            at.y,
            None,
            hwnd,
            None,
        )
        .0 as usize;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match command {
            CMD_AUTOSTART => {
                if let Err(e) = autostart::set(!autostart::is_enabled()) {
                    eprintln!("imetap: cannot change run at login: {e:#}");
                }
            }
            CMD_CONFIG => {
                if let Err(e) = open_config() {
                    eprintln!("imetap: cannot open the config file: {e:#}");
                }
            }
            CMD_QUIT => PostQuitMessage(0),
            _ => {}
        }
    }
}

/// Opens `config.toml` in the default editor, creating a commented template first if needed.
fn open_config() -> Result<()> {
    let path = config::config_path().context("no configuration directory")?;
    if !path.exists() {
        fs::create_dir_all(path.parent().context("no parent directory")?)?;
        fs::write(&path, config::DEFAULT_TOML)?;
    }
    // SAFETY: shell-opens a file path; the HSTRING outlives the call.
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(path.as_os_str()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
    Ok(())
}

// ---- message handling --------------------------------------------------------------------------

fn on_outcomes(hwnd: HWND) {
    loop {
        let outcome = APP.with(|a| {
            a.borrow()
                .as_ref()
                .and_then(|app| app.outcomes.lock().unwrap().pop_front())
        });
        let Some(outcome) = outcome else { return };
        if let Some(error) = &outcome.error {
            eprintln!("imetap: {error}");
        }
        let snapshot = outcome.snapshot;
        let show = APP.with(|a| {
            let mut a = a.borrow_mut();
            let Some(app) = a.as_mut() else { return false };
            let changed = app.last != Some(snapshot);
            let first = app.last.is_none();
            app.last = Some(snapshot);
            let wanted = outcome.force || (changed && !first && app.config.hud.on_window_switch);
            if wanted && app.config.hud.enabled {
                app.hud.show(snapshot, &app.config.hud);
                REMINDER.with(|r| {
                    if let Some(r) = r.borrow_mut().as_mut() {
                        r.hud_shown(now_ms());
                    }
                });
            }
            changed
        });
        if show {
            update_tray(hwnd, Some(snapshot), false);
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // At login this tool may start before the taskbar exists; the icon is added again when it appears.
    if message != 0 && message == TASKBAR_CREATED.with(Cell::get) {
        let last = APP.with(|a| a.borrow().as_ref().and_then(|app| app.last));
        update_tray(hwnd, last, true);
        return LRESULT(0);
    }
    match message {
        WM_HOTKEY => {
            if let Some(ime) = HOTKEY_IMES.get(wparam.0) {
                send(Job::Set(*ime));
            }
            LRESULT(0)
        }
        WM_TAP => {
            send(Job::Toggle);
            LRESULT(0)
        }
        WM_REMIND => {
            REMIND_PENDING.with(|p| p.set(true));
            // SAFETY: restarts the debounce timer of this window.
            unsafe { SetTimer(Some(hwnd), TIMER_DEBOUNCE, DEBOUNCE_MS, None) };
            LRESULT(0)
        }
        WM_PASSIVE => {
            // SAFETY: restarts the debounce timer of this window.
            unsafe { SetTimer(Some(hwnd), TIMER_DEBOUNCE, DEBOUNCE_MS, None) };
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_DEBOUNCE => {
            // SAFETY: stops the timer set above.
            unsafe { KillTimer(Some(hwnd), TIMER_DEBOUNCE).ok() };
            // A pending reminder shows the HUD whatever the state; it waits for the debounce so that
            // it reads the window that has just come to the front.
            let remind = REMIND_PENDING.with(|p| p.replace(false));
            send(if remind { Job::Show } else { Job::Probe });
            LRESULT(0)
        }
        WM_OUTCOME => {
            on_outcomes(hwnd);
            LRESULT(0)
        }
        WM_TRAY => {
            match lparam.0 as u32 {
                WM_RBUTTONUP => show_menu(hwnd),
                WM_LBUTTONUP => send(Job::Show),
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: ends the message loop.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: forwarding an unhandled message with its original arguments.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
