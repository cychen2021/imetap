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

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::config::config_path;
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

fn pid_file() -> Result<PathBuf> {
    Ok(data_dir()?.join("imetap.pid"))
}

/// Records this process as the running instance; fails if another one is alive.
pub(crate) fn claim_pid() -> Result<()> {
    if is_running() {
        bail!("imetap is already running");
    }
    let path = pid_file()?;
    fs::create_dir_all(path.parent().context("no parent directory")?)?;
    fs::write(path, std::process::id().to_string())?;
    Ok(())
}

fn running_pid() -> Option<u32> {
    let pid: u32 = fs::read_to_string(pid_file().ok()?)
        .ok()?
        .trim()
        .parse()
        .ok()?;
    // `kill -0` only checks that the process exists.
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .ok()?
        .status
        .success()
        .then_some(pid)
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
    let exe = std::env::current_exe()?;
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
        let exe = std::env::current_exe()?;
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
