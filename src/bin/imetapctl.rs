//! The console-subsystem command line for `imetap`: the same commands, with output that works in pipes.
//!
//! Only Windows needs it, because `imetap` is a GUI-subsystem program there. Elsewhere `imetap` already
//! prints to the terminal, so this is a stub that points to it.

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    imetap::cli::main(true)
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("imetapctl is only needed on Windows; use `imetap` instead");
    std::process::ExitCode::FAILURE
}
