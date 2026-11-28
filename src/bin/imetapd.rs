//! `imetapd`, the resident process: hotkeys, the Ctrl double-tap, the HUD and the tray icon.
//!
//! A GUI-subsystem program on Windows, so that it and the login entry never flash a console window.
//! `imetap daemon start` and the login entry launch it.
#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::Result;
use clap::Parser;

/// Resident process of imetap. Run `imetap --help` for the command line.
#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// Write the debug log, the same as setting `IMETAP_DEBUG`.
    #[arg(long)]
    debug: bool,
}

fn main() -> Result<()> {
    imetap::cli::run_daemon(Args::parse().debug)
}
