//! The command line `imetap`, and the entry point of the resident process `imetapd`.

use std::fs;
use std::process::Command as Process;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};

use crate::config::{self, Config};
use crate::daemon_path::daemon_exe;
use crate::debug;
use crate::protocol::mingkuai;
use crate::state::Ime;

/// Switch input methods, toggle Chinese/English and show a HUD for each change.
#[derive(Parser)]
#[command(
    version,
    about,
    arg_required_else_help = true,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Manage the resident process (`imetapd`).
    #[command(arg_required_else_help = true, disable_help_subcommand = true)]
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Switch the input method or the language.
    #[command(arg_required_else_help = true, disable_help_subcommand = true)]
    Switch {
        #[command(subcommand)]
        action: SwitchAction,
    },
    /// Show whether the resident process runs, and the foreground input method state.
    Status {
        /// Also print the raw keyboard layout and the active text input processor.
        #[arg(short, long)]
        verbose: bool,
    },
    /// Manage starting at login.
    #[command(arg_required_else_help = true, disable_help_subcommand = true)]
    Autostart {
        #[command(subcommand)]
        action: AutostartAction,
    },
    /// Print the paths imetap uses: the configuration file, the debug log and the resident process.
    Paths,
    /// Write a commented default configuration file, unless one exists.
    Init,
    /// Change the configuration file.
    #[command(arg_required_else_help = true, disable_help_subcommand = true)]
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Run in the foreground, in this terminal.
    Run {
        /// Write the debug log, the same as setting `IMETAP_DEBUG`.
        #[arg(long)]
        debug: bool,
    },
    /// Start the resident process in the background.
    Start,
    /// Stop the resident process.
    Stop,
    /// Make the running process read the configuration file again.
    Reload,
}

#[derive(Subcommand)]
enum SwitchAction {
    /// Switch the input method: us, pinyin, doubao (Windows), shuangpin (macOS).
    Im { ime: String },
    /// Toggle Chinese/English in the current input method.
    Lang,
}

#[derive(Subcommand)]
enum AutostartAction {
    /// Start imetap at login.
    Enable {
        /// Have the started process write the debug log (`imetapd --debug`).
        #[arg(long)]
        debug: bool,
    },
    /// Do not start imetap at login.
    Disable,
    /// Show whether it starts at login.
    Status,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Open the file in `$EDITOR`, or in the default editor of the system with `--gui` (created first if missing).
    Edit {
        /// Use the default editor of the system, not `$EDITOR`.
        #[arg(long)]
        gui: bool,
    },
}

/// Reads the settings that every command needs: the hidden Mingkuai support and the list of
/// enabled input methods. A broken config leaves them at their defaults here, because `daemon run`
/// and `daemon reload` report the error.
fn read_global_settings() {
    if let Some(Ok(config)) = config::config_path().map(|path| Config::load(&path)) {
        mingkuai::set_enabled(config.mingkuai.enable);
        if let Ok(list) = config.enabled_imes() {
            Ime::set_enabled(&list);
        }
    }
}

/// The entry point of the resident process `imetapd`: reads the config and runs until quit.
pub fn run_daemon(debug: bool) -> Result<()> {
    read_global_settings();
    if debug {
        debug::enable();
    }
    let config = match config::config_path() {
        Some(path) => Config::load(&path)?,
        None => Config::default(),
    };
    platform::run(config)
}

