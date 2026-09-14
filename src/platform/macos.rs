//! Finder / "Open With" delivery on macOS.
//!
//! Launch Services does not put the file on `argv`. It sends `kAEOpenDocuments`.
//! winit 0.30 owns `NSApplicationDelegate` and does not forward that event, so
//! we:
//!
//! * Observe `NSApplicationWillFinishLaunchingNotification` and inject
//!   `application:openURLs:` onto winit's delegate class (cold launch).
//! * Install a Carbon `kAEOpenDocuments` handler after the app is ready
//!   (warm open while Savage is already running).
//!
//! Paths are forwarded through `EventLoopProxy<UserEvent>`.

use std::ffi::{c_char, c_void, CStr};
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::{Mutex, Once, OnceLock};

use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

#[repr(C)]
struct AEDesc {
    descriptor_type: u32,
    data_handle: *mut c_void,
}

impl AEDesc {
    const fn null() -> Self {
        Self {
            descriptor_type: 0,
            data_handle: std::ptr::null_mut(),
        }
    }
}

type AEEventHandlerProc =
    extern "C" fn(event: *const AEDesc, reply: *mut AEDesc, refcon: *mut c_void) -> i16;

type CFNotificationCallback = extern "C" fn(
    center: *mut c_void,
    observer: *mut c_void,
    name: *const c_void,
    object: *const c_void,
    user_info: *const c_void,
);

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn AEInstallEventHandler(
        event_class: u32,
        event_id: u32,
        handler: AEEventHandlerProc,
        refcon: *mut c_void,
        is_sys_handler: u8,
    ) -> i16;

    fn AEGetParamDesc(
        apple_event: *const AEDesc,
        keyword: u32,
        desired_type: u32,
        result: *mut AEDesc,
    ) -> i16;

    fn AECountItems(list: *const AEDesc, count: *mut std::ffi::c_long) -> i16;

    fn AEGetNthPtr(
        list: *const AEDesc,
        index: std::ffi::c_long,
        desired_type: u32,
        keyword: *mut u32,
        type_code: *mut u32,
        data_ptr: *mut c_void,
        maximum_size: std::ffi::c_long,
        actual_size: *mut std::ffi::c_long,
    ) -> i16;

    fn AEDisposeDesc(desc: *mut AEDesc) -> i16;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFNotificationCenterGetLocalCenter() -> *mut c_void;
    fn CFNotificationCenterAddObserver(
        center: *mut c_void,
        observer: *const c_void,
        callback: CFNotificationCallback,
        name: *const c_void,
        object: *const c_void,
        suspension_behavior: isize,
    );
    fn CFStringCreateWithCString(
        alloc: *const c_void,
        c_str: *const c_char,
        encoding: u32,
    ) -> *const c_void;
}

#[link(name = "objc")]
extern "C" {
    fn objc_msgSend();
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn object_getClass(obj: *mut c_void) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *const c_void;
    fn class_addMethod(
        cls: *mut c_void,
        name: *const c_void,
        imp: *mut c_void,
        types: *const c_char,
    ) -> u8;
}

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const CF_NOTIFICATION_DELIVER_IMMEDIATELY: isize = 4;

const fn fourcc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

fn queue() -> &'static Mutex<Vec<PathBuf>> {
    static QUEUE: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
    QUEUE.get_or_init(|| Mutex::new(Vec::new()))
}

fn proxy_slot() -> &'static Mutex<Option<EventLoopProxy<UserEvent>>> {
    static PROXY: OnceLock<Mutex<Option<EventLoopProxy<UserEvent>>>> = OnceLock::new();
    PROXY.get_or_init(|| Mutex::new(None))
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

fn deliver(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let proxy = lock(proxy_slot());
    if let Some(proxy) = proxy.as_ref() {
        for path in paths {
            let _ = proxy.send_event(UserEvent::Open(path));
        }
    } else {
        lock(queue()).extend(paths);
    }
}

/// Call before `EventLoop` exists so the launch notification is observed.
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let name = CFStringCreateWithCString(
            std::ptr::null(),
            c"NSApplicationWillFinishLaunchingNotification".as_ptr(),
            K_CF_STRING_ENCODING_UTF8,
        );
        if name.is_null() {
            eprintln!("savage: failed to register macOS open-document observer");
            return;
        }
        CFNotificationCenterAddObserver(
            CFNotificationCenterGetLocalCenter(),
            std::ptr::null(),
            on_will_finish_launching,
            name,
            std::ptr::null(),
            CF_NOTIFICATION_DELIVER_IMMEDIATELY,
        );
    });
}

/// Flush any files that arrived before the event loop, then keep the proxy.
pub fn bind_proxy(proxy: EventLoopProxy<UserEvent>) {
    let pending = std::mem::take(&mut *lock(queue()));
    for path in pending {
        let _ = proxy.send_event(UserEvent::Open(path));
    }
    *lock(proxy_slot()) = Some(proxy);
}

