//! Double- and triple-tap detection for the Ctrl key.
//!
//! The detector is a pure state machine fed with key events and timestamps in milliseconds, so it
//! is shared by the Windows keyboard hook and the macOS event tap and is tested without either.
//!
//! A double tap toggles Chinese/English and a triple tap shows the HUD. When triple taps are on,
//! the second tap cannot fire at once, since a third may follow: it reports [`Tap::Pending`], the
//! caller arms a timer of [`TapDetector::triple_ms`] and calls [`TapDetector::expire`] when it
//! fires. A third tap in time gives [`Tap::Triple`]; anything else in between (another key, a
//! click) resolves the double tap at once.

/// The only distinction the detector needs: Ctrl (left or right) or anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Ctrl,
    Other,
}

/// What an event completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tap {
    /// Nothing yet.
    None,
    /// A double tap that waits for a possible third tap; arm the timer.
    Pending,
    /// A double tap.
    Double,
    /// A triple tap.
    Triple,
}

/// Recognises "press and release Ctrl two (or three) times in quick succession, with nothing else
/// in between".
#[derive(Debug, Clone)]
pub struct TapDetector {
    /// Longest time between the release of the first tap and the release of the second.
    gap_ms: u64,
    /// Longest time Ctrl may be held for the press to still count as a tap.
    hold_ms: u64,
    /// Longest time between the release of the second tap and the release of the third; 0 turns
    /// triple taps off.
    triple_ms: u64,
    down_at: Option<u64>,
    combo: bool,
    last_tap: Option<u64>,
    /// The release of a second tap that is waiting for a third.
    pending: Option<u64>,
}

impl TapDetector {
    /// The defaults used by the original AutoHotkey and Hammerspoon scripts, widened a little.
    pub const DEFAULT_GAP_MS: u64 = 500;
    pub const DEFAULT_HOLD_MS: u64 = 300;
    pub const DEFAULT_TRIPLE_MS: u64 = 300;

    /// A detector; `triple_ms` of 0 turns triple taps off, so double taps fire at once.
    pub fn new(gap_ms: u64, hold_ms: u64, triple_ms: u64) -> Self {
        Self {
            gap_ms,
            hold_ms,
            triple_ms,
            down_at: None,
            combo: false,
            last_tap: None,
            pending: None,
        }
    }

    /// How long to wait after [`Tap::Pending`] before calling [`expire`](Self::expire).
    pub fn triple_ms(&self) -> u64 {
        self.triple_ms
    }

    /// Feeds a key press. Auto-repeat of a held Ctrl is ignored. Another key resolves a pending
    /// double tap, so it can return [`Tap::Double`].
    pub fn key_down(&mut self, key: Key, now: u64) -> Tap {
        match key {
            Key::Ctrl => {
                if self.down_at.is_none() {
                    self.down_at = Some(now);
                    self.combo = false;
                }
                Tap::None
            }
            Key::Other => self.interrupt(),
        }
    }

    /// Feeds a mouse click or scroll: Ctrl+click is a shortcut, not a tap. It resolves a pending
    /// double tap, so it can return [`Tap::Double`].
    pub fn pointer(&mut self) -> Tap {
        self.interrupt()
    }

    /// Feeds a key release.
    pub fn key_up(&mut self, key: Key, now: u64) -> Tap {
        if key != Key::Ctrl {
            return Tap::None;
        }
        let Some(down) = self.down_at.take() else {
            self.last_tap = None;
            return self.resolve();
        };
        if self.combo || now.saturating_sub(down) > self.hold_ms {
            self.last_tap = None;
            return self.resolve();
        }
        if let Some(second) = self.pending.take() {
            if now.saturating_sub(second) < self.triple_ms {
                self.last_tap = None;
                return Tap::Triple;
            }
            // Too late for a triple: the double tap stands, and this tap starts a new sequence.
            self.last_tap = Some(now);
            return Tap::Double;
        }
        match self.last_tap {
            Some(first) if now.saturating_sub(first) < self.gap_ms => {
                self.last_tap = None;
                if self.triple_ms == 0 {
                    Tap::Double
                } else {
                    self.pending = Some(now);
                    Tap::Pending
                }
            }
            _ => {
                self.last_tap = Some(now);
                Tap::None
            }
        }
    }

    /// The timer armed on [`Tap::Pending`] fired: the double tap stands, unless it was resolved
    /// already.
    pub fn expire(&mut self) -> Tap {
        self.resolve()
    }

    fn resolve(&mut self) -> Tap {
        if self.pending.take().is_some() {
            Tap::Double
        } else {
            Tap::None
        }
    }

    fn interrupt(&mut self) -> Tap {
        if self.down_at.is_some() {
            self.combo = true;
        }
        self.last_tap = None;
        self.resolve()
    }
}

