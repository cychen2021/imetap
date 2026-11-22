// A GUI-subsystem binary, so that the resident process and the login entry never flash a console
// window. The management commands attach to the parent's console instead. `imetapctl` is the
// console-subsystem twin for terminals and scripts.
#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::Result;

fn main() -> Result<()> {
    imetap::cli::main(false)
}
