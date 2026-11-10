//! Bridge between the Rust iOS app and its Swift helpers (Bonjour + document picker).
//!
//! The only crate in the workspace that allows `unsafe_code`, because it declares C symbols and
//! parses foreign pointers. Every unsafe block is a direct FFI boundary with a null/UTF-8 check.
#![allow(unsafe_code)]

use std::ffi::CString;
use std::os::raw::c_char;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub enum PeerEvent {
    Found {
        device_id: String,
        alias: String,
        ip: String,
        port: u16,
    },
    Lost {
        device_id: String,
    },
}

type PeerHandler = Box<dyn Fn(PeerEvent) + Send + Sync>;
static PEER_HANDLER: OnceLock<PeerHandler> = OnceLock::new();

pub fn set_peer_handler(handler: impl Fn(PeerEvent) + Send + Sync + 'static) {
    let _ = PEER_HANDLER.set(Box::new(handler));
}

pub fn emit_peer(event: PeerEvent) {
    if let Some(handler) = PEER_HANDLER.get() {
        handler(event);
    }
}

static DESTINATION: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn destination_slot() -> &'static Mutex<Option<String>> {
    DESTINATION.get_or_init(|| Mutex::new(None))
}

pub fn set_destination(name: Option<String>) {
    if let Ok(mut slot) = destination_slot().lock() {
        *slot = name;
    }
}

pub fn destination() -> Option<String> {
    destination_slot().lock().ok().and_then(|slot| slot.clone())
}

pub fn pick_folder() {
    unsafe { droplink_ios_pick_folder() }
}

pub fn open_files() {
    unsafe { droplink_ios_open_files() }
}

pub fn export_received(path: &std::path::Path) {
    let Ok(path) = CString::new(path.to_string_lossy().as_bytes()) else {
        return;
    };
    unsafe { droplink_ios_export(path.as_ptr()) }
}

static PICKED_FILES: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();

fn picked_files() -> &'static Mutex<Vec<PathBuf>> {
    PICKED_FILES.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn emit_file_picked(path: PathBuf) {
    if let Ok(mut queue) = picked_files().lock() {
        queue.push(path);
    }
}

/// Drain the files chosen in the iOS document picker.
pub fn take_picked_files() -> Vec<PathBuf> {
    picked_files()
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default()
}

extern "C" {
    fn droplink_ios_start_bonjour(
        port: u16,
        device_id: *const c_char,
        alias: *const c_char,
        ip: *const c_char,
    );
    fn droplink_ios_stop_bonjour();
    fn droplink_ios_pick_file();
    fn droplink_ios_open_file(path: *const c_char);
    fn droplink_ios_pick_folder();
    fn droplink_ios_open_files();
    fn droplink_ios_export(path: *const c_char);
}

pub fn start_bonjour(port: u16, device_id: &str, alias: &str, ip: &str) {
    let Ok(device_id) = CString::new(device_id) else {
        return;
    };
    let Ok(alias) = CString::new(alias) else {
        return;
    };
    let Ok(ip) = CString::new(ip) else {
        return;
    };
    unsafe { droplink_ios_start_bonjour(port, device_id.as_ptr(), alias.as_ptr(), ip.as_ptr()) }
}

pub fn log(message: &str) {
    tracing::info!(target: "droplink_ios", "{message}");
}

pub fn stop_bonjour() {
    unsafe { droplink_ios_stop_bonjour() }
}

pub fn pick_file() {
    unsafe { droplink_ios_pick_file() }
}

/// Open a received file: the Swift side presents the system share/preview sheet.
pub fn open_file(path: &std::path::Path) {
    let Ok(path) = CString::new(path.to_string_lossy().as_bytes()) else {
        return;
    };
    unsafe { droplink_ios_open_file(path.as_ptr()) }
}
