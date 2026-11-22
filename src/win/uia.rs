//! UI Automation: the Chinese/English mode of Microsoft Pinyin, and whether focus is in a text field.
//!
//! Reads the Chinese/English mode of Microsoft Pinyin from the taskbar.
//!
//! Microsoft Pinyin offers no supported way to query its mode from another process in TSF-native
//! apps, but the taskbar's input indicator shows it ("Chinese Mode" / "English Mode"), and it is
//! exposed through UI Automation. This is also exactly what the user sees.

use anyhow::Result;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationValuePattern, TreeScope_Descendants,
    UIA_ClassNamePropertyId, UIA_IsTextPatternAvailablePropertyId, UIA_ValuePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetClassNameW, GetForegroundWindow};
use windows::core::w;

/// What UI Automation says about the focused element, as far as telling a text field apart goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusInfo {
    /// A `UIA_*ControlTypeId`.
    pub control_type: i32,
    /// Whether the element supports the text pattern.
    pub text_pattern: bool,
    /// `Some(read_only)` when the element supports the value pattern.
    pub value_read_only: Option<bool>,
    /// Whether the element is backed by a window of its own (custom-drawn apps expose just that).
    pub has_window: bool,
    /// Whether the element's window class belongs to Chromium (`Chrome_WidgetWin_1`,
    /// `Chrome_RenderWidgetHostHWND`), which also covers WebView2 apps such as the new Outlook.
    pub chromium: bool,
}

impl FocusInfo {
    /// A bare window or pane and nothing else: what a custom-drawn app exposes, and what a
    /// Chromium window shows until its accessibility tree is ready.
    pub fn is_bare_window(&self) -> bool {
        matches!(
            self.control_type,
            CONTROL_WINDOW | CONTROL_PANE | CONTROL_CUSTOM
        ) && self.has_window
            && !self.text_pattern
            && self.value_read_only.is_none()
    }
}

const CONTROL_EDIT: i32 = 50004;
const CONTROL_LIST_ITEM: i32 = 50007;
const CONTROL_TREE_ITEM: i32 = 50024;
const CONTROL_DATA_ITEM: i32 = 50029;
const CONTROL_CUSTOM: i32 = 50025;
const CONTROL_WINDOW: i32 = 50032;
const CONTROL_PANE: i32 = 50033;

/// Whether typing would go to a text field, given what UI Automation reports for the focus.
///
/// Browsers and Electron apps expose their page as a read-only `Document` and real inputs as
/// writable `Edit`s. Native and WinUI inputs are `Edit`s or writable value elements. Word-like
/// editors and Windows Terminal expose the text pattern. A custom-drawn window (WezTerm, games) shows
/// up only as a bare window, which tells nothing, so it counts as a text field: showing the HUD too
/// often is better than never.
pub fn is_text_field(info: FocusInfo) -> bool {
    if info.value_read_only == Some(true) {
        return false;
    }
    if info.control_type == CONTROL_EDIT {
        return true;
    }
    // Explorer's file and folder items expose a writable value (the rename name), but are not fields.
    let item = matches!(
        info.control_type,
        CONTROL_LIST_ITEM | CONTROL_TREE_ITEM | CONTROL_DATA_ITEM
    );
    if info.value_read_only == Some(false) && !item {
        return true;
    }
    if info.text_pattern {
        return true;
    }
    matches!(
        info.control_type,
        CONTROL_WINDOW | CONTROL_PANE | CONTROL_CUSTOM
    ) && info.has_window
}

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

