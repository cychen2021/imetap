//! The status item in the menu bar, which shows the mode, and its menu.

use anyhow::{Result, anyhow};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2::{AnyThread, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSBezierPath, NSColor, NSCompositingOperation, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSGraphicsContext, NSImage, NSMenu, NSMenuItem, NSStatusBar,
    NSStatusItem, NSStringDrawing, NSTextField, NSView,
};
use objc2_foundation::{
    MainThreadMarker, NSAttributedStringKey, NSData, NSDictionary, NSObject, NSObjectProtocol,
    NSPoint, NSRect, NSSize, NSString, ns_string,
};

use super::{autostart_enabled, open, set_autostart};
use crate::config;
use crate::debug;

/// Left inset of the menu header, lined up with the text of the other menu items.
const HEADER_INSET: f64 = 14.0;

define_class!(
    /// Target of the status item's menu actions.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ImetapMenuTarget"]
    struct MenuTarget;

    unsafe impl NSObjectProtocol for MenuTarget {}

    impl MenuTarget {
        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }

        #[unsafe(method(toggleLogin:))]
        fn toggle_login(&self, _sender: Option<&AnyObject>) {
            if let Err(e) = set_autostart(!autostart_enabled(), debug::enabled()) {
                eprintln!("imetap: cannot change run at login: {e:#}");
            }
        }

        #[unsafe(method(openConfig:))]
        fn open_config(&self, _sender: Option<&AnyObject>) {
            if let Err(e) = open_config_file() {
                eprintln!("imetap: cannot open the config file: {e:#}");
            }
        }
    }
);

/// Opens `config.toml` with the default application, creating a commented template first if needed.
fn open_config_file() -> Result<()> {
    let path = config::config_path().ok_or_else(|| anyhow!("no configuration directory"))?;
    if !path.exists() {
        std::fs::create_dir_all(
            path.parent()
                .ok_or_else(|| anyhow!("no parent directory"))?,
        )?;
        std::fs::write(&path, config::DEFAULT_TOML)?;
    }
    open(&path)
}

/// Shows "E" in the menu bar icon while `english`, and "中" otherwise.
pub(super) fn show_lang(item: &NSStatusItem, mtm: MainThreadMarker, english: bool) {
    if let Some(button) = item.button(mtm) {
        button.setImage(Some(&status_image(english)));
    }
}

/// The menu bar icon for a language: "中" or "E" knocked out of a rounded square, like the Windows
/// tray icon. It is a template image, so macOS tints it for light and dark menu bars. It shows
/// only Chinese or English: the system input menu shows the input method, but not the mode of an
/// input method such as Mingkuai.
fn status_image(english: bool) -> Retained<NSImage> {
    const SIZE: f64 = 18.0;
    let draw = RcBlock::new(move |_: NSRect| -> Bool {
        let square = NSRect::new(NSPoint::new(1.0, 1.0), NSSize::new(SIZE - 2.0, SIZE - 2.0));
        NSColor::blackColor().setFill();
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(square, 4.0, 4.0).fill();
        let text = if english {
            ns_string!("E")
        } else {
            ns_string!("中")
        };
        knock_out(text, NSPoint::new(SIZE / 2.0, SIZE / 2.0), 12.5);
        Bool::YES
    });
    let image =
        NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(SIZE, SIZE), false, &draw);
    image.setTemplate(true);
    image
}

/// The menu bar icon for `tray.enable_lang_indicator = false`: the logo drawn for the menu bar
/// (`assets/mac/menubar.svg`, an outlined pill), as a template image at 2x.
fn logo_image() -> Retained<NSImage> {
    const LOGO: &[u8] = include_bytes!("../../assets/mac/menubar.png");
    let data = NSData::with_bytes(LOGO);
    let image = NSImage::initWithData(NSImage::alloc(), &data)
        .expect("the embedded menu bar logo is a valid PNG");
    image.setSize(NSSize::new(26.0, 18.0));
    image.setTemplate(true);
    image
}

/// Cuts `text`, centred on `center`, out of what has been drawn in the current image.
fn knock_out(text: &NSString, center: NSPoint, font_size: f64) {
    if let Some(context) = NSGraphicsContext::currentContext() {
        context.setCompositingOperation(NSCompositingOperation::DestinationOut);
    }
    let font = NSFont::boldSystemFontOfSize(font_size);
    let color = NSColor::blackColor();
    // SAFETY: the attribute keys are AppKit constants and the values have the types they expect.
    let attributes: Retained<NSDictionary<NSAttributedStringKey, AnyObject>> = unsafe {
        NSDictionary::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[&**font as &AnyObject, &**color as &AnyObject],
        )
    };
    // SAFETY: drawing into the image's current graphics context, which AppKit set up.
    unsafe {
        let size = text.sizeWithAttributes(Some(&attributes));
        let origin = NSPoint::new(center.x - size.width / 2.0, center.y - size.height / 2.0);
        text.drawAtPoint_withAttributes(origin, Some(&attributes));
    }
}

/// Puts the status item in the menu bar, with the mode icon (`lang_indicator`) or the logo.
pub(super) fn make_status_item(
    mtm: MainThreadMarker,
    lang_indicator: bool,
) -> Retained<NSStatusItem> {
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0);
    if let Some(button) = item.button(mtm) {
        let image = if lang_indicator {
            status_image(false)
        } else {
            logo_image()
        };
        button.setImage(Some(&image));
        button.setToolTip(Some(ns_string!("imetap")));
    }
    let target: Retained<MenuTarget> = unsafe { msg_send![mtm.alloc::<MenuTarget>(), init] };
    let menu = NSMenu::new(mtm);
    // A header that names the tool. AppKit greys out a menu item that has no action whatever its
    // title says, so the header is a plain label shown as the item's view, which also keeps it from
    // highlighting.
    let header = NSMenuItem::new(mtm);
    let name = NSTextField::labelWithString(ns_string!("imetap"), mtm);
    name.setFont(Some(
        &NSFont::boldSystemFontOfSize(NSFont::systemFontSize()),
    ));
    name.setTextColor(Some(&NSColor::labelColor()));
    name.sizeToFit();
    let size = name.frame().size;
    name.setFrameOrigin(NSPoint::new(HEADER_INSET, 3.0));
    let view = NSView::initWithFrame(
        mtm.alloc::<NSView>(),
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(size.width + 2.0 * HEADER_INSET, size.height + 6.0),
        ),
    );
    view.addSubview(&name);
    header.setView(Some(&view));
    menu.addItem(&header);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    for (title, action) in [
        ("Run at login", sel!(toggleLogin:)),
        ("Open config file", sel!(openConfig:)),
        ("Quit imetap", sel!(quit:)),
    ] {
        let entry = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc::<NSMenuItem>(),
                &NSString::from_str(title),
                Some(action),
                ns_string!(""),
            )
        };
        unsafe { entry.setTarget(Some(&target)) };
        menu.addItem(&entry);
    }
    item.setMenu(Some(&menu));
    // The menu items hold only weak references to their target.
    std::mem::forget(target);
    item
}
