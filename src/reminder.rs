//! The reminder: shows the HUD again when the user may have forgotten the input method.
//!
//! In [`ReminderMode::Resume`] the HUD is shown when the user comes back after the computer sat
//! idle, that is after no key, mouse or focus-window activity for `idle_secs`. In
//! [`ReminderMode::ResumeAndSwitch`] it is also shown when the user leaves a window they stayed in
//! for at least `dwell_secs` without the HUD appearing meanwhile, whether or not the input method
//! differs in the next window.
//!
//! Like `tap`, it is a pure state machine fed with timestamps in milliseconds, shared by both
//! platforms and tested without either.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

/// What, besides coming back after idling, shows the HUD.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ReminderMode {
    /// Only coming back after idling.
    #[default]
    Resume,
    /// Also leaving a window used for a long time without seeing the HUD.
    ResumeAndSwitch,
}

/// The reminder's state.
#[derive(Debug, Clone)]
pub struct Reminder {
    mode: ReminderMode,
    idle_ms: u64,
    dwell_ms: u64,
    last_activity: u64,
    /// When focus moved to the current window.
    window_since: u64,
    /// When the HUD was last shown.
    last_hud: Option<u64>,
}

impl Reminder {
    /// A reminder that treats `now` as the last activity and the time focus arrived.
    pub fn new(mode: ReminderMode, idle_secs: u64, dwell_secs: u64, now: u64) -> Self {
        Self {
            mode,
            idle_ms: idle_secs * 1000,
            dwell_ms: dwell_secs * 1000,
            last_activity: now,
            window_since: now,
            last_hud: None,
        }
    }

    /// Feeds key or mouse activity. Returns true when the user came back after idling.
    pub fn activity(&mut self, now: u64) -> bool {
        let resumed = now.saturating_sub(self.last_activity) >= self.idle_ms;
        self.last_activity = now;
        resumed
    }

    /// Feeds a change of the focused window, which is activity too. Returns true when the HUD
    /// should be shown regardless of whether the input method changed.
    pub fn focus_changed(&mut self, now: u64) -> bool {
        let resumed = self.activity(now);
        let dwelt = now.saturating_sub(self.window_since) >= self.dwell_ms
            && self.last_hud.is_none_or(|shown| shown < self.window_since);
        self.window_since = now;
        resumed || (self.mode == ReminderMode::ResumeAndSwitch && dwelt)
    }

    /// Records that the HUD was shown, for whatever reason.
    pub fn hud_shown(&mut self, now: u64) {
        self.last_hud = Some(now);
    }
}

/// Milliseconds of wall-clock time, the reminder's time base.
///
/// Unlike `Instant` on macOS, it keeps running while the computer sleeps, which is the most common
/// kind of idling. A clock set backwards only shortens one gap.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = 1000;

    fn resume() -> Reminder {
        Reminder::new(ReminderMode::Resume, 60, 300, 0)
    }

    fn switch() -> Reminder {
        Reminder::new(ReminderMode::ResumeAndSwitch, 60, 300, 0)
    }

    #[test]
    fn steady_activity_never_reminds() {
        let mut r = resume();
        for t in 1..200 {
            assert!(!r.activity(t * 10 * S));
        }
    }

    #[test]
    fn coming_back_after_idling_reminds_once() {
        let mut r = resume();
        assert!(!r.activity(10 * S));
        assert!(r.activity(70 * S));
        assert!(!r.activity(71 * S));
    }

    #[test]
    fn a_focus_change_after_idling_is_a_return() {
        let mut r = resume();
        assert!(!r.focus_changed(30 * S));
        assert!(r.focus_changed(100 * S));
    }

    #[test]
    fn resume_mode_ignores_long_stays() {
        let mut r = resume();
        for t in 1..40 {
            r.activity(t * 10 * S);
        }
        assert!(!r.focus_changed(400 * S));
    }

    #[test]
    fn leaving_a_long_used_window_reminds() {
        let mut r = switch();
        for t in 1..40 {
            r.activity(t * 10 * S);
        }
        assert!(r.focus_changed(400 * S));
        // The next window was used only briefly.
        assert!(!r.focus_changed(410 * S));
    }

    #[test]
    fn a_hud_during_the_stay_cancels_the_reminder() {
        let mut r = switch();
        r.focus_changed(10 * S);
        for t in 2..40 {
            r.activity(t * 10 * S);
        }
        r.hud_shown(100 * S);
        assert!(!r.focus_changed(400 * S));
    }

    #[test]
    fn a_hud_before_the_stay_does_not_count() {
        let mut r = switch();
        r.hud_shown(5 * S);
        r.focus_changed(10 * S);
        for t in 2..40 {
            r.activity(t * 10 * S);
        }
        assert!(r.focus_changed(400 * S));
    }

    #[test]
    fn modes_parse_from_kebab_case() {
        #[derive(Deserialize)]
        struct T {
            mode: ReminderMode,
        }
        let t: T = toml::from_str("mode = \"resume-and-switch\"").unwrap();
        assert_eq!(t.mode, ReminderMode::ResumeAndSwitch);
        let t: T = toml::from_str("mode = \"resume\"").unwrap();
        assert_eq!(t.mode, ReminderMode::Resume);
        assert!(toml::from_str::<T>("mode = \"never\"").is_err());
    }
}
