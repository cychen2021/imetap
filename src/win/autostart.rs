//! "Run at login": a value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

use std::env;

use anyhow::Result;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegDeleteValueW,
    RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};
use windows::core::HSTRING;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "imetap";

/// Whether the Run entry exists.
pub fn is_enabled() -> bool {
    // SAFETY: a size-only query; no output buffer is passed.
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

/// Adds or removes the Run entry for the running executable.
pub fn set(enabled: bool) -> Result<()> {
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
            let command = format!("\"{}\" run", env::current_exe()?.display());
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
