//! The global hotkeys of `config.toml` that switch to an input method. The event tap in `hooks`
//! matches them against each key press.

use super::ffi;
use crate::config::{Config, Key as ShortcutKey, Shortcut};
use crate::state::Ime;

const HOTKEY_IMES: [Ime; 4] = [Ime::Us, Ime::Shuangpin, Ime::Doubao, Ime::Mingkuai];
const MODIFIER_MASK: u64 = ffi::FLAG_CONTROL | ffi::FLAG_SHIFT | ffi::FLAG_ALT | ffi::FLAG_COMMAND;

/// A hotkey: the key code, the modifier flags and the input method it switches to.
pub(super) type Hotkey = (u16, u64, Ime);

/// The hotkeys that `config` sets.
pub(super) fn from_config(config: &Config) -> Vec<Hotkey> {
    HOTKEY_IMES
        .into_iter()
        .filter_map(|ime| {
            config
                .shortcut(ime)
                .ok()
                .flatten()
                .map(|s| (keycode(s.key), flags_of(&s), ime))
        })
        .collect()
}

/// The input method of the hotkey that a key press with `code` and `flags` hits.
pub(super) fn find(hotkeys: &[Hotkey], code: i64, flags: u64) -> Option<Ime> {
    hotkeys
        .iter()
        .find(|(k, f, _)| *k as i64 == code && flags & MODIFIER_MASK == *f)
        .map(|(_, _, ime)| *ime)
}

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