/// Call from `resumed` so warm "Open With" events reach us after AppKit launch.
pub fn on_ready() {
    static ONCE: Once = Once::new();
    ONCE.call_once(install_ae_handler);
}

extern "C" fn on_will_finish_launching(
    _center: *mut c_void,
    _observer: *mut c_void,
    _name: *const c_void,
    _object: *const c_void,
    _user_info: *const c_void,
) {
    unsafe { inject_open_urls_method() };
}

unsafe fn msg_obj(obj: *mut c_void, sel: *const c_void) -> *mut c_void {
    let send: unsafe extern "C" fn(*mut c_void, *const c_void) -> *mut c_void =
        unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    send(obj, sel)
}

unsafe fn inject_open_urls_method() {
    let app_cls = objc_getClass(c"NSApplication".as_ptr());
    if app_cls.is_null() {
        return;
    }
    let app = msg_obj(app_cls, sel_registerName(c"sharedApplication".as_ptr()));
    let delegate = if app.is_null() {
        std::ptr::null_mut()
    } else {
        msg_obj(app, sel_registerName(c"delegate".as_ptr()))
    };
    if delegate.is_null() {
        return;
    }
    let cls = object_getClass(delegate);
    let _ = class_addMethod(
        cls,
        sel_registerName(c"application:openURLs:".as_ptr()),
        handle_open_urls as *mut c_void,
        c"v@:@@".as_ptr(),
    );
}

extern "C" fn handle_open_urls(
    _this: *mut c_void,
    _cmd: *const c_void,
    _app: *mut c_void,
    urls: *mut c_void,
) {
    deliver(unsafe { ns_urls_to_paths(urls) });
}

unsafe fn ns_urls_to_paths(urls: *mut c_void) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if urls.is_null() {
        return out;
    }

    let count: unsafe extern "C" fn(*mut c_void, *const c_void) -> usize =
        unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    let at_index: unsafe extern "C" fn(*mut c_void, *const c_void, usize) -> *mut c_void =
        unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    let fs_repr: unsafe extern "C" fn(*mut c_void, *const c_void) -> *const c_char =
        unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };

    let n = count(urls, sel_registerName(c"count".as_ptr()));
    for i in 0..n {
        let url = at_index(urls, sel_registerName(c"objectAtIndex:".as_ptr()), i);
        if url.is_null() {
            continue;
        }
        let repr = fs_repr(url, sel_registerName(c"fileSystemRepresentation".as_ptr()));
        if repr.is_null() {
            continue;
        }
        let bytes = CStr::from_ptr(repr).to_bytes().to_vec();
        out.push(PathBuf::from(std::ffi::OsString::from_vec(bytes)));
    }
    out
}

fn install_ae_handler() {
    let err = unsafe {
        AEInstallEventHandler(
            fourcc(b"aevt"),
            fourcc(b"odoc"),
            handle_open_documents,
            std::ptr::null_mut(),
            0,
        )
    };
    if err != 0 {
        eprintln!("savage: failed to install macOS open-document handler ({err})");
    }
}

extern "C" fn handle_open_documents(
    event: *const AEDesc,
    _reply: *mut AEDesc,
    _refcon: *mut c_void,
) -> i16 {
    deliver(unsafe { extract_paths(event) });
    0
}

unsafe fn extract_paths(event: *const AEDesc) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if event.is_null() {
        return out;
    }

    let mut list = AEDesc::null();
    if AEGetParamDesc(event, fourcc(b"----"), fourcc(b"list"), &mut list) != 0 {
        return out;
    }

    let mut count: std::ffi::c_long = 0;
    if AECountItems(&list, &mut count) == 0 {
        for i in 1..=count {
            let mut buf = [0u8; 4096];
            let mut keyword = 0u32;
            let mut type_code = 0u32;
            let mut actual: std::ffi::c_long = 0;
            let err = AEGetNthPtr(
                &list,
                i,
                fourcc(b"furl"),
                &mut keyword,
                &mut type_code,
                buf.as_mut_ptr().cast(),
                buf.len() as std::ffi::c_long,
                &mut actual,
            );
            if err == 0 && actual > 0 && (actual as usize) <= buf.len() {
                if let Some(path) = file_url_to_path(&buf[..actual as usize]) {
                    out.push(path);
                }
            }
        }
    }

    AEDisposeDesc(&mut list);
    out
}

fn file_url_to_path(bytes: &[u8]) -> Option<PathBuf> {
    let s = std::str::from_utf8(bytes).ok()?;
    let rest = s.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    Some(PathBuf::from(std::ffi::OsString::from_vec(percent_decode(
        path.as_bytes(),
    ))))
}

fn percent_decode(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_file_url() {
        let path = file_url_to_path(b"file:///Users/name/My%20Icon.svg").unwrap();
        assert_eq!(path, PathBuf::from("/Users/name/My Icon.svg"));
    }
}
