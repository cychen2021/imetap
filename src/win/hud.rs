//! The HUD: a translucent, click-through, always-on-top box in the middle of the primary monitor.

use std::cell::RefCell;
use std::time::Instant;

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
    CreateWindowExW, DefWindowProcW, GetForegroundWindow, HWND_TOPMOST, IsWindowVisible, KillTimer,
    LWA_ALPHA, RegisterClassExW, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, WM_PAINT, WM_TIMER,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::config::HudConfig;
use crate::fade::Fade;
use crate::state::{Color, HudStyle, Rgb, Snapshot};

/// Timer that steps the brightness: fade in, hold, fade out.
const TIMER_FADE: usize = 1;
const FADE_STEP_MS: u32 = 15;

/// Logical size of the box (at 96 DPI).
const WIDTH: i32 = 240;
/// Width of the stripe on the left that names the input method.
const STRIPE: i32 = 60;
const HEIGHT: i32 = 100;
/// Gap between the box and the bottom edge of the monitor's work area.
const BOTTOM_MARGIN: i32 = 48;

struct Shown {
    snapshot: Snapshot,
    style: HudStyle,
    /// The window's alpha at full brightness, from the configured opacity.
    alpha: u8,
    fade: Fade,
    /// When this appearance started, and how far into the curve it began (non-zero when it took
    /// over from one that was still visible).
    started: Instant,
    offset_ms: u64,
    /// The last brightness set, from 0.0 to 1.0.
    level: f32,
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
        let fade = config.fade();
        // Showing again while still visible carries on from the current brightness.
        let previous = SHOWN.with(|s| s.borrow().as_ref().map(|shown| shown.level));
        // SAFETY: querying the visibility of the window owned by `self`.
        let visible = unsafe { IsWindowVisible(self.hwnd).as_bool() };
        let offset_ms = match previous {
            Some(level) if visible => fade.start_for(level),
            _ => 0,
        };
        let level = fade.level(offset_ms);
        SHOWN.with(|s| {
            *s.borrow_mut() = Some(Shown {
                snapshot,
                style: config.style(snapshot),
                alpha,
                fade,
                started: Instant::now(),
                offset_ms,
                level,
            })
        });
        // SAFETY: all calls operate on the window owned by `self` or on stack values.
        unsafe {
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
            SetLayeredWindowAttributes(
                self.hwnd,
                COLORREF(0),
                (alpha as f32 * level) as u8,
                LWA_ALPHA,
            )
            .ok();
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
            SetTimer(Some(self.hwnd), TIMER_FADE, FADE_STEP_MS, None);
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
        SHOWN.with(|s| {
            let s = s.borrow();
            let Some(shown) = s.as_ref() else {
                // Nothing to show yet: a plain dark box.
                let brush = CreateSolidBrush(rgb(Color::Black.fill()));
                FillRect(hdc, &ps.rcPaint, brush);
                let _ = DeleteObject(HGDIOBJ(brush.0));
                return;
            };
            let colors = shown.style.colors;
            let dpi = GetDpiForSystem() as i32;
            let px = |v: i32| v * dpi / 96;
            let fill = |area: &RECT, color: Rgb| {
                let brush = CreateSolidBrush(rgb(color));
                FillRect(hdc, area, brush);
                let _ = DeleteObject(HGDIOBJ(brush.0));
            };
            fill(&ps.rcPaint, colors.background);
            let stripe = RECT {
                left: 0,
                top: 0,
                right: px(STRIPE),
                bottom: px(HEIGHT),
            };
            fill(&stripe, colors.stripe);
            SetBkMode(hdc, TRANSPARENT);
            // The input method's label, big, in the middle of the stripe.
            SetTextColor(hdc, rgb(colors.stripe_text));
            let size = if shown.style.label.chars().count() > 1 {
                32
            } else {
                48
            };
            draw_text(hdc, &shown.style.label, stripe, px(size), 700);
            // The mode, big, in the rest of the box.
            SetTextColor(hdc, rgb(colors.glyph));
            let glyph = RECT {
                left: px(STRIPE),
                top: 0,
                right: px(WIDTH),
                bottom: px(HEIGHT),
            };
            draw_text(hdc, shown.snapshot.glyph(), glyph, px(60), 600);
        });
        let _ = EndPaint(hwnd, &ps);
    }
}

fn on_timer(hwnd: HWND, id: usize) {
    if id != TIMER_FADE {
        return;
    }
    // The alpha for now, or `None` once it is over.
    let alpha = SHOWN.with(|s| {
        let mut s = s.borrow_mut();
        let shown = s.as_mut()?;
        let elapsed = shown.started.elapsed().as_millis() as u64 + shown.offset_ms;
        if shown.fade.finished(elapsed) {
            return None;
        }
        shown.level = shown.fade.level(elapsed);
        Some((shown.alpha as f32 * shown.level) as u8)
    });
    // SAFETY: timers and layered attributes of the window being processed.
    unsafe {
        match alpha {
            Some(alpha) => {
                SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA).ok();
            }
            None => {
                KillTimer(Some(hwnd), TIMER_FADE).ok();
                let _ = ShowWindow(hwnd, SW_HIDE);
                SHOWN.with(|s| {
                    if let Some(shown) = s.borrow_mut().as_mut() {
                        shown.level = 0.0;
                    }
                });
            }
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
