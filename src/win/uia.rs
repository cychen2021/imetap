//! Reads the Chinese/English mode of Microsoft Pinyin from the taskbar.
//!
//! Microsoft Pinyin offers no supported way to query its mode from another process in TSF-native
//! apps, but the taskbar's input indicator shows it ("Chinese Mode" / "English Mode"), and it is
//! exposed through UI Automation. This is also exactly what the user sees.

use anyhow::Result;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, TreeScope_Descendants, UIA_ClassNamePropertyId,
};
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
use windows::core::w;

/// Looks up the mode indicator in the taskbar. Must be created and used on a COM-initialised thread.
pub struct TrayReader {
    uia: IUIAutomation,
}

impl TrayReader {
    pub fn new() -> Result<Self> {
        // SAFETY: plain in-process COM activation.
        Ok(Self {
            uia: unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)? },
        })
    }

    /// `Some(true)` for Chinese mode, `Some(false)` for English mode, `None` if the indicator is not
    /// found (the taskbar is not running, or the indicator is hidden).
    pub fn chinese_mode(&self) -> Option<bool> {
        // SAFETY: every call below works on COM objects owned by this function.
        unsafe {
            let tray = FindWindowW(w!("Shell_TrayWnd"), None).ok()?;
            let root = self.uia.ElementFromHandle(tray).ok()?;
            let class = self
                .uia
                .CreatePropertyCondition(
                    UIA_ClassNamePropertyId,
                    &VARIANT::from("SystemTray.NormalButton"),
                )
                .ok()?;
            let buttons = root.FindAll(TreeScope_Descendants, &class).ok()?;
            for i in 0..buttons.Length().ok()? {
                let name = buttons.GetElement(i).ok()?.CurrentName().ok()?.to_string();
                if let Some(mode) = parse_mode(&name) {
                    return Some(mode);
                }
            }
        }
        None
    }
}

/// Extracts the mode from the accessible name of a tray button, in English or Chinese Windows.
fn parse_mode(name: &str) -> Option<bool> {
    if !(name.contains("Mode") || name.contains("模式")) {
        return None;
    }
    if name.contains("Chinese") || name.contains("中文") {
        Some(true)
    } else if name.contains("English") || name.contains("英语") || name.contains("英文") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_english_windows_names() {
        assert_eq!(
            parse_mode("Tray Input Indicator Chinese Mode\r\n\r\nRight-click for more options"),
            Some(true)
        );
        assert_eq!(parse_mode("Tray Input Indicator English Mode"), Some(false));
    }

    #[test]
    fn parses_chinese_windows_names() {
        assert_eq!(parse_mode("托盘输入指示器 中文模式"), Some(true));
        assert_eq!(parse_mode("托盘输入指示器 英语模式"), Some(false));
    }

    #[test]
    fn the_language_button_is_not_a_mode() {
        assert_eq!(
            parse_mode(
                "Tray Input Indicator Chinese (Simplified, Mainland China)\r\nMicrosoft Pinyin"
            ),
            None
        );
        assert_eq!(parse_mode("Wi-Fi"), None);
    }
}
