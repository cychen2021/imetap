//! The resident tool: the main window, its message loop and the worker thread. The hooks, the
//! hotkeys and the tray icon are in `hooks`, `hotkeys` and `tray`.
//!
//! Everything runs on one message-loop thread, driven by events. Nothing polls. Work that can
//! block (UI Automation, the Doubao pipe, `SendInput`) goes to a worker thread, which reports back
//! with a posted message.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowW, GetMessageW, HICON, KillTimer,
    MSG, PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SMTO_ABORTIFHUNG,
    SendMessageTimeoutW, SetTimer, TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_DESTROY,
    WM_HOTKEY, WM_LBUTTONUP, WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW, WS_OVERLAPPED,
};
use windows::core::{PCWSTR, w};

use super::hooks::{
    HOLD, HOLD_ENABLED, Hooks, MAIN, REMIND_PENDING, REMINDER, TAP, TAP_ENABLED, WAIT_FOR_TEXT,
};
use super::hotkeys::{HOTKEY_IMES, register_hotkeys, unregister_hotkeys};
use super::hud::Hud;
use super::{Engine, ime_hotkey, init_com, mingkuai, tray, uia};
use crate::config::Config;
use crate::debug;
use crate::hold::HeldKey;
use crate::protocol::mingkuai as mingkuai_wire;
use crate::reminder::{Reminder, now_ms};
use crate::state::{Ime, Snapshot, text_field_change};
use crate::tap::{Tap, TapDetector};

pub(super) const WM_TRAY: u32 = WM_APP + 1;
/// The worker finished a job and queued an outcome.
const WM_OUTCOME: u32 = WM_APP + 2;
/// Something happened that may have changed the input method (focus moved, IME notification).
pub(super) const WM_PASSIVE: u32 = WM_APP + 3;
/// The Ctrl double-tap was recognised.
pub(super) const WM_TAP: u32 = WM_APP + 4;
/// A Ctrl double-tap waits for a possible third tap.
pub(super) const WM_TAP_PENDING: u32 = WM_APP + 6;
/// The Ctrl triple-tap was recognised.
pub(super) const WM_TRIPLE_TAP: u32 = WM_APP + 7;
/// The reminder wants the HUD shown.
pub(super) const WM_REMIND: u32 = WM_APP + 5;

/// Held-back keys may be replayed: the input method has applied the switch, or the hold is over.
const WM_DRAIN: u32 = WM_APP + 8;
/// From the Mingkuai watcher thread: the mode of Mingkuai changed.
const WM_MINGKUAI: u32 = WM_APP + 9;

const TIMER_DEBOUNCE: usize = 1;
/// Ends the wait for a third Ctrl tap.
const TIMER_TRIPLE: usize = 2;
/// Ends a key hold that was never drained, so a failure cannot leave the keyboard dead.
pub(super) const TIMER_HOLD: usize = 3;
/// How long focus and IME notifications are allowed to settle before the state is read.
const DEBOUNCE_MS: u32 = 250;
/// How long the worker waits after an action before reading the state back.
const SETTLE: Duration = Duration::from_millis(150);
/// How the worker waits for a switch to take effect: the number of polls, the time between them
/// and the poll at which the request is sent once more.
const SWITCH_STEPS: u32 = 16;
const SWITCH_STEP: Duration = Duration::from_millis(50);
const SWITCH_RETRY_AT: u32 = 6;
/// How long the worker waits after a toggle before releasing the held-back keys, for the input
/// method to apply it.
const APPLY: Duration = Duration::from_millis(50);

/// Work for the worker thread.
enum Job {
    Set(Ime),
    Toggle,
    /// Just read the state.
    Probe,
    /// Read the state and whether focus is in a text field.
    Check,
    /// Read the state and always show it.
    Show,
    /// The mode of Mingkuai changed inside Mingkuai. Read the state and show it if Mingkuai is the
    /// active input method.
    MingkuaiMode,
    /// Replay keys that were held back.
    Replay(Vec<HeldKey>),
}

