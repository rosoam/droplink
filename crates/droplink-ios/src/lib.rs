//! iOS FFI shim: exports the C entry points called by the Swift application.
//!
//! `unsafe_code` is allowed only here and in `droplink-ios-bridge`: `#[unsafe(no_mangle)]` and
//! foreign pointer parsing are unsafe by construction. Every pointer is null-checked and copied
//! into owned Rust data before use.
#![allow(unsafe_code)]

#[cfg(target_os = "ios")]
mod imp {
    use std::ffi::CStr;
    use std::os::raw::c_char;
    use std::path::PathBuf;

    /// Entry point called from the iOS application's `main.m`.
    #[unsafe(no_mangle)]
    pub extern "C" fn droplink_ios_start() {
        droplink_app::run();
    }

    fn copy_string(ptr: *const c_char) -> Option<String> {
        if ptr.is_null() {
            return None;
        }
        let value = unsafe { CStr::from_ptr(ptr) };
        value.to_str().ok().map(str::to_string)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn droplink_ios_peer_found(
        device_id: *const c_char,
        alias: *const c_char,
        ip: *const c_char,
        port: u16,
    ) {
        let (Some(device_id), Some(alias), Some(ip)) =
            (copy_string(device_id), copy_string(alias), copy_string(ip))
        else {
            return;
        };
        if port == 0 {
            return;
        }
        droplink_ios_bridge::emit_peer(droplink_ios_bridge::PeerEvent::Found {
            device_id,
            alias,
            ip,
            port,
        });
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn droplink_ios_peer_lost(device_id: *const c_char) {
        let Some(device_id) = copy_string(device_id) else {
            return;
        };
        droplink_ios_bridge::emit_peer(droplink_ios_bridge::PeerEvent::Lost { device_id });
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn droplink_ios_file_picked(path: *const c_char) {
        let Some(path) = copy_string(path) else {
            return;
        };
        droplink_ios_bridge::emit_file_picked(PathBuf::from(path));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn droplink_ios_file_pick_cancelled() {}

    #[unsafe(no_mangle)]
    pub extern "C" fn droplink_ios_log(message: *const c_char) {
        let Some(message) = copy_string(message) else {
            return;
        };
        droplink_ios_bridge::log(&message);
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn droplink_ios_destination_changed(name: *const c_char) {
        let name = copy_string(name);
        droplink_ios_bridge::set_destination(name);
    }
}

#[cfg(target_os = "ios")]
pub use imp::*;
