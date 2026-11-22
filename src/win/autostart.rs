//! "Run at login".
//!
//! Windows holds back `Run` entries (and the Startup folder) until the desktop has settled, which on
//! a busy login can take minutes. A scheduled logon task starts right at logon, but creating one
//! needs an elevated terminal. So enabling tries the task first and always also sets the `Run`
//! value as the fallback. A second instance exits immediately thanks to the single-instance lock.

use std::env;
use std::os::windows::process::CommandExt;
use std::process::Command;

use crate::daemon_exe;

use anyhow::Result;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegDeleteValueW,
    RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};
use windows::core::HSTRING;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "imetap";
const TASK: &str = "imetap";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn schtasks(args: &[&str]) -> bool {
    Command::new("schtasks")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Whether the logon task exists.
pub fn task_exists() -> bool {
    schtasks(&["/Query", "/TN", TASK])
}

/// Creates the logon task for the current user. Fails without elevation.
fn create_task() -> bool {
    let (Ok(exe), Ok(user), Ok(domain)) =
        (daemon_exe(), env::var("USERNAME"), env::var("USERDOMAIN"))
    else {
        return false;
    };
    let command = format!("\"{}\" run", exe.display());
    let user = format!("{domain}\\{user}");
    schtasks(&[
        "/Create", "/TN", TASK, "/TR", &command, "/SC", "ONLOGON", "/RU", &user, "/IT", "/RL",
        "LIMITED", "/F",
    ])
}

/// Whether imetap is set to start at login, by the logon task or the Run entry.
pub fn is_enabled() -> bool {
    task_exists() || run_entry_exists()
}

fn run_entry_exists() -> bool {
    // SAFETY: a size-only query. No output buffer is passed.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            &HSTRING::from(VALUE),
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        )
        .is_ok()
    }
}

/// Enables or disables starting at login. When enabling, returns whether the fast logon task
/// could be created, which needs an elevated terminal. The Run entry is set either way.
pub fn set(enabled: bool) -> Result<bool> {
    if enabled {
        let task = create_task();
        set_run_entry(true)?;
        Ok(task)
    } else {
        let _ = schtasks(&["/Delete", "/TN", TASK, "/F"]);
        set_run_entry(false)?;
        Ok(false)
    }
}

fn set_run_entry(enabled: bool) -> Result<()> {
    // SAFETY: the key handle is closed before returning and the data slice outlives the call.
    unsafe {
        let mut key = HKEY::default();
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            None,
            KEY_SET_VALUE,
            &mut key,
        )
        .ok()?;
        let result = if enabled {
            let command = format!("\"{}\" run", daemon_exe()?.display());
            let wide: Vec<u8> = command
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect();
            RegSetValueExW(key, &HSTRING::from(VALUE), None, REG_SZ, Some(&wide)).ok()
        } else {
            // Removing an entry that is not there is fine.
            RegDeleteValueW(key, &HSTRING::from(VALUE)).ok().or(Ok(()))
        };
        let _ = RegCloseKey(key);
        Ok(result?)
    }
}
