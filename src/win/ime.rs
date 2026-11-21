//! Switching and identifying input methods through the Text Services Framework.

use anyhow::{Result, bail};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use windows::Win32::UI::TextServices::{
    CLSID_TF_InputProcessorProfiles, GUID_TFCAT_TIP_KEYBOARD, ITfInputProcessorProfileMgr,
    TF_INPUTPROCESSORPROFILE, TF_IPPMF_FORSESSION, TF_PROFILETYPE_INPUTPROCESSOR,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, PostMessageW, WM_INPUTLANGCHANGEREQUEST,
};
use windows::core::GUID;

use crate::state::Ime;

/// Simplified Chinese (PRC), the language id the Chinese IMEs register under.
const LANG_ZH_CN: u16 = 0x0804;
/// The US keyboard layout handle.
const HKL_US: isize = 0x0409_0409;

/// `(CLSID, profile GUID)` of the TSF text input processors.
const PINYIN: (GUID, GUID) = (
    GUID::from_u128(0x81D4E9C9_1D3B_41BC_9E6C_4B40BF79E35E),
    GUID::from_u128(0xFA550B04_5AD7_411F_A5AC_CA038EC515D7),
);
const DOUBAO: (GUID, GUID) = (
    GUID::from_u128(0x9D2B2E2B_3C93_4D2F_9D35_6EEB85F0D2B0),
    GUID::from_u128(0x2B4D4B3A_4D4F_4C0A_8E66_7F771A2B9C10),
);

fn profile_mgr() -> Result<ITfInputProcessorProfileMgr> {
    // SAFETY: plain COM activation of an in-process system class on a COM-initialised thread.
    Ok(unsafe { CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)? })
}

/// Switches the input method. The US layout is requested from the foreground window; the Chinese
/// IMEs are activated for the whole session.
pub fn activate(ime: Ime) -> Result<()> {
    let (clsid, profile) = match ime {
        Ime::Us => {
            // ActivateProfile does not change the layout of the foreground window for plain
            // keyboard layouts, so ask the window directly, as `im-select` does.
            // SAFETY: posting a message to a window handle is always sound.
            unsafe {
                PostMessageW(
                    Some(GetForegroundWindow()),
                    WM_INPUTLANGCHANGEREQUEST,
                    WPARAM(0),
                    LPARAM(HKL_US),
                )?
            };
            return Ok(());
        }
        Ime::Pinyin => PINYIN,
        Ime::Doubao => DOUBAO,
        other => bail!("{} is not available on Windows", other.name()),
    };
    // SAFETY: the GUID pointers outlive the call.
    unsafe {
        profile_mgr()?.ActivateProfile(
            TF_PROFILETYPE_INPUTPROCESSOR,
            LANG_ZH_CN,
            &clsid,
            &profile,
            Default::default(),
            TF_IPPMF_FORSESSION,
        )?;
    }
    Ok(())
}

/// The keyboard layout handle of the foreground window's thread.
pub fn layout() -> usize {
    // SAFETY: querying the layout of a window's thread has no preconditions.
    unsafe {
        let thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        GetKeyboardLayout(thread).0 as usize
    }
}

/// Which input method the foreground window is using.
pub fn current() -> Ime {
    let lang = (layout() & 0xFFFF) as u16;
    if lang == 0x0409 {
        return Ime::Us;
    }
    if lang != LANG_ZH_CN {
        return Ime::Other;
    }
    match active_tip() {
        Some(clsid) if clsid == PINYIN.0 => Ime::Pinyin,
        Some(clsid) if clsid == DOUBAO.0 => Ime::Doubao,
        _ => Ime::Other,
    }
}

/// CLSID of the active keyboard text input processor.
pub fn active_tip() -> Option<GUID> {
    let mgr = profile_mgr().ok()?;
    let mut profile = TF_INPUTPROCESSORPROFILE::default();
    // SAFETY: `profile` is a valid out parameter for the duration of the call.
    unsafe {
        mgr.GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &mut profile)
            .ok()?
    };
    Some(profile.clsid)
}
