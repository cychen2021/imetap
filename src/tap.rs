//! Double-tap detection for the Ctrl key.
//!
//! The detector is a pure state machine fed with key events and timestamps in milliseconds, so it
//! is shared by the Windows keyboard hook and the macOS event tap and is tested without either.

/// The only distinction the detector needs: Ctrl (left or right) or anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Ctrl,
    Other,
}

/// Recognises "press and release Ctrl twice in quick succession, with nothing else in between".
#[derive(Debug, Clone)]
pub struct TapDetector {
    /// Longest time between the release of the first tap and the release of the second.
    gap_ms: u64,
    /// Longest time Ctrl may be held for the press to still count as a tap.
    hold_ms: u64,
    down_at: Option<u64>,
    combo: bool,
    last_tap: Option<u64>,
}

impl TapDetector {
    /// The defaults used by the original AutoHotkey and Hammerspoon scripts, widened a little.
    pub const DEFAULT_GAP_MS: u64 = 500;
    pub const DEFAULT_HOLD_MS: u64 = 300;

    pub fn new(gap_ms: u64, hold_ms: u64) -> Self {
        Self {
            gap_ms,
            hold_ms,
            down_at: None,
            combo: false,
            last_tap: None,
        }
    }

    /// Feeds a key press. Auto-repeat of a held Ctrl is ignored.
    pub fn key_down(&mut self, key: Key, now: u64) {
        match key {
            Key::Ctrl => {
                if self.down_at.is_none() {
                    self.down_at = Some(now);
                    self.combo = false;
                }
            }
            Key::Other => self.interrupt(),
        }
    }

    /// Feeds a mouse click or scroll: Ctrl+click is a shortcut, not a tap.
    pub fn pointer(&mut self) {
        self.interrupt();
    }

    /// Feeds a key release. Returns `true` when this release completes a double tap.
    pub fn key_up(&mut self, key: Key, now: u64) -> bool {
        if key != Key::Ctrl {
            return false;
        }
        let Some(down) = self.down_at.take() else {
            self.last_tap = None;
            return false;
        };
        if self.combo || now.saturating_sub(down) > self.hold_ms {
            self.last_tap = None;
            return false;
        }
        match self.last_tap {
            Some(first) if now.saturating_sub(first) < self.gap_ms => {
                self.last_tap = None;
                true
            }
            _ => {
                self.last_tap = Some(now);
                false
            }
        }
    }

    fn interrupt(&mut self) {
        if self.down_at.is_some() {
            self.combo = true;
        }
        self.last_tap = None;
    }
}

impl Default for TapDetector {
    fn default() -> Self {
        Self::new(Self::DEFAULT_GAP_MS, Self::DEFAULT_HOLD_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap(d: &mut TapDetector, at: u64, hold: u64) -> bool {
        d.key_down(Key::Ctrl, at);
        d.key_up(Key::Ctrl, at + hold)
    }

    #[test]
    fn two_quick_taps_make_a_double_tap() {
        let mut d = TapDetector::default();
        assert!(!tap(&mut d, 0, 50));
        assert!(tap(&mut d, 150, 50));
    }

    #[test]
    fn a_third_tap_starts_over() {
        let mut d = TapDetector::default();
        tap(&mut d, 0, 50);
        assert!(tap(&mut d, 150, 50));
        assert!(!tap(&mut d, 300, 50));
        assert!(tap(&mut d, 450, 50));
    }

    #[test]
    fn taps_too_far_apart_do_not_count() {
        let mut d = TapDetector::default();
        tap(&mut d, 0, 50);
        assert!(!tap(&mut d, 1000, 50));
    }

    #[test]
    fn the_gap_is_measured_between_releases() {
        let mut d = TapDetector::new(500, 300);
        tap(&mut d, 0, 50); // released at 50
        assert!(!tap(&mut d, 500, 50)); // released at 550: 500 ms after the first release
        let mut d = TapDetector::new(500, 300);
        tap(&mut d, 0, 50);
        assert!(tap(&mut d, 440, 50)); // released at 490: 440 ms after
    }

    #[test]
    fn a_long_press_is_not_a_tap() {
        let mut d = TapDetector::default();
        tap(&mut d, 0, 50);
        assert!(!tap(&mut d, 100, 400));
        assert!(
            !tap(&mut d, 600, 50),
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
        assert!(!d.key_up(Key::Ctrl, 160));
    }

    #[test]
    fn a_key_between_the_taps_cancels() {
        let mut d = TapDetector::default();
        tap(&mut d, 0, 50);
        d.key_down(Key::Other, 100);
        d.key_up(Key::Other, 110);
        assert!(!tap(&mut d, 200, 50));
    }

    #[test]
    fn a_click_while_ctrl_is_down_cancels() {
        let mut d = TapDetector::default();
        tap(&mut d, 0, 50);
        d.key_down(Key::Ctrl, 100);
        d.pointer();
        assert!(!d.key_up(Key::Ctrl, 130));
    }

    #[test]
    fn auto_repeat_does_not_restart_the_hold() {
        let mut d = TapDetector::default();
        d.key_down(Key::Ctrl, 0);
        d.key_down(Key::Ctrl, 250); // auto-repeat
        d.key_down(Key::Ctrl, 500);
        assert!(!d.key_up(Key::Ctrl, 520), "held 520 ms in total");
        assert!(!tap(&mut d, 600, 50));
    }

    #[test]
    fn a_release_without_a_press_is_ignored() {
        let mut d = TapDetector::default();
        assert!(!d.key_up(Key::Ctrl, 10));
        assert!(!tap(&mut d, 100, 50));
    }
}
