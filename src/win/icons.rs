//! The icons Windows shows for the input methods, found through the TSF registration.

use std::cell::RefCell;

use windows::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::Win32::UI::WindowsAndMessaging::HICON;
use windows::core::{HSTRING, PCWSTR};

use crate::state::Ime;

thread_local! {
    static CACHE: RefCell<Vec<(Ime, Option<HICON>)>> = const { RefCell::new(Vec::new()) };
}

/// The registry key of the language profile of a text input processor.
fn profile_key(ime: Ime) -> Option<&'static str> {
    match ime {
        Ime::Pinyin => Some(
            r"SOFTWARE\Microsoft\CTF\TIP\{81D4E9C9-1D3B-41BC-9E6C-4B40BF79E35E}\LanguageProfile\0x00000804\{FA550B04-5AD7-411F-A5AC-CA038EC515D7}",
        ),
        Ime::Doubao => Some(
            r"SOFTWARE\Microsoft\CTF\TIP\{9D2B2E2B-3C93-4D2F-9D35-6EEB85F0D2B0}\LanguageProfile\0x00000804\{2B4D4B3A-4D4F-4C0A-8E66-7F771A2B9C10}",
        ),
        _ => None,
    }
}

fn read_string(key: &str, value: &str) -> Option<String> {
    let mut buf = [0u16; 520];
    let mut bytes = (buf.len() * 2) as u32;
    // SAFETY: `buf` and `bytes` describe the same writable region.
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(key),
            &HSTRING::from(value),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
        .ok()
        .ok()?;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

fn read_dword(key: &str, value: &str) -> Option<i32> {
    let mut data = 0u32;
    let mut bytes = size_of::<u32>() as u32;
    // SAFETY: `data` is a writable DWORD.
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(key),
            &HSTRING::from(value),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut bytes),
        )
        .ok()
        .ok()?;
    }
    Some(data as i32)
}

fn load(ime: Ime) -> Option<HICON> {
    let key = profile_key(ime)?;
    let file = read_string(key, "IconFile")?;
    let index = read_dword(key, "IconIndex").unwrap_or(0);
    let mut icon = HICON::default();
    // SAFETY: extracts one large icon into `icon`; the file name outlives the call.
    let got = unsafe {
        ExtractIconExW(
            PCWSTR(HSTRING::from(file).as_ptr()),
            index,
            Some(&mut icon),
            None,
            1,
        )
    };
    (got > 0 && !icon.is_invalid()).then_some(icon)
}

/// The icon of an input method, if it has one. Loaded once and kept for the process lifetime.
pub fn icon_for(ime: Ime) -> Option<HICON> {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((_, icon)) = cache.iter().find(|(i, _)| *i == ime) {
            return *icon;
        }
        let icon = load(ime);
        cache.push((ime, icon));
        icon
    })
}
