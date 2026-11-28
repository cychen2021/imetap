//! The HUD: a translucent, click-through panel near the bottom of the screen with keyboard focus.

use std::time::Instant;

use objc2::rc::Retained;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSFont, NSPanel, NSScreen, NSTextAlignment, NSTextField, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString, ns_string};

use crate::config::HudConfig;
use crate::fade::Fade;
use crate::state::{Color, Rgb, Snapshot};

/// The HUD's size in points.
const HUD_WIDTH: f64 = 240.0;
/// Width of the stripe on the left that names the input method.
const HUD_STRIPE: f64 = 60.0;
const HUD_HEIGHT: f64 = 100.0;
const HUD_BOTTOM_MARGIN: f64 = 48.0;
/// `NSStatusWindowLevel`.
const STATUS_WINDOW_LEVEL: isize = 25;

/// How often the HUD's brightness is updated.
pub(super) const FADE_STEP_SECS: f64 = 1.0 / 60.0;

/// One appearance of the HUD, stepped by timers.
#[derive(Clone, Copy)]
struct HudRun {
    fade: Fade,
    started: Instant,
    /// How far into the curve it began (non-zero when it took over from one still visible).
    offset_ms: u64,
}

pub(super) struct Hud {
    panel: Retained<NSPanel>,
    glyph: Retained<NSTextField>,
    /// The stripe on the left, in the input method's colour.
    stripe: Retained<NSView>,
    /// The input method's label on the stripe.
    stripe_label: Retained<NSTextField>,
    /// The current appearance. `Some` while a chain of step timers runs.
    run: Option<HudRun>,
    /// The last brightness, from 0.0 to 1.0.
    level: f32,
}

impl Hud {
    pub(super) fn new(mtm: MainThreadMarker) -> Hud {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(HUD_WIDTH, HUD_HEIGHT));
        let panel = {
            NSPanel::initWithContentRect_styleMask_backing_defer(
                mtm.alloc::<NSPanel>(),
                frame,
                NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        panel.setLevel(STATUS_WINDOW_LEVEL);
        panel.setOpaque(false);
        panel.setHasShadow(false);
        panel.setIgnoresMouseEvents(true);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        let glyph = label(
            mtm,
            NSRect::new(
                NSPoint::new(HUD_STRIPE, (HUD_HEIGHT - 66.0) / 2.0),
                NSSize::new(HUD_WIDTH - HUD_STRIPE, 66.0),
            ),
            52.0,
            true,
            &NSColor::whiteColor(),
        );
        let stripe = NSView::new(mtm);
        stripe.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(HUD_STRIPE, HUD_HEIGHT),
        ));
        stripe.setWantsLayer(true);
        let stripe_label = label(
            mtm,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(HUD_STRIPE, 60.0)),
            40.0,
            true,
            &NSColor::blackColor(),
        );
        if let Some(view) = panel.contentView() {
            view.setWantsLayer(true);
            if let Some(layer) = view.layer() {
                layer.setCornerRadius(24.0);
                // Clip the stripe to the rounded corners.
                layer.setMasksToBounds(true);
                layer.setBackgroundColor(Some(&ns_color(Color::Black.fill()).CGColor()));
            }
            view.addSubview(&stripe);
            view.addSubview(&stripe_label);
            view.addSubview(&glyph);
        }
        Hud {
            panel,
            glyph,
            stripe,
            stripe_label,
            run: None,
            level: 0.0,
        }
    }

    /// Shows `snapshot`. Returns true when the caller must start the chain of `step` calls, which
    /// is when none runs yet.
    pub(super) fn show(
        &mut self,
        mtm: MainThreadMarker,
        config: &HudConfig,
        snapshot: Snapshot,
    ) -> bool {
        let style = config.style(snapshot);
        let colors = style.colors;
        if let Some(layer) = self.panel.contentView().and_then(|view| view.layer()) {
            layer.setBackgroundColor(Some(&ns_color(colors.background).CGColor()));
        }
        if let Some(layer) = self.stripe.layer() {
            layer.setBackgroundColor(Some(&ns_color(colors.stripe).CGColor()));
        }
        self.glyph.setTextColor(Some(&ns_color(colors.glyph)));
        self.glyph
            .setStringValue(&NSString::from_str(snapshot.glyph()));
        // A one-character label is bigger than a two-character one. The field is centred on the stripe.
        let size = if style.label.chars().count() > 1 {
            24.0
        } else {
            34.0
        };
        let height = size * 1.4;
        self.stripe_label
            .setFont(Some(&NSFont::boldSystemFontOfSize(size)));
        self.stripe_label.setFrame(NSRect::new(
            NSPoint::new(0.0, (HUD_HEIGHT - height) / 2.0),
            NSSize::new(HUD_STRIPE, height),
        ));
        self.stripe_label
            .setTextColor(Some(&ns_color(colors.stripe_text)));
        self.stripe_label
            .setStringValue(&NSString::from_str(&style.label));
        // The screen of the window with keyboard focus. The first (primary) screen if there is none.
        let screen = NSScreen::mainScreen(mtm).or_else(|| NSScreen::screens(mtm).firstObject());
        if let Some(screen) = screen {
            let area = screen.visibleFrame();
            let origin = NSPoint::new(
                area.origin.x + (area.size.width - HUD_WIDTH) / 2.0,
                area.origin.y + HUD_BOTTOM_MARGIN,
            );
            self.panel.setFrameOrigin(origin);
        }
        // Showing again while still visible carries on from the current brightness.
        let fade = config.fade();
        let offset_ms = if self.panel.isVisible() {
            fade.start_for(self.level)
        } else {
            0
        };
        self.level = fade.level(offset_ms);
        self.panel
            .setAlphaValue(config.opacity as f64 * self.level as f64);
        self.panel.orderFrontRegardless();
        let running = self.run.is_some();
        self.run = Some(HudRun {
            fade,
            started: Instant::now(),
            offset_ms,
        });
        !running
    }

    /// One step of the fade: sets the brightness. Returns true when another step is due, and false
    /// when the fade is over and the HUD is hidden.
    pub(super) fn step(&mut self, config: &HudConfig) -> bool {
        let Some(run) = self.run else { return false };
        let elapsed = run.started.elapsed().as_millis() as u64 + run.offset_ms;
        if run.fade.finished(elapsed) {
            self.panel.orderOut(None);
            self.level = 0.0;
            self.run = None;
            return false;
        }
        self.level = run.fade.level(elapsed);
        self.panel
            .setAlphaValue(config.opacity as f64 * self.level as f64);
        true
    }
}

fn label(
    mtm: MainThreadMarker,
    frame: NSRect,
    size: f64,
    bold: bool,
    color: &NSColor,
) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(ns_string!(""), mtm);
    field.setFrame(frame);
    field.setAlignment(NSTextAlignment::Center);
    field.setTextColor(Some(color));
    let font = if bold {
        NSFont::boldSystemFontOfSize(size)
    } else {
        NSFont::systemFontOfSize(size)
    };
    field.setFont(Some(&font));
    field
}

fn ns_color([r, g, b]: Rgb) -> Retained<NSColor> {
    let channel = |v: u8| v as f64 / 255.0;
    NSColor::colorWithSRGBRed_green_blue_alpha(channel(r), channel(g), channel(b), 1.0)
}
