//! Bindings to the C and Objective-C APIs the macOS port uses (Text Input Sources, Quartz event
//! taps, CoreFoundation run loops, libdispatch and the AppKit calls that `objc2` marks unsafe), and
//! safe wrappers around them. All the unsafe code of the macOS port is in this file.
//!
//! The C callbacks are defined here. They call plain Rust functions, which travel through the
//! user-data pointer that each C API hands back to its callback.

#![allow(non_snake_case, non_upper_case_globals, non_camel_case_types)]

use std::ffi::{CStr, c_char, c_void};
use std::mem;
use std::ptr::{self, NonNull};

use anyhow::{Result, bail};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSMenuItem,
    NSStringDrawing, NSWorkspace, NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::{
    MainThreadMarker, NSAttributedStringKey, NSDictionary, NSNotification, NSObject,
    NSObjectProtocol, NSPoint, NSString, ns_string,
};

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFArrayRef = *const c_void;
type CFIndex = isize;
type CFRunLoopRef = *const c_void;
type CFRunLoopSourceRef = *const c_void;
type CFRunLoopTimerRef = *const c_void;
type CFMachPortRef = *const c_void;
type CGEventRef = *mut c_void;
type CGEventTapProxy = *mut c_void;

type CGEventTapCallBack =
    extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;
type CFRunLoopTimerCallBack = extern "C" fn(CFRunLoopTimerRef, *mut c_void);
type CFNotificationCallback =
    extern "C" fn(*const c_void, *mut c_void, CFStringRef, *const c_void, *const c_void);

const kCFStringEncodingUTF8: u32 = 0x0800_0100;

/// What `CFRunLoopTimerCreate` copies from its context argument.
#[repr(C)]
struct CFRunLoopTimerContext {
    version: CFIndex,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
}

#[allow(clippy::duplicated_attributes)]
#[link(name = "Carbon", kind = "framework")]
#[link(name = "ApplicationServices", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kTISPropertyInputSourceID: CFStringRef;
    static kTISPropertyLocalizedName: CFStringRef;
    static kCFRunLoopCommonModes: CFStringRef;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    static kCFBooleanTrue: CFTypeRef;
    static kCFBooleanFalse: CFTypeRef;

    fn TISCopyCurrentKeyboardInputSource() -> CFTypeRef;
    fn TISCreateInputSourceList(properties: CFTypeRef, include_all_installed: bool) -> CFArrayRef;
    fn TISGetInputSourceProperty(source: CFTypeRef, key: CFStringRef) -> CFTypeRef;
    fn TISSelectInputSource(source: CFTypeRef) -> i32;

    fn CFRelease(cf: CFTypeRef);
    fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, index: CFIndex) -> CFTypeRef;
    fn CFStringGetCString(
        s: CFStringRef,
        buffer: *mut c_char,
        size: CFIndex,
        encoding: u32,
    ) -> bool;
    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        count: CFIndex,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFTypeRef;

    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;

    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: CGEventTapCallBack,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventGetTimestamp(event: CGEventRef) -> u64;

    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: CFMachPortRef,
        order: CFIndex,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetMain() -> CFRunLoopRef;
    /// The main queue of libdispatch. It runs on the main thread, in the main run loop.
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(
        queue: *const c_void,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopAddTimer(rl: CFRunLoopRef, timer: CFRunLoopTimerRef, mode: CFStringRef);
    fn CFRunLoopTimerCreate(
        allocator: *const c_void,
        fire_date: f64,
        interval: f64,
        flags: u64,
        order: CFIndex,
        callout: CFRunLoopTimerCallBack,
        context: *mut CFRunLoopTimerContext,
    ) -> CFRunLoopTimerRef;
    fn CFRunLoopTimerInvalidate(timer: CFRunLoopTimerRef);
    fn CFAbsoluteTimeGetCurrent() -> f64;

    fn CFNotificationCenterGetDistributedCenter() -> *const c_void;
    fn CFNotificationCenterAddObserver(
        center: *const c_void,
        observer: *const c_void,
        callback: CFNotificationCallback,
        name: CFStringRef,
        object: *const c_void,
        suspension_behavior: isize,
    );
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        c_str: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
}

/// Event types and fields of Quartz events.
pub const LEFT_MOUSE_DOWN: u32 = 1;
pub const RIGHT_MOUSE_DOWN: u32 = 3;
pub const MOUSE_MOVED: u32 = 5;
pub const LEFT_MOUSE_DRAGGED: u32 = 6;
pub const RIGHT_MOUSE_DRAGGED: u32 = 7;
pub const KEY_DOWN: u32 = 10;
pub const KEY_UP: u32 = 11;
pub const FLAGS_CHANGED: u32 = 12;
pub const SCROLL_WHEEL: u32 = 22;
pub const OTHER_MOUSE_DOWN: u32 = 25;
pub const OTHER_MOUSE_DRAGGED: u32 = 27;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;
const FIELD_KEYCODE: u32 = 9;

