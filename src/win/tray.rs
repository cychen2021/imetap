//! The tray icon, which shows the mode, and its menu.

use std::fs;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC,
    DeleteObject, FillRect, GetDC, HGDIOBJ, ReleaseDC, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconFromResourceEx, CreateIconIndirect, CreatePopupMenu, DestroyIcon,
    DestroyMenu, GetCursorPos, HICON, ICONINFO, LR_DEFAULTCOLOR, MF_CHECKED, MF_SEPARATOR,
    MF_STRING, PostMessageW, PostQuitMessage, SW_SHOWNORMAL, SetForegroundWindow, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
};
use windows::core::{HSTRING, PCWSTR, w};

use super::app::WM_TRAY;
use super::autostart;
use super::hud::draw_text;
use crate::config;
use crate::debug;
use crate::state::Snapshot;

const CMD_AUTOSTART: usize = 1;
const CMD_CONFIG: usize = 2;
const CMD_QUIT: usize = 3;

const TRAY_ID: u32 = 1;

fn tray_data(hwnd: HWND, icon: HICON, tip: &str) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    for (slot, unit) in data.szTip.iter_mut().zip(tip.encode_utf16().take(127)) {
        *slot = unit;
    }
    data
}

/// Adds the tray icon (`add`) or changes it. With `lang_indicator` the icon shows the mode glyph,
/// otherwise the logo. Returns the new icon handle, which the caller frees with `free_icon` once
/// it is replaced.
pub(super) fn show(
    hwnd: HWND,
    snapshot: Option<Snapshot>,
    lang_indicator: bool,
    add: bool,
) -> HICON {
    // The tooltip is just the tool name: the state is already on the icon and in the input bar.
    // The HUD says "EN", but the tray icon is small, so it shows a single "E" like the macOS one.
    let icon = if lang_indicator {
        let glyph = match snapshot.map_or("?", |s| s.glyph()) {
            "EN" => "E",
            other => other,
        };
        make_icon(glyph, snapshot)
    } else {
        logo_icon()
    };
    let data = tray_data(hwnd, icon, "imetap");
    // SAFETY: `data` is fully initialised for the call.
    unsafe {
        let _ = Shell_NotifyIconW(if add { NIM_ADD } else { NIM_MODIFY }, &data);
    }
    icon
}

/// Frees an icon that the tray no longer shows.
pub(super) fn free_icon(icon: HICON) {
    // SAFETY: the shell has switched to another icon. The old handle is ours to free.
    unsafe { DestroyIcon(icon).ok() };
}

/// Removes the tray icon.
pub(super) fn remove(hwnd: HWND) {
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    };
    // SAFETY: removes the icon added by `show`.
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

fn icon_color(snapshot: Option<Snapshot>) -> COLORREF {
    let rgb = |r: u32, g: u32, b: u32| COLORREF(r | g << 8 | b << 16);
    match snapshot {
        Some(s) if s.chinese == Some(true) => rgb(196, 48, 43),
        Some(_) => rgb(37, 99, 170),
        None => rgb(90, 90, 90),
    }
}

/// The imetap logo as a 32 x 32 icon, for `tray.enable_lang_indicator = false`.
fn logo_icon() -> HICON {
    const LOGO: &[u8] = include_bytes!("../../assets/tray.png");
    // SAFETY: the bytes are a PNG icon image that lives for the whole program. The icon owns a copy.
    unsafe {
        CreateIconFromResourceEx(LOGO, true, 0x0003_0000, 32, 32, LR_DEFAULTCOLOR)
            .unwrap_or_default()
    }
}

/// A 32 x 32 icon: the mode glyph in white on a colour that tells the mode at a glance.
fn make_icon(glyph: &str, snapshot: Option<Snapshot>) -> HICON {
    const SIZE: i32 = 32;
    // SAFETY: GDI objects created here are released before returning. The icon owns copies.
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let color = CreateCompatibleBitmap(screen, SIZE, SIZE);
        let mask = CreateBitmap(SIZE, SIZE, 1, 1, None);
        let old = SelectObject(mem, HGDIOBJ(color.0));
        let rect = RECT {
            left: 0,
            top: 0,
            right: SIZE,
            bottom: SIZE,
        };
        let brush = CreateSolidBrush(icon_color(snapshot));
        FillRect(mem, &rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
        SetBkMode(mem, TRANSPARENT);
        SetTextColor(mem, COLORREF(0x00FF_FFFF));
        draw_text(
            mem,
            glyph,
            rect,
            match glyph {
                "E" => 26,
                _ if glyph.is_ascii() => 17,
                _ => 26,
            },
            700,
        );
        SelectObject(mem, old);
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        icon
    }
}

/// Shows the tray menu at the cursor and carries out the chosen command.
pub(super) fn show_menu(hwnd: HWND) {
    // SAFETY: builds, shows and destroys a popup menu on this thread. No `APP` borrow is held, since
    // the menu runs a nested message loop.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let autostart_flags = if autostart::is_enabled() {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        let _ = AppendMenuW(menu, autostart_flags, CMD_AUTOSTART, w!("Run at login"));
        let _ = AppendMenuW(menu, MF_STRING, CMD_CONFIG, w!("Open config file"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, w!("Quit"));
        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        // The menu closes on an outside click only if its owner is the foreground window.
        let _ = SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            at.x,
            at.y,
            None,
            hwnd,
            None,
        )
        .0 as usize;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match command {
            CMD_AUTOSTART => {
                if let Err(e) = autostart::set(!autostart::is_enabled(), debug::enabled()) {
                    eprintln!("imetap: cannot change run at login: {e:#}");
                }
            }
            CMD_CONFIG => {
                if let Err(e) = open_config() {
                    eprintln!("imetap: cannot open the config file: {e:#}");
                }
            }
            CMD_QUIT => PostQuitMessage(0),
            _ => {}
        }
    }
}

/// Opens `config.toml` in the default editor, creating a commented template first if needed.
fn open_config() -> Result<()> {
    let path = config::config_path().context("no configuration directory")?;
    if !path.exists() {
        fs::create_dir_all(path.parent().context("no parent directory")?)?;
        fs::write(&path, config::DEFAULT_TOML)?;
    }
    // SAFETY: shell-opens a file path. The HSTRING outlives the call.
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(path.as_os_str()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logo_loads_as_an_icon() {
        let icon = logo_icon();
        assert!(!icon.is_invalid());
        // SAFETY: the handle was created above and is not used afterwards.
        unsafe { DestroyIcon(icon).ok() };
    }
}
