//! The global hotkeys of `config.toml` that switch to an input method.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};

use crate::config::{Config, Key as ShortcutKey, Shortcut};
use crate::state::Ime;

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

pub(super) const HOTKEY_IMES: [Ime; 4] = [Ime::Us, Ime::Pinyin, Ime::Doubao, Ime::Mingkuai];

/// Registers the configured hotkeys. Returns the ids that succeeded. The id is the index in
/// `HOTKEY_IMES`.
pub(super) fn register_hotkeys(hwnd: HWND, config: &Config) -> Vec<i32> {
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

/// Unregisters the hotkeys that `register_hotkeys` registered.
pub(super) fn unregister_hotkeys(hwnd: HWND, registered: Vec<i32>) {
    for id in registered {
        // SAFETY: unregisters hotkeys registered by `register_hotkeys`.
        unsafe { UnregisterHotKey(Some(hwnd), id).ok() };
    }
}