struct Outcome {
    snapshot: Snapshot,
    /// Show the HUD even if the state did not change.
    force: bool,
    /// Whether focus is in a text field. `true` when it was not asked for or cannot be told.
    text_focus: bool,
    error: Option<String>,
}

struct App {
    config: Config,
    hud: Hud,
    jobs: Sender<Job>,
    outcomes: Arc<Mutex<VecDeque<Outcome>>>,
    last: Option<Snapshot>,
    /// The state when focus was last in a text field. Only that counts as a change worth a HUD.
    last_text: Option<Snapshot>,
    tray_icon: Option<HICON>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// The id of the `TaskbarCreated` broadcast, sent when Explorer (re)starts its taskbar.
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    /// The id of the message `status` sends to ask whether the debug log is on.
    static DEBUG_QUERY_MESSAGE: Cell<u32> = const { Cell::new(0) };
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
    if mingkuai_wire::enabled() {
        spawn_mingkuai_watcher(hwnd.0 as isize);
    }

    let tap = TapDetector::new(config.tap.gap_ms, config.tap.hold_ms, config.tap.triple_ms);
    TAP.with(|t| *t.borrow_mut() = tap);
    TAP_ENABLED.with(|t| t.set(config.tap.enabled));
    HOLD_ENABLED.with(|h| h.set(config.tap.hold_input));
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
            last_text: None,
            tray_icon: None,
        })
    });
    // SAFETY: registers a window message by name.
    TASKBAR_CREATED.with(|m| m.set(unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }));
    // SAFETY: registers a window message by name.
    DEBUG_QUERY_MESSAGE.with(|m| m.set(unsafe { RegisterWindowMessageW(DEBUG_QUERY) }));
    update_tray(hwnd, None, true);
    send(Job::Probe);

    // SAFETY: the standard message loop on the thread that owns the windows.
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    unregister_hotkeys(hwnd, registered);
    drop(hooks);
    tray::remove(hwnd);
    Ok(())
}

fn send(job: Job) {
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            let _ = app.jobs.send(job);
        }
    });
}

