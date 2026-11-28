//! macOS implementation, a port of the Hammerspoon script `ime.lua`.
//!
//! **Untested.** It is written against the documented Carbon, Quartz and AppKit APIs and is only
//! checked to compile (`cargo check --target aarch64-apple-darwin`). It has never been run on a Mac.
//!
//! There is no Chinese/English mode to read on macOS: the Doubao and Shuangpin input methods
//! are separate input sources, and the double-tap swaps between the US source and the last-used
//! Chinese one, exactly as `ime.lua` does. Mingkuai is the exception: it has a mode in its server,
//! so the double-tap switches that mode.

pub mod app;
pub mod autostart;
mod ffi;
mod hooks;
mod hotkeys;
mod hud;
mod ime;
pub(crate) mod mingkuai;
mod tray;

pub use autostart::{is_enabled as autostart_enabled, set as set_autostart};

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Seek, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};

use crate::daemon_path::daemon_exe;
use crate::debug;
use crate::state::{Ime, Snapshot};
use ime::{ime_of, last_chinese, remember_chinese, source_id};

/// The supported input methods that the system offers. Call it from the main thread only.
pub fn available_imes() -> Vec<Ime> {
    ime::available()
}

/// Reads and changes the input source. Call it from the main thread only (TIS requires that).
pub struct Engine;

impl Engine {
    pub fn new() -> Self {
        Engine
    }

    pub fn snapshot(&self) -> Snapshot {
        let ime = ffi::current_source()
            .map(|id| ime_of(&id))
            .unwrap_or(Ime::Other);
        let chinese = match ime {
            Ime::Us | Ime::Other => None,
            // The mode of Mingkuai is in its server. Unknown while the server is down.
            Ime::Mingkuai => mingkuai::is_chinese().ok(),
            _ => Some(true),
        };
        let snapshot = Snapshot::new(ime, chinese);
        match ime {
            Ime::Other => {
                ffi::current_source_name().map_or(snapshot, |name| snapshot.with_other_name(&name))
            }
            _ => snapshot,
        }
    }

    pub fn describe(&self) -> String {
        format!("source={:?}", ffi::current_source())
    }

    pub fn set(&self, ime: Ime) -> Result<()> {
        let Some(id) = source_id(ime) else {
            bail!("{} is not available on macOS", ime.name())
        };
        ffi::select_source(id)?;
        if ime != Ime::Us {
            remember_chinese(ime);
        }
        Ok(())
    }

    /// Swaps between the US source and the last-used Chinese one. In Mingkuai it switches the mode
    /// of Mingkuai instead and stays in the same input source.
    pub fn toggle(&self) -> Result<()> {
        match self.snapshot().ime {
            Ime::Mingkuai => mingkuai::set_chinese(!mingkuai::is_chinese()?),
            Ime::Us | Ime::Other => self.set(last_chinese()),
            _ => self.set(Ime::Us),
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

// ---- process management ------------------------------------------------------------------------

/// The single-instance lock, which also records the running instance's PID.
///
/// Its place is fixed, not next to the config, so that an instance started with another
/// `XDG_CONFIG_HOME` still sees the running one: two instances would both act on every double tap,
/// and the second switch undoes the first.
fn lock_file() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Library/Caches/imetap/instance.lock"))
}

fn open_lock() -> Result<File> {
    let path = lock_file()?;
    fs::create_dir_all(path.parent().context("no parent directory")?)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("cannot open {}", path.display()))
}

/// Held for the life of the resident process. The OS releases the lock when it exits.
static INSTANCE: OnceLock<File> = OnceLock::new();

/// Takes the single-instance lock and records this process's PID, and on a second line whether it
/// writes the debug log. Fails if another instance holds it.
pub(crate) fn claim_pid() -> Result<()> {
    let mut file = open_lock()?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => bail!("imetap is already running"),
        Err(TryLockError::Error(e)) => return Err(e).context("cannot lock the instance file"),
    }
    file.set_len(0)?;
    file.rewind()?;
    write!(
        file,
        "{}\n{}",
        std::process::id(),
        u8::from(debug::enabled())
    )?;
    file.flush()?;
    let _ = INSTANCE.set(file);
    Ok(())
}

/// What the running instance recorded: the lock is held, and the file names its PID and whether it
/// writes the debug log (an older instance recorded the PID only).
fn running_instance() -> Option<(u32, bool)> {
    let mut file = open_lock().ok()?;
    match file.try_lock() {
        // Nobody holds it. Dropping the file releases our probe.
        Ok(()) => None,
        Err(TryLockError::WouldBlock) => {
            let mut text = String::new();
            file.read_to_string(&mut text).ok()?;
            let mut lines = text.lines();
            let pid = lines.next()?.trim().parse().ok()?;
            Some((pid, lines.next().map(str::trim) == Some("1")))
        }
        Err(TryLockError::Error(_)) => None,
    }
}

/// The PID of the running instance.
fn running_pid() -> Option<u32> {
    running_instance().map(|(pid, _)| pid)
}

/// Whether the running instance writes the debug log. `None` when none runs.
pub fn running_debug_log() -> Option<bool> {
    running_instance().map(|(_, debug)| debug)
}

pub fn is_running() -> bool {
    running_pid().is_some()
}

pub fn stop() -> Result<bool> {
    let Some(pid) = running_pid() else {
        return Ok(false);
    };
    Command::new("kill").arg(pid.to_string()).status()?;
    Ok(true)
}

pub fn start() -> Result<()> {
    let exe = daemon_exe()?;
    Command::new(exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("cannot start imetapd")?;
    Ok(())
}

/// Opens a file with the default application.
pub fn open(path: &std::path::Path) -> Result<()> {
    Command::new("open")
        .arg(path)
        .status()
        .context("cannot run `open`")?;
    Ok(())
}
