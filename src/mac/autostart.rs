//! "Run at login", through a launch agent.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::daemon_path::daemon_exe;

const LAUNCH_AGENT: &str = "io.github.cychen2021.imetap";

fn launch_agent_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{LAUNCH_AGENT}.plist")))
}

fn launch_agent_plist(exe: &str, debug: bool) -> String {
    let debug = if debug {
        "\t\t<string>--debug</string>\n"
    } else {
        ""
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>Label</key>\n\t<string>{LAUNCH_AGENT}</string>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{exe}</string>\n{debug}\t</array>\n\
         \t<key>RunAtLoad</key>\n\t<true/>\n\
         </dict>\n</plist>\n"
    )
}

/// Whether the launch agent is installed.
pub fn is_enabled() -> bool {
    launch_agent_path().map(|p| p.exists()).unwrap_or(false)
}

/// Enables or disables starting at login. With `debug`, the started process writes the debug log.
pub fn set(enabled: bool, debug: bool) -> Result<()> {
    let path = launch_agent_path()?;
    if enabled {
        let exe = daemon_exe()?;
        fs::create_dir_all(path.parent().context("no parent directory")?)?;
        fs::write(&path, launch_agent_plist(&exe.display().to_string(), debug))?;
    } else if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_agent_passes_debug_only_when_asked() {
        let plain = launch_agent_plist("/bin/imetapd", false);
        assert!(plain.contains("<string>/bin/imetapd</string>\n\t</array>"));
        assert!(!plain.contains("--debug"));
        let debug = launch_agent_plist("/bin/imetapd", true);
        assert!(
            debug.contains(
                "<string>/bin/imetapd</string>\n\t\t<string>--debug</string>\n\t</array>"
            )
        );
    }
}
