// A GUI-subsystem binary, so that the resident process and the login entry never flash a console
// window; the management commands attach to the parent's console instead.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::fs;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};

use imetap::config::{self, Config};
use imetap::state::Ime;

/// Switch input methods, toggle Chinese/English and show a HUD for each change.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run in the foreground (what `start` and the login entry launch).
    Run,
    /// Start the resident process in the background.
    Start,
    /// Stop the resident process.
    Stop,
    /// Stop and start again, e.g. after editing the config.
    Restart,
    /// Switch the input method: us, pinyin, doubao (Windows), shuangpin (macOS).
    Set { ime: String },
    /// Toggle Chinese/English in the current input method.
    Toggle,
    /// Show whether the resident process runs, and the foreground input method state.
    Status {
        /// Also print the raw keyboard layout and the active text input processor.
        #[arg(short, long)]
        verbose: bool,
    },
    /// Manage starting at login.
    Autostart {
        #[command(subcommand)]
        action: AutostartAction,
    },
    /// Manage the configuration file.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
enum AutostartAction {
    /// Start imetap at login.
    Enable,
    /// Do not start imetap at login.
    Disable,
    /// Show whether it starts at login.
    Status,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Print the path of the configuration file.
    Path,
    /// Write a commented default file, unless one exists.
    Init,
    /// Open the file in the default editor (created first if missing).
    Edit,
    /// Check the file for errors and print the effective settings.
    Check,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Run);
    if !matches!(command, Command::Run) {
        platform::attach_console();
    }
    match command {
        Command::Run => run(),
        Command::Start => {
            if platform::is_running() {
                println!("imetap is already running");
            } else {
                platform::start()?;
                println!("imetap started");
            }
            Ok(())
        }
        Command::Stop => {
            println!(
                "{}",
                if platform::stop()? {
                    "imetap stopped"
                } else {
                    "imetap is not running"
                }
            );
            Ok(())
        }
        Command::Restart => {
            if platform::stop()? {
                // Wait for the old instance to release its single-instance lock.
                for _ in 0..50 {
                    if !platform::is_running() {
                        break;
                    }
                    sleep(Duration::from_millis(100));
                }
            }
            platform::start()?;
            println!("imetap restarted");
            Ok(())
        }
        Command::Set { ime } => {
            let ime = Ime::parse(&ime).ok_or_else(|| anyhow!("unknown input method `{ime}`"))?;
            platform::engine().set(ime)
        }
        Command::Toggle => platform::engine().toggle(),
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
            let engine = platform::engine();
            println!("input:   {}", engine.snapshot());
            if verbose {
                println!("         {}", engine.describe());
            }
            Ok(())
        }
        Command::Autostart { action } => match action {
            AutostartAction::Enable => {
                platform::set_autostart(true)?;
                println!("imetap will start at login");
                Ok(())
            }
            AutostartAction::Disable => {
                platform::set_autostart(false)?;
                println!("imetap will not start at login");
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
        Command::Config { action } => manage_config(action),
    }
}

fn run() -> Result<()> {
    let config = match config::config_path() {
        Some(path) => Config::load(&path)?,
        None => Config::default(),
    };
    platform::run(config)
}

fn manage_config(action: ConfigAction) -> Result<()> {
    let path = config::config_path().context("no configuration directory on this system")?;
    match action {
        ConfigAction::Path => println!("{}", path.display()),
        ConfigAction::Init => {
            if path.exists() {
                bail!("{} already exists", path.display());
            }
            fs::create_dir_all(path.parent().context("no parent directory")?)?;
            fs::write(&path, config::DEFAULT_TOML)?;
            println!("wrote {}", path.display());
        }
        ConfigAction::Edit => {
            if !path.exists() {
                fs::create_dir_all(path.parent().context("no parent directory")?)?;
                fs::write(&path, config::DEFAULT_TOML)?;
            }
            platform::open(&path)?;
        }
        ConfigAction::Check => {
            let source = if path.exists() {
                path.display().to_string()
            } else {
                "defaults (no file)".to_string()
            };
            let config = Config::load(&path)?;
            println!("ok: {source}");
            println!("{config:#?}");
        }
    }
    Ok(())
}

#[cfg(windows)]
mod platform {
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::Command;

    use anyhow::{Context, Result};
    use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR, w};

    use imetap::config::Config;
    use imetap::win::{Engine, app, autostart, init_com};

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// Lets `println!` reach the terminal this process was started from.
    pub fn attach_console() {
        // SAFETY: harmless if the parent has no console.
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS).ok() };
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
        let exe = std::env::current_exe()?;
        Command::new(exe)
            .arg("run")
            .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
            .spawn()
            .context("cannot start imetap")?;
        Ok(())
    }

    pub fn autostart_enabled() -> bool {
        autostart::is_enabled()
    }

    pub fn set_autostart(enabled: bool) -> Result<()> {
        if enabled && !autostart::set(true)? {
            eprintln!(
                "note: Windows delays this kind of login entry, sometimes by minutes. Run `imetap autostart enable` once from an elevated terminal to start at logon without the delay."
            );
        } else if !enabled {
            autostart::set(false)?;
        }
        Ok(())
    }

    pub fn open(path: &Path) -> Result<()> {
        // SAFETY: shell-opens a path; the HSTRING outlives the call.
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

    use imetap::config::Config;
    pub use imetap::mac::{
        Engine, autostart_enabled, is_running, open, set_autostart, start, stop,
    };

    pub fn attach_console() {}

    pub fn engine() -> Engine {
        Engine::new()
    }

    pub fn run(config: Config) -> Result<()> {
        imetap::mac::app::run(config)
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use std::path::Path;

    use anyhow::{Result, bail};

    use imetap::config::Config;
    use imetap::state::{Ime, Snapshot};

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

    pub fn attach_console() {}
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
    pub fn set_autostart(_: bool) -> Result<()> {
        bail!("not supported on this platform yet")
    }
    pub fn open(_: &Path) -> Result<()> {
        bail!("not supported on this platform yet")
    }
}
