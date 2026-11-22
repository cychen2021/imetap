//! The debug log, which `IMETAP_DEBUG` or `run --debug` turns on, shared by both platforms.
//!
//! It records what the resident process reads and decides, one line per event, in the temporary
//! directory. The variable is read once, when the process first asks.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// The log stops growing at this size and starts over.
const MAX_BYTES: u64 = 2_000_000;

static ENABLED: OnceLock<bool> = OnceLock::new();

/// Whether the log is on: turned on by `enable`, or else by the environment variable
/// `IMETAP_DEBUG` as it was when this process asked first.
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| env::var_os("IMETAP_DEBUG").is_some())
}

/// Turns the log on for this process (`run --debug`). Call it before anything asks `enabled`.
pub fn enable() {
    let _ = ENABLED.set(true);
}

/// Where the log is written.
pub fn log_path() -> PathBuf {
    env::temp_dir().join("imetap-debug.log")
}

/// Appends a line to the log when it is enabled. The message is only built then.
pub fn log(message: impl FnOnce() -> String) {
    if !enabled() {
        return;
    }
    let path = log_path();
    // Start over once it gets big, so that leaving the switch on cannot fill the disk.
    if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = fs::remove_file(&path);
    }
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{secs:.3} {}", message());
    }
}