impl Default for TapDetector {
    fn default() -> Self {
        Self::new(Self::DEFAULT_GAP_MS, Self::DEFAULT_HOLD_MS, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap(d: &mut TapDetector, at: u64, hold: u64) -> Tap {
        d.key_down(Key::Ctrl, at);
        d.key_up(Key::Ctrl, at + hold)
    }

    fn triple() -> TapDetector {
        TapDetector::new(500, 300, 300)
    }

    #[test]
    fn two_quick_taps_make_a_double_tap() {
        let mut d = TapDetector::default();
        assert_eq!(tap(&mut d, 0, 50), Tap::None);
        assert_eq!(tap(&mut d, 150, 50), Tap::Double);
    }

    #[test]
    fn a_third_tap_starts_over_without_triples() {
        let mut d = TapDetector::default();
        tap(&mut d, 0, 50);
        assert_eq!(tap(&mut d, 150, 50), Tap::Double);
        assert_eq!(tap(&mut d, 300, 50), Tap::None);
        assert_eq!(tap(&mut d, 450, 50), Tap::Double);
    }

    #[test]
    fn taps_too_far_apart_do_not_count() {
        let mut d = TapDetector::default();
        tap(&mut d, 0, 50);
        assert_eq!(tap(&mut d, 1000, 50), Tap::None);
    }

    #[test]
    fn the_gap_is_measured_between_releases() {
        let mut d = TapDetector::new(500, 300, 0);
        tap(&mut d, 0, 50); // released at 50
        assert_eq!(tap(&mut d, 500, 50), Tap::None); // released 500 ms after the first release
        let mut d = TapDetector::new(500, 300, 0);
        tap(&mut d, 0, 50);
        assert_eq!(tap(&mut d, 440, 50), Tap::Double); // released 440 ms after
    }

    #[test]
    fn a_long_press_is_not_a_tap() {
        let mut d = TapDetector::default();
        tap(&mut d, 0, 50);
        assert_eq!(tap(&mut d, 100, 400), Tap::None);
        assert_eq!(
            tap(&mut d, 600, 50),
            Tap::None,
            "a rejected press must also reset the first tap"
        );
    }

    #[test]
    fn ctrl_with_another_key_is_a_shortcut() {
        let mut d = TapDetector::default();
        tap(&mut d, 0, 50);
        d.key_down(Key::Ctrl, 100);
        d.key_down(Key::Other, 120);
        d.key_up(Key::Other, 140);
        assert_eq!(d.key_up(Key::Ctrl, 160), Tap::None);
    }

    #[test]
    fn a_key_between_the_taps_cancels() {
        let mut d = TapDetector::default();
        tap(&mut d, 0, 50);
        d.key_down(Key::Other, 100);
        d.key_up(Key::Other, 110);
        assert_eq!(tap(&mut d, 200, 50), Tap::None);
    }

    #[test]
    fn a_click_while_ctrl_is_down_cancels() {
        let mut d = TapDetector::default();
        tap(&mut d, 0, 50);
        d.key_down(Key::Ctrl, 100);
        d.pointer();
        assert_eq!(d.key_up(Key::Ctrl, 130), Tap::None);
    }

    #[test]
    fn auto_repeat_does_not_restart_the_hold() {
        let mut d = TapDetector::default();
        d.key_down(Key::Ctrl, 0);
        d.key_down(Key::Ctrl, 250); // auto-repeat
        d.key_down(Key::Ctrl, 500);
        assert_eq!(d.key_up(Key::Ctrl, 520), Tap::None, "held 520 ms in total");
        assert_eq!(tap(&mut d, 600, 50), Tap::None);
    }

    #[test]
    fn a_release_without_a_press_is_ignored() {
        let mut d = TapDetector::default();
        assert_eq!(d.key_up(Key::Ctrl, 10), Tap::None);
        assert_eq!(tap(&mut d, 100, 50), Tap::None);
    }

    #[test]
    fn with_triples_a_double_tap_waits_for_the_timer() {
        let mut d = triple();
        tap(&mut d, 0, 50);
        assert_eq!(tap(&mut d, 150, 50), Tap::Pending);
        assert_eq!(d.expire(), Tap::Double);
        assert_eq!(d.expire(), Tap::None, "it fires only once");
    }

    #[test]
    fn three_quick_taps_make_a_triple_tap() {
        let mut d = triple();
        tap(&mut d, 0, 50);
        tap(&mut d, 150, 50);
        assert_eq!(tap(&mut d, 300, 50), Tap::Triple);
        assert_eq!(d.expire(), Tap::None, "the late timer does nothing");
    }

    #[test]
    fn a_late_third_tap_keeps_the_double_and_starts_over() {
        let mut d = triple();
        tap(&mut d, 0, 50);
        tap(&mut d, 150, 50); // released at 200
        assert_eq!(tap(&mut d, 500, 50), Tap::Double); // released 350 ms later
        assert_eq!(tap(&mut d, 650, 50), Tap::Pending);
    }

    #[test]
    fn another_key_resolves_a_pending_double_at_once() {
        let mut d = triple();
        tap(&mut d, 0, 50);
        tap(&mut d, 150, 50);
        assert_eq!(d.key_down(Key::Other, 250), Tap::Double);
        assert_eq!(d.expire(), Tap::None);
    }

    #[test]
    fn a_click_resolves_a_pending_double_at_once() {
        let mut d = triple();
        tap(&mut d, 0, 50);
        tap(&mut d, 150, 50);
        assert_eq!(d.pointer(), Tap::Double);
    }

    #[test]
    fn a_long_third_press_keeps_the_double() {
        let mut d = triple();
        tap(&mut d, 0, 50);
        tap(&mut d, 150, 50);
        assert_eq!(tap(&mut d, 250, 400), Tap::Double);
    }
}
