//! `imetap`: switch input methods with hotkeys, toggle Chinese/English with a Ctrl double-tap, and
//! show a HUD whenever the input method or its mode changes.
//!
//! The pure logic (`tap`, `state`, `doubao`) is platform independent and unit-tested. Everything
//! that talks to the operating system lives in `win` (Windows) or `mac` (macOS).

pub mod config;
pub mod doubao;
pub mod state;
pub mod tap;

#[cfg(windows)]
pub mod win;

#[cfg(target_os = "macos")]
pub mod mac;