/// The class name of the foreground window.
pub fn foreground_class() -> String {
    let mut buf = [0u16; 128];
    // SAFETY: `buf` is valid for the length passed.
    let len = unsafe { GetClassNameW(GetForegroundWindow(), &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

/// Whether the foreground window belongs to Chromium: Chrome, Edge, and every Electron app.
pub fn foreground_is_chromium() -> bool {
    foreground_class().starts_with("Chrome_WidgetWin")
}

/// Whether a window class is a piece of the Windows shell: the taskbar, the desktop, the task view,
/// Start, search and the Explorer window. They expose a bare window, but nothing in them is typed into by accident.
pub fn is_shell_class(class: &str) -> bool {
    matches!(
        class,
        "Shell_TrayWnd"
            | "CabinetWClass"
            | "Shell_SecondaryTrayWnd"
            | "Progman"
            | "WorkerW"
            | "TopLevelWindowForOverflowXamlIsland"
            | "XamlExplorerHostIslandWindow"
            | "MultitaskingViewFrame"
            | "Windows.UI.Core.CoreWindow"
    )
}

impl TrayReader {
    /// What UI Automation reports for the element that has keyboard focus.
    pub fn focus_info(&self) -> Option<FocusInfo> {
        // SAFETY: every call below works on COM objects owned by this function.
        unsafe {
            let element = self.uia.GetFocusedElement().ok()?;
            let flag = |id| {
                element
                    .GetCurrentPropertyValue(id)
                    .ok()
                    .and_then(|v| bool::try_from(&v).ok())
                    .unwrap_or(false)
            };
            let value_read_only = element
                .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .ok()
                .and_then(|p| p.CurrentIsReadOnly().ok())
                .map(|b| b.as_bool());
            Some(FocusInfo {
                control_type: element.CurrentControlType().ok()?.0,
                text_pattern: flag(UIA_IsTextPatternAvailablePropertyId),
                value_read_only,
                has_window: element
                    .CurrentNativeWindowHandle()
                    .is_ok_and(|h| !h.0.is_null()),
                chromium: element
                    .CurrentClassName()
                    .is_ok_and(|class| class.to_string().starts_with("Chrome_")),
            })
        }
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

    fn focus(control_type: i32, text: bool, read_only: Option<bool>, window: bool) -> FocusInfo {
        FocusInfo {
            control_type,
            text_pattern: text,
            value_read_only: read_only,
            has_window: window,
            chromium: false,
        }
    }

    #[test]
    fn shell_windows_are_recognised() {
        assert!(is_shell_class("Shell_TrayWnd"));
        assert!(is_shell_class("Windows.UI.Core.CoreWindow"));
        assert!(!is_shell_class("Chrome_WidgetWin_1"));
        assert!(!is_shell_class("Notepad"));
    }

    #[test]
    fn a_bare_window_is_recognised() {
        assert!(focus(50032, false, None, true).is_bare_window());
        assert!(!focus(50032, false, None, false).is_bare_window());
        assert!(!focus(50004, true, Some(false), true).is_bare_window());
        assert!(!focus(50033, true, None, true).is_bare_window());
    }

    #[test]
    fn writable_edits_are_text_fields() {
        // Edge's address bar, a ProseMirror editor in an Electron app.
        assert!(is_text_field(focus(50004, true, Some(false), false)));
        // A WinUI RichEditBox may expose no value pattern at all.
        assert!(is_text_field(focus(50004, true, None, false)));
    }

    #[test]
    fn a_read_only_page_is_not_a_text_field() {
        // The focused `Document` of a browser tab, and Edge's read-only `Textfield`.
        assert!(!is_text_field(focus(50030, true, Some(true), false)));
        assert!(!is_text_field(focus(50004, true, Some(true), false)));
    }

    #[test]
    fn editors_with_the_text_pattern_are_text_fields() {
        // Word's document, Windows Terminal's terminal control.
        assert!(is_text_field(focus(50030, true, None, false)));
        assert!(is_text_field(focus(50020, true, None, false)));
    }

    #[test]
    fn explorer_items_are_not_text_fields() {
        assert!(!is_text_field(focus(50007, false, Some(false), false)));
        assert!(!is_text_field(focus(50024, false, Some(false), false)));
        assert!(is_shell_class("CabinetWClass"));
    }

    #[test]
    fn a_writable_combo_box_is_a_text_field() {
        assert!(is_text_field(focus(50003, false, Some(false), false)));
    }

    #[test]
    fn buttons_and_lists_are_not_text_fields() {
        assert!(!is_text_field(focus(50000, false, None, false)));
        assert!(!is_text_field(focus(50007, false, None, false)));
        assert!(!is_text_field(focus(50033, false, None, false)));
    }

    #[test]
    fn a_bare_custom_drawn_window_counts_as_a_text_field() {
        // WezTerm and the like expose nothing but their window.
        assert!(is_text_field(focus(50032, false, None, true)));
        assert!(is_text_field(focus(50033, false, None, true)));
    }

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
