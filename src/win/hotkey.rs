//! Windows' own IME on/off hotkey.
//!
//! Microsoft Pinyin keeps its Chinese/English state in an engine that lives in `ctfmon.exe` and is
//! reachable only through the system input stack. Messages to the IME window, TSF compartments and
//! `ImmSimulateHotKey` all fail in TSF-native apps such as the new Notepad. What does work is the
//! system IME hotkey `IME_CHOTKEY_IME_NONIME_TOGGLE` (id `0x10`, `Ctrl+Space` by default), so the
//! combination is read from the registry, where Windows stores it, and sent as input.

use anyhow::{Result, bail};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RegGetValueW};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput, VIRTUAL_KEY, VK_LCONTROL, VK_LMENU, VK_LSHIFT,
    VK_LWIN, VK_SPACE,
};
use windows::core::HSTRING;

const KEY: &str = r"Control Panel\Input Method\Hot Keys\00000010";

/// Modifier bits of `Key Modifiers`.
const MOD_ALT: u32 = 0x01;
const MOD_CONTROL: u32 = 0x02;
const MOD_SHIFT: u32 = 0x04;
const MOD_WIN: u32 = 0x08;

/// A hotkey: one virtual key plus modifier bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combo {
    pub vk: u16,
    pub mods: u32,
}

impl Combo {
    /// What Windows uses when the registry does not say otherwise.
    pub const DEFAULT: Combo = Combo {
        vk: VK_SPACE.0,
        mods: MOD_CONTROL,
    };
}

/// Decodes the two little-endian REG_BINARY DWORDs. A zero virtual key means "no hotkey".
pub fn decode(virtual_key: &[u8], modifiers: &[u8]) -> Option<Combo> {
    let dword = |b: &[u8]| Some(u32::from_le_bytes(b.get(..4)?.try_into().ok()?));
    let vk = dword(virtual_key)?;
    // Bits 0x4000 / 0x8000 only say "left" / "right" side of a modifier.
    let mods = dword(modifiers)? & 0xFF;
    (vk != 0).then_some(Combo {
        vk: vk as u16,
        mods,
    })
}

/// The configured combination, or the Windows default when it cannot be read.
pub fn configured() -> Combo {
    let read = |value: &str| -> Option<Vec<u8>> {
        let mut buf = [0u8; 8];
        let mut len = buf.len() as u32;
        // SAFETY: `buf` and `len` describe the same writable region.
        unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(KEY),
                &HSTRING::from(value),
                RRF_RT_REG_BINARY,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut len),
            )
            .ok()
            .ok()?;
        }
        Some(buf[..len as usize].to_vec())
    };
    read("Virtual Key")
        .zip(read("Key Modifiers"))
        .and_then(|(vk, mods)| decode(&vk, &mods))
        .unwrap_or(Combo::DEFAULT)
}

fn key_input(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    // SAFETY: mapping a virtual key to a scan code has no preconditions.
    let scan = unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) } as u16;
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Presses the configured IME toggle hotkey.
pub fn send_toggle() -> Result<()> {
    let combo = configured();
    let mods: Vec<VIRTUAL_KEY> = [
        (MOD_CONTROL, VK_LCONTROL),
        (MOD_SHIFT, VK_LSHIFT),
        (MOD_ALT, VK_LMENU),
        (MOD_WIN, VK_LWIN),
    ]
    .into_iter()
    .filter(|(bit, _)| combo.mods & bit != 0)
    .map(|(_, vk)| vk)
    .collect();
    let key = VIRTUAL_KEY(combo.vk);
    let mut inputs: Vec<INPUT> = mods.iter().map(|&m| key_input(m, false)).collect();
    inputs.push(key_input(key, false));
    inputs.push(key_input(key, true));
    inputs.extend(mods.iter().rev().map(|&m| key_input(m, true)));
    // SAFETY: `inputs` is a valid slice of fully initialised INPUT structures.
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        bail!(
            "SendInput injected {sent} of {} events (blocked by a higher-integrity window?)",
            inputs.len()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_windows_default() {
        // Values as found in the registry: VK=20 00 00 00, Key Modifiers=02 C0 00 00
        let combo = decode(&[0x20, 0, 0, 0], &[0x02, 0xC0, 0, 0]).unwrap();
        assert_eq!(combo, Combo::DEFAULT);
    }

    #[test]
    fn a_zero_virtual_key_means_no_hotkey() {
        assert_eq!(decode(&[0, 0, 0, 0], &[0x02, 0xC0, 0, 0]), None);
    }

    #[test]
    fn short_values_are_rejected() {
        assert_eq!(decode(&[0x20], &[0x02, 0xC0, 0, 0]), None);
    }

    #[test]
    fn side_flags_are_dropped_from_the_modifiers() {
        let combo = decode(&[0xBE, 0, 0, 0], &[0x03, 0xC0, 0, 0]).unwrap();
        assert_eq!(combo.mods, MOD_ALT | MOD_CONTROL);
    }
}
