//! `imetap` and `imetapd`: switch input methods with hotkeys, toggle Chinese/English with a Ctrl double-tap, and
//! show a HUD whenever the input method or its mode changes.
//!
//! The platform-independent modules (`tap`, `hold`, `fade`, `reminder`, `state`, `protocol`, `config`,
//! `debug`, `daemon_path`) are unit-tested without touching the OS. Everything that talks to the operating
//! system lives in `win` (Windows) or `mac` (macOS).

pub mod cli;
pub mod config;
pub mod daemon_path;
pub mod debug;
pub mod fade;
pub mod hold;
pub mod protocol;
pub mod reminder;
pub mod state;
pub mod tap;

#[cfg(windows)]
pub mod win;

#[cfg(target_os = "macos")]
pub mod mac;
