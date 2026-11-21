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

use anyhow::Result;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

use crate::state::{Ime, Snapshot};

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
        Snapshot::new(ime, chinese)
    }

    /// Raw facts behind `snapshot`, for diagnosing detection problems.
    pub fn describe(&self) -> String {
        format!("hkl={:#x} tip={:?}", ime::layout(), ime::active_tip())
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
