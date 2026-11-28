//! The resident macOS tool: the main run loop, the actions and the reading of the state. The event
//! taps, the hotkeys, the status item and the HUD are in `hooks`, `hotkeys`, `tray` and `hud`.
//!
//! Everything runs on the main run loop, apart from the thread that watches the mode of Mingkuai.

use std::cell::{Cell, RefCell};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSStatusItem};
use objc2_foundation::MainThreadMarker;

use super::ffi::{self, Timer};
use super::hooks::{HOTKEYS, TAP, start_event_tap};
use super::hud::{FADE_STEP_SECS, Hud};
use super::{Engine, claim_pid, hotkeys, mingkuai, tray};
use crate::config::Config;
use crate::debug;
use crate::protocol::mingkuai as mingkuai_wire;
use crate::reminder::{Reminder, now_ms};
use crate::state::{Ime, Snapshot};
use crate::tap::TapDetector;

/// How long input-source and focus notifications may settle before the state is read.
const DEBOUNCE_SECS: f64 = 0.25;

struct State {
    mtm: MainThreadMarker,
    config: Config,
    engine: Engine,
    hud: Hud,
    status_item: Retained<NSStatusItem>,
    last: Option<Snapshot>,
    probe_timer: Option<Timer>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    /// `None` when the reminder is off. The event taps touch it, so it is kept out of `STATE`.
    pub(super) static REMINDER: RefCell<Option<Reminder>> = const { RefCell::new(None) };
    /// The next probe shows the HUD even if nothing changed.
    static REMIND_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// Runs the tool until the user quits from the status item menu.
pub fn run(config: Config) -> Result<()> {
    claim_pid()?;
    let mtm = MainThreadMarker::new().expect("run must be called on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    HOTKEYS.with(|h| *h.borrow_mut() = hotkeys::from_config(&config));
    let tap = TapDetector::new(config.tap.gap_ms, config.tap.hold_ms, config.tap.triple_ms);
    TAP.with(|t| *t.borrow_mut() = tap);
    let r = &config.reminder;
    if r.enabled {
        let reminder = Reminder::new(r.mode, r.idle_secs, r.dwell_secs, now_ms());
        REMINDER.with(|cell| *cell.borrow_mut() = Some(reminder));
    }

    let status_item = tray::make_status_item(mtm, config.tray.enable_lang_indicator);
    let hud = Hud::new(mtm);
    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            mtm,
            config,
            engine: Engine::new(),
            hud,
            status_item,
            last: None,
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

/// What a hotkey or a Ctrl tap asks for.
pub(super) enum Action {
    Set(Ime),
    Toggle,
    /// Show the HUD without changing anything.
    Show,
}

/// Performs an action on the main queue, after the current callback returns. The event tap calls
/// this: every key waits while the tap callback runs, and an action can block for up to the
/// Mingkuai call timeout when its server is busy.
pub(super) fn act_later(action: Action) {
    ffi::run_on_main(move || act(action));
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

/// Records the last state seen, and shows "E" in the menu bar icon while it is English (unless the
/// language indicator is off).
fn set_last(state: &mut State, snapshot: Snapshot) {
    let english = is_english(snapshot);
    if state.config.tray.enable_lang_indicator && state.last.map(is_english) != Some(english) {
        tray::show_lang(&state.status_item, state.mtm, english);
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
                    ffi::run_on_main(mingkuai_mode_changed);
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
fn mingkuai_mode_changed() {
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

fn probe_timer_fired() {
    probe(true, REMIND_PENDING.replace(false));
}

/// Shows the HUD once the state has settled, whatever it is.
fn remind() {
    REMIND_PENDING.set(true);
    schedule_probe();
}

/// Feeds key or mouse activity to the reminder.
pub(super) fn note_activity() {
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
            old.cancel();
        }
        state.probe_timer = Some(ffi::schedule(DEBOUNCE_SECS, probe_timer_fired));
    });
}

/// Watches for changes of the input source and of the frontmost application.
fn observe_changes() {
    ffi::on_input_source_change(schedule_probe);
    ffi::on_app_activation(focus_changed);
}

/// Shows the HUD, unless it is off, and starts its fade.
fn show_hud(state: &mut State, snapshot: Snapshot) {
    if !state.config.hud.enabled {
        return;
    }
    REMINDER.with(|r| {
        if let Some(r) = r.borrow_mut().as_mut() {
            r.hud_shown(now_ms());
        }
    });
    if state.hud.show(state.mtm, &state.config.hud, snapshot) {
        ffi::schedule_once(FADE_STEP_SECS, fade_step);
    }
}

/// One step of the HUD's fade, which arms the next step until the fade is over. There is at most
/// one chain of steps. Showing the HUD again only restarts the fade it steps.
fn fade_step() {
    STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else {
            return;
        };
        let Some(state) = s.as_mut() else { return };
        if state.hud.step(&state.config.hud) {
            ffi::schedule_once(FADE_STEP_SECS, fade_step);
        }
    });
}