pub const FLAG_SHIFT: u64 = 0x0002_0000;
pub const FLAG_CONTROL: u64 = 0x0004_0000;
pub const FLAG_ALT: u64 = 0x0008_0000;
pub const FLAG_COMMAND: u64 = 0x0010_0000;

fn string_of(s: CFStringRef) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let mut buf = [0 as c_char; 256];
    // SAFETY: `buf` is writable for its length and `s` is a live CFString.
    let ok = unsafe {
        CFStringGetCString(
            s,
            buf.as_mut_ptr(),
            buf.len() as CFIndex,
            kCFStringEncodingUTF8,
        )
    };
    // SAFETY: on success the buffer holds a NUL-terminated string.
    ok.then(|| {
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    })
}

fn id_of(source: CFTypeRef) -> Option<String> {
    // SAFETY: `source` is a live TISInputSource. The property is owned by it ("get" rule).
    string_of(unsafe { TISGetInputSourceProperty(source, kTISPropertyInputSourceID) })
}

/// The localized name of the current keyboard input source, as the input menu shows it.
pub fn current_source_name() -> Option<String> {
    // SAFETY: "copy" rule: the source is released after reading. The name belongs to it ("get" rule).
    unsafe {
        let source = TISCopyCurrentKeyboardInputSource();
        if source.is_null() {
            return None;
        }
        let name = string_of(TISGetInputSourceProperty(source, kTISPropertyLocalizedName));
        CFRelease(source);
        name
    }
}

/// The identifier of the current keyboard input source.
pub fn current_source() -> Option<String> {
    // SAFETY: "copy" rule: the source is released after reading.
    unsafe {
        let source = TISCopyCurrentKeyboardInputSource();
        if source.is_null() {
            return None;
        }
        let id = id_of(source);
        CFRelease(source);
        id
    }
}

/// The identifiers of the enabled input sources, those in the input menu.
pub fn enabled_sources() -> Vec<String> {
    // SAFETY: the list follows the "create" rule and is released. Sources stay valid while it lives.
    unsafe {
        let list = TISCreateInputSourceList(std::ptr::null(), false);
        if list.is_null() {
            return Vec::new();
        }
        let ids = (0..CFArrayGetCount(list))
            .filter_map(|i| id_of(CFArrayGetValueAtIndex(list, i)))
            .collect();
        CFRelease(list);
        ids
    }
}

/// Selects the enabled input source with the given identifier.
pub fn select_source(wanted: &str) -> Result<()> {
    // SAFETY: the list follows the "create" rule and is released. Sources stay valid while it lives.
    unsafe {
        let list = TISCreateInputSourceList(std::ptr::null(), false);
        if list.is_null() {
            bail!("cannot list the input sources");
        }
        let mut status = None;
        for i in 0..CFArrayGetCount(list) {
            let source = CFArrayGetValueAtIndex(list, i);
            if id_of(source).as_deref() == Some(wanted) {
                status = Some(TISSelectInputSource(source));
                break;
            }
        }
        CFRelease(list);
        match status {
            Some(0) => Ok(()),
            Some(code) => bail!("selecting {wanted} failed (OSStatus {code})"),
            None => bail!("{wanted} is not an enabled input source"),
        }
    }
}

/// Whether the process has the Accessibility permission. With `prompt`, shows the system prompt
/// if it is missing.
pub fn ensure_accessibility(prompt: bool) -> bool {
    // SAFETY: builds a one-entry dictionary from static CF constants.
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [if prompt {
            kCFBooleanTrue
        } else {
            kCFBooleanFalse
        }];
        let options = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            std::ptr::null(),
            std::ptr::null(),
        );
        let trusted = AXIsProcessTrustedWithOptions(options);
        if !options.is_null() {
            CFRelease(options);
        }
        trusted
    }
}

/// A CFString from a Rust string. The caller owns it.
fn cfstring(text: &CStr) -> CFStringRef {
    // SAFETY: `text` is NUL-terminated.
    unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), kCFStringEncodingUTF8) }
}

// ---- event taps --------------------------------------------------------------------------------

/// What a tap's handler learns about an event.
pub struct TapEvent {
    /// The event type, or `TapEvent::DISABLED` when the system switched the tap off.
    pub kind: u32,
    pub flags: u64,
    pub keycode: i64,
    /// The event's own timestamp, in milliseconds.
    pub time_ms: u64,
}

impl TapEvent {
    /// The `kind` of the notice that the system switched the tap off, because it was slow or
    /// because of user input. The handler should switch it back on with `EventTap::enable`.
    pub const DISABLED: u32 = TAP_DISABLED_BY_TIMEOUT;
}

