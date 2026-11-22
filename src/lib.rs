//! `imetap`: switch input methods with hotkeys, toggle Chinese/English with a Ctrl double-tap, and
//! show a HUD whenever the input method or its mode changes.
//!
//! The pure logic (`tap`, `reminder`, `state`, `doubao`) is platform independent and unit-tested. Everything
//! that talks to the operating system lives in `win` (Windows) or `mac` (macOS).

pub mod cli;
pub mod config;
pub mod doubao;
pub mod hold;
pub mod reminder;
pub mod state;
pub mod tap;

#[cfg(windows)]
pub mod win;

#[cfg(target_os = "macos")]
pub mod mac;

/// The executable of the resident process: the running executable, or the `imetap` next to it when
/// running as `imetapctl`.
pub fn daemon_exe() -> std::io::Result<std::path::PathBuf> {
    let exe = std::env::current_exe()?;
    if exe.file_stem().is_some_and(|stem| stem == "imetapctl") {
        Ok(exe.with_file_name(format!("imetap{}", std::env::consts::EXE_SUFFIX)))
    } else {
        Ok(exe)
    }
}
