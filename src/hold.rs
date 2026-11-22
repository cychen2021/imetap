//! Holding back keystrokes while a Ctrl double tap is being applied.
//!
//! A double tap needs time to take effect: the tap detector waits for a possible third tap, and the
//! input method then needs a moment to switch. Keys typed in that time would reach the application
//! in the old mode. [`KeyHold`] captures them, so the caller can swallow them and replay them, in
//! order, once the switch is done. It is a plain queue and shared by the platform code.

/// One keyboard event, as the platform code needs it to replay it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldKey {
    /// The virtual key code.
    pub vk: u32,
    /// The hardware scan code.
    pub scan: u32,
    /// Whether it is an extended key (right Ctrl, arrows, ...).
    pub extended: bool,
    /// A release rather than a press.
    pub up: bool,
}

/// Queue of keyboard events captured while active.
#[derive(Debug, Default)]
pub struct KeyHold {
    active: bool,
    queue: Vec<HeldKey>,
}

impl KeyHold {
    /// Starts capturing. Does nothing when already active.
    pub fn begin(&mut self) {
        self.active = true;
    }

    /// Whether events are being captured.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Offers an event: when active it is queued and the caller must swallow it (`true`). Otherwise
    /// the caller lets it through.
    pub fn capture(&mut self, key: HeldKey) -> bool {
        if self.active {
            self.queue.push(key);
        }
        self.active
    }

    /// Takes the events captured so far, in order. Capturing goes on, so the caller replays them and
    /// then calls `take` again until it is empty, and only then [`stop`](Self::stop)s.
    pub fn take(&mut self) -> Vec<HeldKey> {
        std::mem::take(&mut self.queue)
    }

    /// Stops capturing. Events still queued are dropped, so take them first.
    pub fn stop(&mut self) {
        self.active = false;
        self.queue.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(vk: u32, up: bool) -> HeldKey {
        HeldKey {
            vk,
            scan: 0,
            extended: false,
            up,
        }
    }

    #[test]
    fn events_pass_while_inactive() {
        let mut hold = KeyHold::default();
        assert!(!hold.capture(key(0x41, false)));
        assert!(hold.take().is_empty());
    }

    #[test]
    fn events_are_queued_in_order_while_active() {
        let mut hold = KeyHold::default();
        hold.begin();
        assert!(hold.capture(key(0x41, false)));
        assert!(hold.capture(key(0x42, false)));
        assert!(hold.capture(key(0x41, true)));
        assert_eq!(
            hold.take(),
            [key(0x41, false), key(0x42, false), key(0x41, true)]
        );
        assert!(hold.take().is_empty());
    }

    #[test]
    fn capturing_goes_on_until_stopped() {
        let mut hold = KeyHold::default();
        hold.begin();
        hold.capture(key(0x41, false));
        assert_eq!(hold.take().len(), 1);
        assert!(hold.capture(key(0x42, false)));
        assert_eq!(hold.take().len(), 1);
        hold.stop();
        assert!(!hold.is_active());
        assert!(!hold.capture(key(0x43, false)));
    }

    #[test]
    fn beginning_twice_keeps_the_queue() {
        let mut hold = KeyHold::default();
        hold.begin();
        hold.capture(key(0x41, false));
        hold.begin();
        assert_eq!(hold.take().len(), 1);
    }
}