/// Handles an event of a tap. Returns false to swallow the event.
pub type TapHandler = fn(&TapEvent) -> bool;

/// A Quartz event tap made by `create_event_tap`.
#[derive(Clone, Copy)]
pub struct EventTap(CFMachPortRef);

extern "C" fn tap_fired(
    _: CGEventTapProxy,
    kind: u32,
    event: CGEventRef,
    handler: *mut c_void,
) -> CGEventRef {
    // SAFETY: `handler` is the `TapHandler` that `create_event_tap` passed as the user data.
    let handler = unsafe { mem::transmute::<*mut c_void, TapHandler>(handler) };
    let disabled = matches!(kind, TAP_DISABLED_BY_TIMEOUT | TAP_DISABLED_BY_USER_INPUT);
    let (flags, keycode, time_ms) = if disabled || event.is_null() {
        (0, 0, 0)
    } else {
        // SAFETY: `event` is valid for the duration of the callback.
        unsafe {
            (
                CGEventGetFlags(event),
                CGEventGetIntegerValueField(event, FIELD_KEYCODE),
                CGEventGetTimestamp(event) / 1_000_000,
            )
        }
    };
    let kind = if disabled { TapEvent::DISABLED } else { kind };
    let keep = handler(&TapEvent {
        kind,
        flags,
        keycode,
        time_ms,
    });
    if keep { event } else { ptr::null_mut() }
}

/// Creates an event tap for the given event types in the login session. `at_tail` places it after
/// the other taps, and a `listen_only` tap cannot swallow events. `None` when the process lacks
/// the Accessibility permission.
pub fn create_event_tap(
    at_tail: bool,
    listen_only: bool,
    events: &[u32],
    handler: TapHandler,
) -> Option<EventTap> {
    let mask = events.iter().fold(0u64, |acc, t| acc | 1 << t);
    // SAFETY: the callback is a plain `extern "C"` function, and the user data is a function
    // pointer, which lives for the whole program.
    let port = unsafe {
        CGEventTapCreate(
            0, /* session */
            u32::from(at_tail),
            u32::from(listen_only),
            mask,
            tap_fired,
            handler as *mut c_void,
        )
    };
    (!port.is_null()).then_some(EventTap(port))
}

impl EventTap {
    /// Attaches the tap to the main run loop and switches it on.
    pub fn attach(self) {
        // SAFETY: `self.0` is a live event tap, never released.
        unsafe {
            let source = CFMachPortCreateRunLoopSource(ptr::null(), self.0, 0);
            CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
            CGEventTapEnable(self.0, true);
        }
    }

    /// Switches the tap back on after the system switched it off.
    pub fn enable(self) {
        // SAFETY: `self.0` is a live event tap, never released.
        unsafe { CGEventTapEnable(self.0, true) };
    }
}

// ---- run loop and main queue -------------------------------------------------------------------

/// A one-shot timer on the main run loop, made by `schedule`.
#[derive(Clone, Copy)]
pub struct Timer(CFRunLoopTimerRef);

impl Timer {
    /// Stops the timer. Harmless if it has fired already.
    pub fn cancel(self) {
        // SAFETY: `schedule` never releases its timers, so `self.0` is still alive.
        unsafe { CFRunLoopTimerInvalidate(self.0) };
    }
}

extern "C" fn timer_fired(_: CFRunLoopTimerRef, handler: *mut c_void) {
    // SAFETY: `handler` is the `fn()` that `add_timer` put in the timer's context.
    let handler = unsafe { mem::transmute::<*mut c_void, fn()>(handler) };
    handler();
}

/// Adds a one-shot timer that calls `handler` after `seconds` to the main run loop. The caller
/// owns the returned reference.
fn add_timer(seconds: f64, handler: fn()) -> CFRunLoopTimerRef {
    let mut context = CFRunLoopTimerContext {
        version: 0,
        info: handler as *mut c_void,
        retain: ptr::null(),
        release: ptr::null(),
        copy_description: ptr::null(),
    };
    // SAFETY: creates a one-shot timer on the main run loop. CF copies the context, and its
    // `info` is a function pointer, which lives for the whole program.
    unsafe {
        let timer = CFRunLoopTimerCreate(
            ptr::null(),
            CFAbsoluteTimeGetCurrent() + seconds,
            0.0,
            0,
            0,
            timer_fired,
            &mut context,
        );
        CFRunLoopAddTimer(CFRunLoopGetMain(), timer, kCFRunLoopCommonModes);
        timer
    }
}

/// Calls `handler` once after `seconds` on the main run loop. The timer can be cancelled.
pub fn schedule(seconds: f64, handler: fn()) -> Timer {
    Timer(add_timer(seconds, handler))
}