/// Parses the command line of `imetap` and runs the command.
pub fn main() -> Result<()> {
    let cli = Cli::parse();
    read_global_settings();
    match cli.command {
        Command::Daemon { action } => match action {
            DaemonAction::Run { debug } => run_daemon(debug),
            DaemonAction::Start => {
                if platform::is_running() {
                    println!("imetapd is already running");
                } else {
                    platform::start()?;
                    println!("imetapd started");
                }
                Ok(())
            }
            DaemonAction::Stop => {
                println!(
                    "{}",
                    if platform::stop()? {
                        "imetapd stopped"
                    } else {
                        "imetapd is not running"
                    }
                );
                Ok(())
            }
            DaemonAction::Reload => reload(),
        },
        Command::Switch { action } => match action {
            SwitchAction::Im { ime } => {
                let ime = Ime::parse(&ime)
                    .filter(|&i| i != Ime::Mingkuai || mingkuai::enabled())
                    .filter(|&i| i.is_enabled())
                    .ok_or_else(|| anyhow!("unknown input method `{ime}`"))?;
                platform::engine().set(ime)
            }
            SwitchAction::Lang => platform::engine().toggle(),
        },
        Command::Status { verbose } => {
            println!(
                "process: {}",
                if platform::is_running() {
                    "running"
                } else {
                    "not running"
                }
            );
            println!(
                "login:   {}",
                if platform::autostart_enabled() {
                    "starts at login"
                } else {
                    "does not start at login"
                }
            );
            println!("debug:   {}", debug_state());
            let engine = platform::engine();
            println!("input:   {}", engine.snapshot());
            if verbose {
                println!("         {}", engine.describe());
            }
            Ok(())
        }
        Command::Autostart { action } => match action {
            AutostartAction::Enable { debug } => {
                platform::set_autostart(true, debug)?;
                if debug {
                    println!("imetapd will start at login and write the debug log");
                } else {
                    println!("imetapd will start at login");
                }
                Ok(())
            }
            AutostartAction::Disable => {
                platform::set_autostart(false, false)?;
                println!("imetapd will not start at login");
                Ok(())
            }
            AutostartAction::Status => {
                println!(
                    "{}",
                    if platform::autostart_enabled() {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
                Ok(())
            }
        },
        Command::Paths => {
            let path =
                config::config_path().context("no configuration directory on this system")?;
            let state = if path.exists() {
                ""
            } else {
                " (does not exist, `imetap init` writes it)"
            };
            println!("config:  {}{state}", path.display());
            let log = debug::log_path().display().to_string();
            match platform::running_debug_log() {
                Some(true) => println!("log:     {log}"),
                _ => println!("log:     {log} (debug logging is not on: {HOW_TO_LOG})"),
            }
            println!("daemon:  {}", daemon_exe()?.display());
            Ok(())
        }
        Command::Init => {
            let path =
                config::config_path().context("no configuration directory on this system")?;
            if path.exists() {
                bail!("{} already exists", path.display());
            }
            write_default_config(&path)?;
            println!("wrote {}", path.display());
            Ok(())
        }
        Command::Config { action } => match action {
            ConfigAction::Edit { gui } => edit_config(gui),
        },
    }
}

/// How to turn the debug log on.
const HOW_TO_LOG: &str =
    "start with `imetap daemon run --debug`, `imetapd --debug` or IMETAP_DEBUG=1";

/// Whether the running instance writes the debug log, as a line for `status`.
fn debug_state() -> String {
    match platform::running_debug_log() {
        None => "off (not running)".to_string(),
        Some(true) => format!("on, logging to {}", debug::log_path().display()),
        Some(false) => format!("off ({HOW_TO_LOG})"),
    }
}

fn write_default_config(path: &std::path::Path) -> Result<()> {
    fs::create_dir_all(path.parent().context("no parent directory")?)?;
    fs::write(path, config::DEFAULT_TOML)?;
    Ok(())
}

/// Opens the config file (written first if missing) in `$EDITOR`, or with `--gui` in the default
/// editor of the system.
fn edit_config(gui: bool) -> Result<()> {
    let path = config::config_path().context("no configuration directory on this system")?;
    if !path.exists() {
        write_default_config(&path)?;
    }
    if gui {
        return platform::open(&path);
    }
    let editor = std::env::var("EDITOR")
        .ok()
        .filter(|e| !e.trim().is_empty())
        .context("`$EDITOR` is not set (use `--gui` for the default editor of the system)")?;
    // `$EDITOR` may carry arguments, for example `code --wait`.
    let mut words = editor.split_whitespace();
    let status = Process::new(words.next().context("`$EDITOR` is empty")?)
        .args(words)
        .arg(&path)
        .status()
        .with_context(|| format!("cannot run `{editor}`"))?;
    if !status.success() {
        bail!("`{editor}` exited with {status}");
    }
    Ok(())
}

/// Checks the config file and makes the running process start again with it. The new process reads
/// the file, so the settings that other code reads at start (hooks, hotkeys, tray) all follow.
fn reload() -> Result<()> {
    if let Some(path) = config::config_path() {
        let config = Config::load(&path)?;
        config.enabled_imes()?;
    }
    if !platform::stop()? {
        println!("imetapd is not running, the configuration file is fine");
        return Ok(());
    }
    // Wait for the old instance to release its single-instance lock.
    for _ in 0..50 {
        if !platform::is_running() {
            break;
        }
        sleep(Duration::from_millis(100));
    }
    platform::start()?;
    println!("imetapd reloaded the configuration");
    Ok(())
}

#[cfg(windows)]
mod platform {
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::Command;

    use anyhow::{Context, Result};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR, w};

    use crate::config::Config;
    use crate::daemon_path::daemon_exe;
    use crate::win::{Engine, app, autostart, init_com};

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn running_debug_log() -> Option<bool> {
        app::running_debug_log()
    }

    pub fn engine() -> Engine {
        init_com();
        Engine::new()
    }

    pub fn run(config: Config) -> Result<()> {
        app::run(config)
    }

    pub fn is_running() -> bool {
        app::is_running()
    }

    pub fn stop() -> Result<bool> {
        app::stop()
    }

    pub fn start() -> Result<()> {
        Command::new(daemon_exe()?)
            .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
            .spawn()
            .context("cannot start imetapd")?;
        Ok(())
    }

    pub fn autostart_enabled() -> bool {
        autostart::is_enabled()
    }

    pub fn set_autostart(enabled: bool, debug: bool) -> Result<()> {
        if enabled && !autostart::set(true, debug)? {
            eprintln!(
                "note: Windows delays this kind of login entry, sometimes by minutes. Run `imetap autostart enable` once from an elevated terminal to start at logon without the delay."
            );
        } else if !enabled {
            autostart::set(false, false)?;
        }
        Ok(())
    }

    pub fn open(path: &Path) -> Result<()> {
        // SAFETY: shell-opens a path. The HSTRING outlives the call.
        unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(path.as_os_str()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use anyhow::Result;

    use crate::config::Config;
    pub use crate::mac::{
        Engine, autostart_enabled, is_running, open, running_debug_log, set_autostart, start, stop,
    };

    pub fn engine() -> Engine {
        Engine::new()
    }

    pub fn run(config: Config) -> Result<()> {
        crate::mac::app::run(config)
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use std::path::Path;

    use anyhow::{Result, bail};

    use crate::config::Config;
    use crate::state::{Ime, Snapshot};

    pub struct Engine;

    impl Engine {
        pub fn set(&self, _: Ime) -> Result<()> {
            bail!("not supported on this platform yet")
        }
        pub fn toggle(&self) -> Result<()> {
            bail!("not supported on this platform yet")
        }
        pub fn snapshot(&self) -> Snapshot {
            Snapshot::new(Ime::Other, None)
        }
        pub fn describe(&self) -> String {
            String::new()
        }
    }

    pub fn running_debug_log() -> Option<bool> {
        None
    }
    pub fn engine() -> Engine {
        Engine
    }
    pub fn run(_: Config) -> Result<()> {
        bail!("not supported on this platform yet")
    }
    pub fn is_running() -> bool {
        false
    }
    pub fn stop() -> Result<bool> {
        bail!("not supported on this platform yet")
    }
    pub fn start() -> Result<()> {
        bail!("not supported on this platform yet")
    }
    pub fn autostart_enabled() -> bool {
        false
    }
    pub fn set_autostart(_: bool, _: bool) -> Result<()> {
        bail!("not supported on this platform yet")
    }
    pub fn open(_: &Path) -> Result<()> {
        bail!("not supported on this platform yet")
    }
}
