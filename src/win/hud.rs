//! The HUD: a translucent, click-through, always-on-top box in the middle of the primary monitor.

use std::cell::RefCell;

use anyhow::Result;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DT_CENTER, DT_SINGLELINE,
    DT_VCENTER, DeleteObject, DrawTextW, EndPaint, FillRect, GetMonitorInfoW, HGDIOBJ,
    InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow, PAINTSTRUCT,
    SelectObject, SetBkMode, SetTextColor, SetWindowRgn, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DI_NORMAL, DefWindowProcW, DrawIconEx, GetForegroundWindow, HICON,
    HWND_TOPMOST, KillTimer, LWA_ALPHA, RegisterClassExW, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, WM_PAINT, WM_TIMER,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{HSTRING, PCWSTR, w};

use super::icons;
use crate::config::HudConfig;
use crate::state::{HudColors, Rgb, Snapshot};

/// Timer that starts the fade-out once the hold time is over.
const TIMER_HOLD: usize = 1;
/// Timer that steps the fade-out.
const TIMER_FADE: usize = 2;
const FADE_STEPS: u32 = 8;
const FADE_STEP_MS: u32 = 20;

/// Logical size of the box (at 96 DPI).
const WIDTH: i32 = 220;
const HEIGHT: i32 = 116;
/// Gap between the box and the bottom edge of the monitor's work area.
const BOTTOM_MARGIN: i32 = 48;

struct Shown {
    snapshot: Snapshot,
    icon: Option<HICON>,
    alpha: u8,
    fade_step: u32,
}

thread_local! {
    static SHOWN: RefCell<Option<Shown>> = const { RefCell::new(None) };
}

/// The overlay window. Create it on the thread that runs the message loop and use it only there.
pub struct Hud {
    hwnd: HWND,
}

impl Hud {
    pub fn new() -> Result<Hud> {
        // SAFETY: registers a window class and creates a window owned by this thread.
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: w!("imetap_hud"),
                ..Default::default()
            };
            RegisterClassExW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW,
                w!("imetap_hud"),
                PCWSTR::null(),
                WS_POPUP,
                0,
                0,
                WIDTH,
                HEIGHT,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            Ok(Hud { hwnd })
        }
    }

    /// Shows the snapshot in the middle of the primary monitor.
    pub fn show(&self, snapshot: Snapshot, config: &HudConfig) {
        let alpha = (config.opacity.clamp(0.0, 1.0) * 255.0) as u8;
        SHOWN.with(|s| {
            *s.borrow_mut() = Some(Shown {
                snapshot,
                icon: icons::icon_for(snapshot.ime),
                alpha,
                fade_step: 0,
            })
        });
        // SAFETY: all calls operate on the window owned by `self` or on stack values.
        unsafe {
            KillTimer(Some(self.hwnd), TIMER_FADE).ok();
            let scale = |v: i32| v * GetDpiForSystem() as i32 / 96;
            let (width, height) = (scale(WIDTH), scale(HEIGHT));
            let mut monitor = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            // The monitor of the foreground window; the primary one if there is none.
            let target = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
            let _ = GetMonitorInfoW(target, &mut monitor);
            let area = monitor.rcWork;
            let (x, y) = (
                area.left + (area.right - area.left - width) / 2,
                area.bottom - height - scale(BOTTOM_MARGIN),
            );
            SetLayeredWindowAttributes(self.hwnd, COLORREF(0), alpha, LWA_ALPHA).ok();
            let radius = scale(24);
            SetWindowRgn(
                self.hwnd,
                Some(CreateRoundRectRgn(
                    0,
                    0,
                    width + 1,
                    height + 1,
                    radius,
                    radius,
                )),
                true,
            );
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
            .ok();
            let _ = InvalidateRect(Some(self.hwnd), None, true);
            SetTimer(
                Some(self.hwnd),
                TIMER_HOLD,
                config.duration_ms.max(1) as u32,
                None,
            );
        }
    }
}

fn rgb([r, g, b]: Rgb) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

/// Draws `text` centred in `area` with a font of the given pixel height.
///
/// SAFETY: `hdc` must be a valid device context.
pub(super) unsafe fn draw_text(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    text: &str,
    mut area: RECT,
    pixels: i32,
    weight: i32,
) {
    unsafe {
        let font = CreateFontW(
            -pixels,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            0,
            &HSTRING::from("Microsoft YaHei UI"),
        );
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        DrawTextW(
            hdc,
            &mut wide,
            &mut area,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER,
        );
        SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(font.0));
    }
}

fn paint(hwnd: HWND) {
    // SAFETY: standard WM_PAINT sequence on the window being painted.
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let colors = SHOWN.with(|s| {
            s.borrow()
                .as_ref()
                .map_or(HudColors::UNKNOWN, |shown| shown.snapshot.hud_colors())
        });
        let brush = CreateSolidBrush(rgb(colors.background));
        FillRect(hdc, &ps.rcPaint, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
        SHOWN.with(|s| {
            if let Some(shown) = s.borrow().as_ref() {
                let dpi = GetDpiForSystem() as i32;
                let px = |v: i32| v * dpi / 96;
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, rgb(colors.glyph));
                let glyph = RECT {
                    left: 0,
                    top: px(6),
                    right: px(WIDTH),
                    bottom: px(80),
                };
                draw_text(hdc, shown.snapshot.glyph(), glyph, px(60), 600);
                if let Some(icon) = shown.icon {
                    // The input method's own icon in the top-left corner.
                    let _ = DrawIconEx(
                        hdc,
                        px(14),
                        px(12),
                        icon,
                        px(26),
                        px(26),
                        0,
                        None,
                        DI_NORMAL,
                    );
                }
                SetTextColor(hdc, rgb(colors.caption));
                let caption = RECT {
                    left: 0,
                    top: px(78),
                    right: px(WIDTH),
                    bottom: px(HEIGHT - 8),
                };
                draw_text(hdc, &shown.snapshot.caption(), caption, px(15), 400);
            }
        });
        let _ = EndPaint(hwnd, &ps);
    }
}

fn on_timer(hwnd: HWND, id: usize) {
    // SAFETY: timers and layered attributes of the window being processed.
    unsafe {
        match id {
            TIMER_HOLD => {
                KillTimer(Some(hwnd), TIMER_HOLD).ok();
                SetTimer(Some(hwnd), TIMER_FADE, FADE_STEP_MS, None);
            }
            TIMER_FADE => {
                let alpha = SHOWN.with(|s| {
                    let mut s = s.borrow_mut();
                    let shown = s.as_mut()?;
                    shown.fade_step += 1;
                    Some(
                        (shown.alpha as u32 * (FADE_STEPS - shown.fade_step.min(FADE_STEPS))
                            / FADE_STEPS) as u8,
                    )
                });
                match alpha {
                    Some(a) if a > 0 => {
                        SetLayeredWindowAttributes(hwnd, COLORREF(0), a, LWA_ALPHA).ok();
                    }
                    _ => {
                        KillTimer(Some(hwnd), TIMER_FADE).ok();
                        let _ = ShowWindow(hwnd, SW_HIDE);
                    }
                }
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_TIMER => {
            on_timer(hwnd, wparam.0);
            LRESULT(0)
        }
        // SAFETY: forwarding an unhandled message with its original arguments.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