/// Calls `handler` once after `seconds` on the main run loop, with no way to cancel it.
pub fn schedule_once(seconds: f64, handler: fn()) {
    let timer = add_timer(seconds, handler);
    // SAFETY: the run loop holds its own reference until the timer has fired, so ours can go.
    unsafe { CFRelease(timer) };
}

/// Runs `job` on the main thread, after the current callback returns. Callable from any thread.
pub fn run_on_main(job: impl FnOnce() + Send + 'static) {
    type Job = Box<dyn FnOnce() + Send>;
    extern "C" fn run(context: *mut c_void) {
        // SAFETY: `context` is the box made below, and the queue runs this once.
        let job = unsafe { Box::from_raw(context.cast::<Job>()) };
        job();
    }
    let job: Job = Box::new(job);
    let context = Box::into_raw(Box::new(job)).cast::<c_void>();
    // SAFETY: the main queue takes the box and gives it back to `run`.
    unsafe { dispatch_async_f(&raw const _dispatch_main_q, context, run) };
}

// ---- notifications -----------------------------------------------------------------------------

extern "C" fn notified(
    _: *const c_void,
    handler: *mut c_void,
    _: CFStringRef,
    _: *const c_void,
    _: *const c_void,
) {
    // SAFETY: `handler` is the `fn()` that `on_input_source_change` registered as the observer.
    let handler = unsafe { mem::transmute::<*mut c_void, fn()>(handler) };
    handler();
}

/// Calls `handler` whenever the selected keyboard input source changes, for the whole process.
pub fn on_input_source_change(handler: fn()) {
    let name = cfstring(c"com.apple.Carbon.TISNotifySelectedKeyboardInputSourceChanged");
    // SAFETY: registers a C callback with the distributed notification center. The observer is a
    // function pointer, which lives for the whole program. 4 is "deliver immediately".
    unsafe {
        CFNotificationCenterAddObserver(
            CFNotificationCenterGetDistributedCenter(),
            handler as *const c_void,
            notified,
            name,
            ptr::null(),
            4,
        );
    }
}

/// Calls `handler` whenever another application comes to the front, for the whole process.
pub fn on_app_activation(handler: fn()) {
    let block = RcBlock::new(move |_: NonNull<NSNotification>| handler());
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: registers a block with the workspace's notification center, which copies it. The
    // observer is kept for the whole process.
    let observer = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidActivateApplicationNotification),
            None,
            None,
            &block,
        )
    };
    mem::forget(observer);
}

// ---- AppKit ------------------------------------------------------------------------------------

define_class!(
    /// The target of a menu item made by `menu_item`: calls the item's handler.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ImetapMenuTarget"]
    #[ivars = fn(MainThreadMarker)]
    struct MenuTarget;

    unsafe impl NSObjectProtocol for MenuTarget {}

    impl MenuTarget {
        #[unsafe(method(perform:))]
        fn perform(&self, _sender: Option<&AnyObject>) {
            (self.ivars())(self.mtm());
        }
    }
);

/// A menu item titled `title` that calls `handler` when chosen.
pub fn menu_item(
    mtm: MainThreadMarker,
    title: &str,
    handler: fn(MainThreadMarker),
) -> Retained<NSMenuItem> {
    let target = mtm.alloc::<MenuTarget>().set_ivars(handler);
    // SAFETY: `init` is the initializer of NSObject, the superclass, and the ivars are set.
    let target: Retained<MenuTarget> = unsafe { msg_send![super(target), init] };
    // SAFETY: `perform:` is a method of the target that takes the sender.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc::<NSMenuItem>(),
            &NSString::from_str(title),
            Some(sel!(perform:)),
            ns_string!(""),
        )
    };
    // SAFETY: the target is kept alive below.
    unsafe { item.setTarget(Some(&target)) };
    // The menu item holds only a weak reference to its target.
    mem::forget(target);
    item
}

/// Draws `text` in `font` and `color`, centred on `center`, in the current graphics context.
pub fn draw_text(text: &NSString, center: NSPoint, font: &NSFont, color: &NSColor) {
    let (font, color): (&AnyObject, &AnyObject) = (font, color);
    // SAFETY: the attribute keys are AppKit constants and the values have the types they expect.
    let attributes: Retained<NSDictionary<NSAttributedStringKey, AnyObject>> = unsafe {
        NSDictionary::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[font, color],
        )
    };
    // SAFETY: drawing into the current graphics context, which the caller's AppKit drawing set up.
    unsafe {
        let size = text.sizeWithAttributes(Some(&attributes));
        let origin = NSPoint::new(center.x - size.width / 2.0, center.y - size.height / 2.0);
        text.drawAtPoint_withAttributes(origin, Some(&attributes));
    }
}
