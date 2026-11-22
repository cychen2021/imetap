//! The HUD's brightness over time: fade in, hold, fade out.
//!
//! A pure function of the time since the HUD appeared, shared by the Windows and macOS code, which
//! only have to ask for the level on every timer tick.

/// Fade-in, hold and fade-out times in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fade {
    pub in_ms: u64,
    pub hold_ms: u64,
    pub out_ms: u64,
}

impl Fade {
    /// A fade that takes `in_ms` to come in and `out_ms` to go out around `hold_ms` of full
    /// brightness.
    pub fn new(in_ms: u64, hold_ms: u64, out_ms: u64) -> Fade {
        Fade {
            in_ms,
            hold_ms,
            out_ms,
        }
    }

    /// How long the whole thing lasts.
    pub fn total_ms(&self) -> u64 {
        self.in_ms + self.hold_ms + self.out_ms
    }

    /// Brightness from 0.0 to 1.0 after `elapsed_ms`; 0.0 once it is over.
    pub fn level(&self, elapsed_ms: u64) -> f32 {
        let after_in = elapsed_ms.saturating_sub(self.in_ms);
        if elapsed_ms < self.in_ms {
            elapsed_ms as f32 / self.in_ms as f32
        } else if after_in < self.hold_ms {
            1.0
        } else if after_in - self.hold_ms < self.out_ms {
            1.0 - (after_in - self.hold_ms) as f32 / self.out_ms as f32
        } else {
            0.0
        }
    }

    /// Whether it is over after `elapsed_ms`.
    pub fn finished(&self, elapsed_ms: u64) -> bool {
        elapsed_ms >= self.total_ms()
    }

    /// Where to start when the HUD is shown again while it is still visible at `level`, so that the
    /// brightness carries on from there instead of jumping: part-way through the fade-in.
    pub fn start_for(&self, level: f32) -> u64 {
        (level.clamp(0.0, 1.0) * self.in_ms as f32) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_rises_holds_and_falls() {
        let fade = Fade::new(200, 1000, 200);
        assert_eq!(fade.total_ms(), 1400);
        assert_eq!(fade.level(0), 0.0);
        assert_eq!(fade.level(100), 0.5);
        assert_eq!(fade.level(200), 1.0);
        assert_eq!(fade.level(1200), 1.0);
        assert_eq!(fade.level(1300), 0.5);
        assert_eq!(fade.level(1400), 0.0);
        assert!(!fade.finished(1399));
        assert!(fade.finished(1400));
    }

    #[test]
    fn coming_in_and_going_out_take_their_own_time() {
        let fade = Fade::new(400, 1000, 100);
        assert_eq!(fade.total_ms(), 1500);
        assert_eq!(fade.level(200), 0.5);
        assert_eq!(fade.level(1450), 0.5);
    }

    #[test]
    fn no_fade_time_means_no_fade() {
        let fade = Fade::new(0, 900, 0);
        assert_eq!(fade.level(0), 1.0);
        assert_eq!(fade.level(899), 1.0);
        assert_eq!(fade.level(900), 0.0);
        assert!(fade.finished(900));
    }

    #[test]
    fn showing_again_carries_on_from_the_current_level() {
        let fade = Fade::new(200, 1000, 200);
        assert_eq!(fade.level(fade.start_for(0.5)), 0.5);
        assert_eq!(fade.start_for(0.0), 0);
        assert_eq!(fade.start_for(1.0), 200);
        assert_eq!(fade.start_for(7.0), 200);
    }
}
