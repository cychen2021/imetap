//! Windows implementation.
//!
//! Everything that touches the Win32 API is in this module tree. The unsafe calls are confined to
//! thin wrappers with a safety comment.

pub mod app;
pub mod autostart;
pub mod doubao;
pub mod hotkey;
pub mod hud;
pub mod ime;
pub mod uia;

use std::thread;
use std::time::Duration;

use anyhow::Result;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

use crate::state::{Ime, Snapshot};

/// Whether the environment variable `IMETAP_DEBUG` was set when this process asked first.
pub fn debug_log_enabled() -> bool {
    use std::sync::OnceLock;

    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("IMETAP_DEBUG").is_some())
}

/// Where the debug log is written.
pub fn debug_log_path() -> std::path::PathBuf {
    std::env::temp_dir().join("imetap-debug.log")
}

/// Appends a line to the debug log when `IMETAP_DEBUG` is set. The message is only built then.
pub fn debug_log(message: impl FnOnce() -> String) {
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    if !debug_log_enabled() {
        return;
    }
    let path = debug_log_path();
    // Start over once it gets big, so that leaving the switch on cannot fill the disk.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 2_000_000) {
        let _ = std::fs::remove_file(&path);
    }
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{secs:.3} {}", message());
    }
}

/// Initialises COM for the calling thread. Every thread that uses the types below needs this.
pub fn init_com() {
    // SAFETY: called once per thread. A repeated call returns S_FALSE, which is fine.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().ok() };
}

/// Reads and changes the input method state. Owns the UI Automation client, so keep one per thread.
pub struct Engine {
    tray: Option<uia::TrayReader>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            tray: uia::TrayReader::new().ok(),
        }
    }

    /// The state of the foreground window's input method.
    pub fn snapshot(&self) -> Snapshot {
        let ime = ime::current();
        let chinese = match ime {
            Ime::Doubao => doubao::is_chinese().ok(),
            Ime::Pinyin => self.tray.as_ref().and_then(|t| t.chinese_mode()),
            _ => None,
        };
        let snapshot = Snapshot::new(ime, chinese);
        match ime {
            Ime::Other => {
                ime::other_name().map_or(snapshot, |name| snapshot.with_other_name(&name))
            }
            _ => snapshot,
        }
    }

    /// Whether keyboard focus is in a text field (`true` when that cannot be told), and what UI
    /// Automation reported for it.
    ///
    /// A bare window has three meanings. A Windows shell window (taskbar, Start) is not a text field.
    /// A Chromium window (Chrome, Edge, Electron, and WebView2 apps like the new Outlook) has its
    /// accessibility tree not ready yet, since Chromium builds it on the first request, so ask again
    /// a few times, and if it is still not there, it does not count as a text field. The next focus
    /// change, key press or click checks again. Any other bare window is a custom-drawn app, which
    /// tells nothing and counts as a text field.
    pub fn focus_in_text_field(&self) -> (bool, Option<uia::FocusInfo>) {
        let read = || self.tray.as_ref().and_then(uia::TrayReader::focus_info);
        let bare = |info: &Option<uia::FocusInfo>| info.is_some_and(|i| i.is_bare_window());
        let class = uia::foreground_class();
        let chromium = |info: &Option<uia::FocusInfo>| {
            class.starts_with("Chrome_WidgetWin") || info.is_some_and(|i| i.chromium)
        };
        let mut info = read();
        if bare(&info) && uia::is_shell_class(&class) {
            return (false, info);
        }
        for _ in 0..5 {
            if !(bare(&info) && chromium(&info)) {
                break;
            }
            thread::sleep(Duration::from_millis(100));
            info = read();
        }
        let text = if bare(&info) && chromium(&info) {
            false
        } else {
            info.is_none_or(uia::is_text_field)
        };
        (text, info)
    }

    /// Raw facts behind `snapshot`, for diagnosing detection problems.
    pub fn describe(&self) -> String {
        let focus = self.tray.as_ref().and_then(uia::TrayReader::focus_info);
        format!(
            "hkl={:#x} tip={:?} name={:?}
         focus={:?} text-field={}",
            ime::layout(),
            ime::active_tip(),
            ime::other_name(),
            focus,
            self.focus_in_text_field().0
        )
    }

    /// The handle of the foreground window.
    pub fn foreground(&self) -> isize {
        ime::foreground()
    }

    /// Which input method the foreground window is using, without the mode.
    pub fn ime(&self) -> Ime {
        ime::current()
    }

    /// Switches to another input method.
    pub fn set(&self, target: Ime) -> Result<()> {
        ime::activate(target)
    }

    /// Toggles Chinese/English in the current input method. Does nothing for the US layout.
    pub fn toggle(&self) -> Result<()> {
        match ime::current() {
            Ime::Doubao => doubao::set_chinese(!doubao::is_chinese()?),
            Ime::Pinyin => hotkey::send_toggle(),
            _ => Ok(()),
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
