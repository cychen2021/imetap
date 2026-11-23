//! `imetap`: switch input methods with hotkeys, toggle Chinese/English with a Ctrl double-tap, and
//! show a HUD whenever the input method or its mode changes.
//!
//! The pure logic (`tap`, `reminder`, `state`, `doubao`) is platform independent and unit-tested. Everything
//! that talks to the operating system lives in `win` (Windows) or `mac` (macOS).

pub mod cli;
pub mod config;
pub mod debug;
pub mod doubao;
pub mod fade;
pub mod hold;
pub mod mingkuai;
pub mod reminder;
pub mod state;
pub mod tap;

use std::path::{Path, PathBuf};

#[cfg(windows)]
pub mod win;

#[cfg(target_os = "macos")]
pub mod mac;

/// Where `scripts/install.py` puts the macOS app bundle's executable, relative to the home directory.
pub const MAC_BUNDLE_EXE: &str = "Applications/imetap.app/Contents/MacOS/imetap";

/// The executable of the resident process: the running executable, or the `imetap` next to it when
/// running as `imetapctl`. On macOS, an installed app bundle takes precedence over an executable
/// outside it, because the Accessibility permission belongs to the bundle.
pub fn daemon_exe() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let exe = if exe.file_stem().is_some_and(|stem| stem == "imetapctl") {
        exe.with_file_name(format!("imetap{}", std::env::consts::EXE_SUFFIX))
    } else {
        exe
    };
    Ok(bundled_exe(
        &exe,
        std::env::var_os("HOME").as_deref().map(Path::new),
    ))
}

/// `exe`, or the installed app bundle's executable under `home` when `exe` is not inside a bundle
/// and the bundle exists. Only on macOS.
fn bundled_exe(exe: &Path, home: Option<&Path>) -> PathBuf {
    if cfg!(target_os = "macos")
        && !exe.components().any(|c| c.as_os_str() == "imetap.app")
        && let Some(bundled) = home.map(|h| h.join(MAC_BUNDLE_EXE))
        && bundled.exists()
    {
        return bundled;
    }
    exe.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_executable_without_a_bundle_stays() {
        let exe = Path::new("/usr/bin/imetap");
        assert_eq!(bundled_exe(exe, Some(Path::new("/nonexistent-home"))), exe);
        assert_eq!(bundled_exe(exe, None), exe);
    }

    #[test]
    fn an_executable_inside_the_bundle_stays() {
        let exe = Path::new("/Users/a/Applications/imetap.app/Contents/MacOS/imetap");
        assert_eq!(bundled_exe(exe, Some(Path::new("/Users/a"))), exe);
    }
}