/// Watches the mode of Mingkuai, which changes inside Mingkuai (for example with Shift) without
/// any system event. Each change becomes `Job::MingkuaiMode`. When the server is not running, it
/// tries again every few seconds.
fn spawn_mingkuai_watcher(main: isize) {
    thread::spawn(move || {
        loop {
            // The first message is the mode at the time of the connection, not a change.
            let mut first = true;
            let result = mingkuai::watch(|chinese| {
                debug::log(|| format!("mingkuai mode chinese={chinese} first={first}"));
                if !std::mem::replace(&mut first, false) {
                    // SAFETY: posting to the main window handle, which outlives this thread.
                    unsafe {
                        PostMessageW(
                            Some(HWND(main as *mut _)),
                            WM_MINGKUAI,
                            WPARAM(0),
                            LPARAM(0),
                        )
                        .ok()
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

fn spawn_worker(rx: Receiver<Job>, outcomes: Arc<Mutex<VecDeque<Outcome>>>, main: isize) {
    thread::spawn(move || {
        init_com();
        let engine = Engine::new();
        let post = |message: u32| {
            // SAFETY: posting to the main window handle, which outlives the worker.
            unsafe { PostMessageW(Some(HWND(main as *mut _)), message, WPARAM(0), LPARAM(0)).ok() };
        };
        for job in rx {
            let label = match &job {
                Job::Set(ime) => format!("set {ime:?}"),
                Job::Toggle => "toggle".to_string(),
                Job::Show => "show".to_string(),
                Job::MingkuaiMode => "mingkuai mode".to_string(),
                Job::Probe => "probe".to_string(),
                Job::Check => "check".to_string(),
                Job::Replay(keys) => format!("replay {} keys", keys.len()),
            };
            let toggled = matches!(job, Job::Toggle);
            let target = match &job {
                Job::Set(ime) => Some(*ime),
                _ => None,
            };
            let check = matches!(job, Job::Check);
            let mingkuai_mode = matches!(job, Job::MingkuaiMode);
            let (action, force) = match job {
                Job::Set(ime) => (Some(engine.set(ime)), true),
                Job::Toggle => (Some(engine.toggle()), true),
                Job::Show => (None, true),
                Job::Probe | Job::Check | Job::MingkuaiMode => (None, false),
                Job::Replay(keys) => {
                    if let Err(e) = ime_hotkey::replay(&keys) {
                        eprintln!("imetap: {e:#}");
                    }
                    post(WM_DRAIN);
                    continue;
                }
            };
            if toggled {
                // The toggle is in. Let the input method apply it, then release the held-back keys.
                thread::sleep(APPLY);
                post(WM_DRAIN);
            }
            let error = action.and_then(Result::err).map(|e| format!("{e:#}"));
            if let Some(target) = target {
                // A window can take a while to follow a switch, or miss the request. Wait until the
                // input method is the one asked for, ask again once, and only then read the state,
                // so that the HUD shows what is in effect and not what was before. Give up when
                // another window comes to the front, and never wait less than a plain settle, since
                // the mode of Microsoft Pinyin shows up on the taskbar later than the switch itself.
                let started = Instant::now();
                let window = engine.foreground();
                for step in 0..SWITCH_STEPS {
                    if engine.ime() == target || engine.foreground() != window {
                        break;
                    }
                    if step == SWITCH_RETRY_AT {
                        let _ = engine.set(target);
                    }
                    thread::sleep(SWITCH_STEP);
                }
                thread::sleep(SETTLE.saturating_sub(started.elapsed()));
            } else if force {
                // Let the input method apply the change before reading it back.
                thread::sleep(SETTLE);
            }
            let snapshot = engine.snapshot();
            let force = force || (mingkuai_mode && snapshot.ime == Ime::Mingkuai);
            let (text_focus, focus) = if check {
                engine.focus_in_text_field()
            } else {
                (true, None)
            };
            if force {
                debug::log(|| {
                    format!(
                        "job {label} window={} error={error:?} snapshot={snapshot:?}",
                        uia::foreground_class()
                    )
                });
            }
            if check {
                debug::log(|| {
                    format!(
                        "check window={} snapshot={snapshot:?} text_focus={text_focus} focus={focus:?}",
                        uia::foreground_class()
                    )
                });
            }
            let outcome = Outcome {
                snapshot,
                force,
                text_focus,
                error,
            };
            outcomes.lock().unwrap().push_back(outcome);
            post(WM_OUTCOME);
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

/// The name of the window message that asks a running instance whether its debug log is on.
const DEBUG_QUERY: PCWSTR = w!("imetap_debug_query");

/// Asks the running instance whether it writes the debug log, which depends on the environment it
/// was started in. `None` when none runs or it does not answer.
pub fn running_debug_log() -> Option<bool> {
    let hwnd = find_running()?;
    let mut answer = 0usize;
    // SAFETY: registers a window message by name and sends it to a window of another process with
    // a timeout, so that a hung instance cannot block the caller.
    let sent = unsafe {
        SendMessageTimeoutW(
            hwnd,
            RegisterWindowMessageW(DEBUG_QUERY),
            WPARAM(0),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            500,
            Some(&mut answer),
        )
    };
    (sent.0 != 0).then_some(answer != 0)
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

/// Replays what was held back, or, when nothing is left, ends the hold. Called again after every
/// replay, so keys typed meanwhile also stay in order.
fn drain(hwnd: HWND) {
    let held = HOLD.with(|h| h.borrow_mut().take());
    if held.is_empty() {
        HOLD.with(|h| h.borrow_mut().stop());
        // SAFETY: stops the timer set in `begin_hold`. Fails harmlessly when it is not running.
        unsafe { KillTimer(Some(hwnd), TIMER_HOLD).ok() };
    } else {
        send(Job::Replay(held));
    }
}

/// Adds the tray icon (`add`) or brings it up to date, and frees the icon it replaces.
fn update_tray(hwnd: HWND, snapshot: Option<Snapshot>, add: bool) {
    let lang_indicator = APP.with(|a| {
        a.borrow()
            .as_ref()
            .is_none_or(|app| app.config.tray.enable_lang_indicator)
    });
    let icon = tray::show(hwnd, snapshot, lang_indicator, add);
    let old = APP.with(|a| {
        a.borrow_mut()
            .as_mut()
            .and_then(|app| app.tray_icon.replace(icon))
    });
    if let Some(old) = old {
        tray::free_icon(old);
    }
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
            let hud = &app.config.hud;
            let wait = hud.wait_for_text_field && hud.on_window_switch;
            let wanted = if outcome.force {
                app.last_text = Some(snapshot);
                WAIT_FOR_TEXT.with(|w| w.set(false));
                true
            } else if wait {
                // Only a text field counts, and only against the last text field.
                WAIT_FOR_TEXT.with(|w| w.set(!outcome.text_focus));
                text_field_change(&mut app.last_text, snapshot, outcome.text_focus)
            } else {
                changed && !first && hud.on_window_switch
            };
            debug::log(|| {
                format!(
                    "outcome force={} text_focus={} last_text={:?} -> show={wanted}",
                    outcome.force, outcome.text_focus, app.last_text
                )
            });
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
    if message != 0 && message == DEBUG_QUERY_MESSAGE.with(Cell::get) {
        return LRESULT(debug::enabled() as isize);
    }
    // At login this tool may start before the taskbar exists. The icon is added again when it appears.
    if message != 0 && message == TASKBAR_CREATED.with(Cell::get) {
        let last = APP.with(|a| a.borrow().as_ref().and_then(|app| app.last));
        update_tray(hwnd, last, true);
        return LRESULT(0);
    }
    match message {
        WM_HOTKEY => {
            if let Some(ime) = HOTKEY_IMES.get(wparam.0) {
                debug::log(|| format!("hotkey {ime:?}"));
                send(Job::Set(*ime));
            }
            LRESULT(0)
        }
        WM_TAP => {
            send(Job::Toggle);
            LRESULT(0)
        }
        WM_TAP_PENDING => {
            let wait = TAP.with(|t| t.borrow().triple_ms()) as u32;
            // SAFETY: (re)starts the triple-tap timer of this window.
            unsafe { SetTimer(Some(hwnd), TIMER_TRIPLE, wait, None) };
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_TRIPLE => {
            // SAFETY: stops the timer set above.
            unsafe { KillTimer(Some(hwnd), TIMER_TRIPLE).ok() };
            if TAP.with(|t| t.borrow_mut().expire()) == Tap::Double {
                send(Job::Toggle);
            }
            LRESULT(0)
        }
        WM_TRIPLE_TAP => {
            send(Job::Show);
            // Nothing is switched, so the held-back keys can go right away.
            drain(hwnd);
            LRESULT(0)
        }
        WM_DRAIN => {
            drain(hwnd);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_HOLD => {
            drain(hwnd);
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
            // A pending reminder shows the HUD whatever the state. It waits for the debounce so that
            // it reads the window that has just come to the front.
            let remind = REMIND_PENDING.with(|p| p.replace(false));
            let check = APP.with(|a| {
                a.borrow().as_ref().is_some_and(|app| {
                    app.config.hud.wait_for_text_field && app.config.hud.on_window_switch
                })
            });
            send(match (remind, check) {
                (true, _) => Job::Show,
                (false, true) => Job::Check,
                (false, false) => Job::Probe,
            });
            LRESULT(0)
        }
        WM_OUTCOME => {
            on_outcomes(hwnd);
            LRESULT(0)
        }
        WM_MINGKUAI => {
            send(Job::MingkuaiMode);
            LRESULT(0)
        }
        WM_TRAY => {
            match lparam.0 as u32 {
                WM_RBUTTONUP => tray::show_menu(hwnd),
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
