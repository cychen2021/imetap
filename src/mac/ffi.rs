//! Raw bindings to the C APIs the macOS port uses: Text Input Sources (Carbon), Quartz event taps
//! and CoreFoundation run-loop plumbing.

#![allow(non_snake_case, non_upper_case_globals, non_camel_case_types)]

use std::ffi::{CStr, c_char, c_void};

use anyhow::{Result, bail};

pub type CFTypeRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CFArrayRef = *const c_void;
pub type CFIndex = isize;
pub type CFRunLoopRef = *const c_void;
pub type CFRunLoopSourceRef = *const c_void;
pub type CFRunLoopTimerRef = *const c_void;
pub type CFMachPortRef = *const c_void;
pub type CGEventRef = *mut c_void;
pub type CGEventTapProxy = *mut c_void;

pub type CGEventTapCallBack =
    extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;
pub type CFRunLoopTimerCallBack = extern "C" fn(CFRunLoopTimerRef, *mut c_void);
pub type CFNotificationCallback =
    extern "C" fn(*const c_void, *mut c_void, CFStringRef, *const c_void, *const c_void);

const kCFStringEncodingUTF8: u32 = 0x0800_0100;

#[allow(clippy::duplicated_attributes)]
#[link(name = "Carbon", kind = "framework")]
#[link(name = "ApplicationServices", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub static kTISPropertyInputSourceID: CFStringRef;
    pub static kCFRunLoopCommonModes: CFStringRef;
    pub static kAXTrustedCheckOptionPrompt: CFStringRef;
    pub static kCFBooleanTrue: CFTypeRef;

    fn TISCopyCurrentKeyboardInputSource() -> CFTypeRef;
    fn TISCreateInputSourceList(properties: CFTypeRef, include_all_installed: bool) -> CFArrayRef;
    fn TISGetInputSourceProperty(source: CFTypeRef, key: CFStringRef) -> CFTypeRef;
    fn TISSelectInputSource(source: CFTypeRef) -> i32;

    pub fn CFRelease(cf: CFTypeRef);
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

    pub fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;

    pub fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: CGEventTapCallBack,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    pub fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    pub fn CGEventGetFlags(event: CGEventRef) -> u64;
    pub fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    pub fn CGEventGetTimestamp(event: CGEventRef) -> u64;

    pub fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: CFMachPortRef,
        order: CFIndex,
    ) -> CFRunLoopSourceRef;
    pub fn CFRunLoopGetMain() -> CFRunLoopRef;
    pub fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    pub fn CFRunLoopAddTimer(rl: CFRunLoopRef, timer: CFRunLoopTimerRef, mode: CFStringRef);
    pub fn CFRunLoopTimerCreate(
        allocator: *const c_void,
        fire_date: f64,
        interval: f64,
        flags: u64,
        order: CFIndex,
        callout: CFRunLoopTimerCallBack,
        context: *mut c_void,
    ) -> CFRunLoopTimerRef;
    pub fn CFRunLoopTimerInvalidate(timer: CFRunLoopTimerRef);
    pub fn CFAbsoluteTimeGetCurrent() -> f64;

    pub fn CFNotificationCenterGetDistributedCenter() -> *const c_void;
    pub fn CFNotificationCenterAddObserver(
        center: *const c_void,
        observer: *const c_void,
        callback: CFNotificationCallback,
        name: CFStringRef,
        object: *const c_void,
        suspension_behavior: isize,
    );
    pub fn CFStringCreateWithCString(
        allocator: *const c_void,
        c_str: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
}

/// Event types and fields of Quartz events.
pub const LEFT_MOUSE_DOWN: u32 = 1;
pub const RIGHT_MOUSE_DOWN: u32 = 3;
pub const KEY_DOWN: u32 = 10;
pub const KEY_UP: u32 = 11;
pub const FLAGS_CHANGED: u32 = 12;
pub const OTHER_MOUSE_DOWN: u32 = 25;
pub const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
pub const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;
pub const FIELD_KEYCODE: u32 = 9;

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
    // SAFETY: `source` is a live TISInputSource; the property is owned by it ("get" rule).
    string_of(unsafe { TISGetInputSourceProperty(source, kTISPropertyInputSourceID) })
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

/// Selects the enabled input source with the given identifier.
pub fn select_source(wanted: &str) -> Result<()> {
    // SAFETY: the list follows the "create" rule and is released; sources stay valid while it lives.
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

/// Asks for the Accessibility permission, showing the system prompt if it is missing.
pub fn ensure_accessibility() -> bool {
    // SAFETY: builds a one-entry dictionary from static CF constants.
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
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

/// A CFString from a Rust string; the caller owns it.
pub fn cfstring(text: &CStr) -> CFStringRef {
    // SAFETY: `text` is NUL-terminated.
    unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), kCFStringEncodingUTF8) }
}
