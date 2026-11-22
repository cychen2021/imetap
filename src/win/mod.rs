//! Windows implementation.
//!
//! Everything that touches the Win32 API is in this module tree; the unsafe calls are confined to
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

/// Appends a line to `%TEMP%\imetap-debug.log` when the environment variable `IMETAP_DEBUG` is set.
/// The message is only built then.
pub fn debug_log(message: impl FnOnce() -> String) {
    use std::io::Write;
    use std::sync::OnceLock;
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var_os("IMETAP_DEBUG").is_some()) {
        return;
    }
    let path = std::env::temp_dir().join("imetap-debug.log");
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
    // SAFETY: called once per thread; a repeated call returns S_FALSE, which is fine.
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
    /// A Chromium window (Chrome, Edge, any Electron app) that shows only a bare window has its
    /// accessibility tree not ready yet, since Chromium builds it on the first request. Ask again a
    /// few times; if it is still not there, it does not count as a text field, and the next focus
    /// change, key press or click checks again. Other bare windows are custom-drawn apps, which
    /// tell nothing and count as text fields.
    pub fn focus_in_text_field(&self) -> (bool, Option<uia::FocusInfo>) {
        let read = || self.tray.as_ref().and_then(uia::TrayReader::focus_info);
        let bare = |info: &Option<uia::FocusInfo>| info.is_some_and(|i| i.is_bare_window());
        let chromium = uia::foreground_is_chromium();
        let mut info = read();
        if chromium {
            for _ in 0..5 {
                if !bare(&info) {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
                info = read();
            }
        }
        let text = if chromium && bare(&info) {
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
