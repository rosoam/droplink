#![allow(dead_code)]
// consumed in Task 6 (scanner) and Task 7 (main integration)

use serde::{Deserialize, Serialize};
use uuid::{uuid, Uuid};

/// UUID fixe identifiant le service BLE DropLink sur tous les appareils
pub const DROPLINK_SERVICE_UUID: Uuid = uuid!("A7B8C9D0-E1F2-3456-789A-BCDEF0123456");
/// Characteristic contenant {name, ip, port} en JSON
pub const DROPLINK_INFO_CHAR_UUID: Uuid = uuid!("B7C8D9E0-F1A2-3456-789B-CDEF01234567");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BleDeviceInfo {
    pub name: String,
    pub ip: String,
    pub port: u16,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod advertiser {
    // TODO(task-6): bluest 0.6 ne supporte que les rôles GAP Central + GATT Client
    // (peripheral/server roles non supportés — cf. https://docs.rs/bluest/0.6).
    // L'advertising BLE doit donc passer par un autre crate (ex: btleplug avec
    // peripheral support, ou bindings natifs CoreBluetooth/WinRT). Task 6 choisira
    // l'approche unifiée — pour l'instant, stub graceful pour ne pas bloquer la
    // compilation et permettre au scanner (côté client, supporté par bluest) d'avancer.

    pub async fn start(_name: &str, _ip: &str, _port: u16) -> Result<(), String> {
        log::warn!(
            "BLE advertising stub — bluest 0.6 API mismatch: peripheral/server roles not supported (only GAP Central + GATT Client)"
        );
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub mod advertiser {
    pub async fn start(_name: &str, _ip: &str, _port: u16) -> Result<(), String> {
        log::info!("BLE advertising non supporté sur cette plateforme");
        Ok(())
    }
}
