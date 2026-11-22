//! macOS implementation, a port of the Hammerspoon script `ime.lua`.
//!
//! **Untested.** It is written against the documented Carbon, Quartz and AppKit APIs and is only
//! checked to compile (`cargo check --target aarch64-apple-darwin`); it has never been run on a Mac.
//!
//! There is no Chinese/English mode to read on macOS: the Doubao and Shuangpin input methods
//! are separate input sources, and the double-tap swaps between the US source and the last-used
//! Chinese one, exactly as `ime.lua` does.

pub mod app;
mod ffi;

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Seek, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};

use crate::config::config_path;
use crate::daemon_exe;
use crate::state::{Ime, Snapshot};

const SOURCE_US: &str = "com.apple.keylayout.US";
const SOURCE_DOUBAO: &str = "com.bytedance.inputmethod.doubaoime.pinyin";
const SOURCE_SHUANGPIN: &str = "com.apple.inputmethod.SCIM.Shuangpin";
const LAUNCH_AGENT: &str = "io.github.cychen2021.imetap";

/// The input source identifier of an input method.
fn source_id(ime: Ime) -> Option<&'static str> {
    match ime {
        Ime::Us => Some(SOURCE_US),
        Ime::Doubao => Some(SOURCE_DOUBAO),
        Ime::Shuangpin => Some(SOURCE_SHUANGPIN),
        Ime::Pinyin | Ime::Other => None,
    }
}

fn ime_of(source: &str) -> Ime {
    match source {
        SOURCE_US => Ime::Us,
        SOURCE_DOUBAO => Ime::Doubao,
        SOURCE_SHUANGPIN => Ime::Shuangpin,
        _ => Ime::Other,
    }
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
            _ => Some(true),
        };
        Snapshot::new(ime, chinese)
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

    /// Swaps between the US source and the last-used Chinese one.
    pub fn toggle(&self) -> Result<()> {
        match self.snapshot().ime {
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

fn data_dir() -> Result<PathBuf> {
    let config = config_path().context("no configuration directory")?;
    Ok(config
        .parent()
        .context("no parent directory")?
        .to_path_buf())
}

fn last_chinese() -> Ime {
    data_dir()
        .ok()
        .and_then(|d| fs::read_to_string(d.join("last_chinese")).ok())
        .and_then(|s| Ime::parse(s.trim()))
        .filter(|i| matches!(i, Ime::Doubao | Ime::Shuangpin))
        .unwrap_or(Ime::Doubao)
}

fn remember_chinese(ime: Ime) {
    if let Ok(dir) = data_dir() {
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(dir.join("last_chinese"), ime.name());
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

/// Held for the life of the resident process; the OS releases the lock when it exits.
static INSTANCE: OnceLock<File> = OnceLock::new();

/// Takes the single-instance lock and records this process's PID; fails if another instance
/// holds it.
pub(crate) fn claim_pid() -> Result<()> {
    let mut file = open_lock()?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => bail!("imetap is already running"),
        Err(TryLockError::Error(e)) => return Err(e).context("cannot lock the instance file"),
    }
    file.set_len(0)?;
    file.rewind()?;
    write!(file, "{}", std::process::id())?;
    file.flush()?;
    let _ = INSTANCE.set(file);
    Ok(())
}

/// The PID of the running instance: the lock is held, and the file names it.
fn running_pid() -> Option<u32> {
    let mut file = open_lock().ok()?;
    match file.try_lock() {
        // Nobody holds it; dropping the file releases our probe.
        Ok(()) => None,
        Err(TryLockError::WouldBlock) => {
            let mut text = String::new();
            file.read_to_string(&mut text).ok()?;
            text.trim().parse().ok()
        }
        Err(TryLockError::Error(_)) => None,
    }
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
        .arg("run")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("cannot start imetap")?;
    Ok(())
}

// ---- login item --------------------------------------------------------------------------------

fn launch_agent_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{LAUNCH_AGENT}.plist")))
}

fn launch_agent_plist(exe: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>Label</key>\n\t<string>{LAUNCH_AGENT}</string>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{exe}</string>\n\t\t<string>run</string>\n\t</array>\n\
         \t<key>RunAtLoad</key>\n\t<true/>\n\
         </dict>\n</plist>\n"
    )
}

pub fn autostart_enabled() -> bool {
    launch_agent_path().map(|p| p.exists()).unwrap_or(false)
}

pub fn set_autostart(enabled: bool) -> Result<()> {
    let path = launch_agent_path()?;
    if enabled {
        let exe = daemon_exe()?;
        fs::create_dir_all(path.parent().context("no parent directory")?)?;
        fs::write(&path, launch_agent_plist(&exe.display().to_string()))?;
    } else if path.exists() {
        fs::remove_file(&path)?;
    }
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
