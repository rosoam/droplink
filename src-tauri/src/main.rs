#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod discovery;
mod protocol;
mod transfer;
mod tray;

use discovery::{find_free_port, get_local_ip, Discovery, DiscoveryEvent};
use protocol::DeviceInfo;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, Mutex};
use transfer::{send_file, PendingMap, TransferEvent, TransferServer};

struct AppState {
    devices: Arc<Mutex<Vec<DeviceInfo>>>,
    pending_transfers: PendingMap,
    device_name: String,
    event_tx: mpsc::Sender<TransferEvent>,
}

#[tauri::command]
async fn get_devices(state: State<'_, Arc<AppState>>) -> Result<Vec<DeviceInfo>, String> {
    Ok(state.devices.lock().await.clone())
}

#[tauri::command]
async fn send_file_to_device(
    state: State<'_, Arc<AppState>>,
    device: DeviceInfo,
    file_path: String,
) -> Result<(), String> {
    let event_tx = state.event_tx.clone();
    let device_name = state.device_name.clone();
    tokio::spawn(async move {
        if let Err(reason) = send_file(device, file_path, event_tx.clone(), device_name).await {
            let _ = event_tx.send(TransferEvent::Failed {
                transfer_id: String::new(),
                reason,
            }).await;
        }
    });
    Ok(())
}

#[tauri::command]
async fn respond_transfer(
    state: State<'_, Arc<AppState>>,
    transfer_id: String,
    accepted: bool,
) -> Result<(), String> {
    let mut pending = state.pending_transfers.lock().await;
    if let Some(tx) = pending.remove(&transfer_id) {
        let _ = tx.send(accepted);
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let app_handle: AppHandle = app.handle().clone();

            let device_name = hostname::get()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let local_ip = get_local_ip().unwrap_or_else(|| "127.0.0.1".to_string());
            let port = find_free_port(7777, 7800).expect("No free port found");

            let (transfer_event_tx, mut transfer_event_rx) =
                mpsc::channel::<TransferEvent>(64);

            let server = TransferServer::new(port, transfer_event_tx.clone());
            let pending_map = server.pending_map();

            tauri::async_runtime::spawn(async move {
                server.start().await.expect("Transfer server failed");
            });

            let discovery = Discovery::new(&device_name, &local_ip, port)
                .expect("mDNS init failed");

            let devices: Arc<Mutex<Vec<DeviceInfo>>> = Arc::new(Mutex::new(Vec::new()));
            let devices_clone = devices.clone();

            let (disc_tx, mut disc_rx) = mpsc::channel::<DiscoveryEvent>(32);
            discovery
                .start_browsing(disc_tx)
                .expect("mDNS browse failed");

            let app_handle_disc = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                while let Some(event) = disc_rx.recv().await {
                    let mut list = devices_clone.lock().await;
                    match event {
                        DiscoveryEvent::DeviceFound(d) => {
                            if !list.iter().any(|x| x.name == d.name) {
                                list.push(d.clone());
                                let _ = app_handle_disc.emit("device-discovered", d);
                            }
                        }
                        DiscoveryEvent::DeviceLost(fullname) => {
                            list.retain(|d| !fullname.starts_with(&format!("{}.", d.name)));
                            let _ = app_handle_disc.emit("device-lost", fullname);
                        }
                    }
                }
            });

            let app_handle_transfer = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                while let Some(event) = transfer_event_rx.recv().await {
                    match &event {
                        TransferEvent::IncomingRequest { .. } => {
                            let _ =
                                app_handle_transfer.emit("incoming-transfer", &event);
                        }
                        TransferEvent::Progress { .. } => {
                            let _ =
                                app_handle_transfer.emit("transfer-progress", &event);
                        }
                        TransferEvent::Complete { .. } => {
                            let _ =
                                app_handle_transfer.emit("transfer-complete", &event);
                        }
                        TransferEvent::Failed { .. } => {
                            let _ =
                                app_handle_transfer.emit("transfer-failed", &event);
                        }
                    }
                }
            });

            let state = Arc::new(AppState {
                devices,
                pending_transfers: pending_map,
                device_name,
                event_tx: transfer_event_tx,
            });
            app.manage(state);

            tray::setup_tray(&app_handle)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_devices,
            send_file_to_device,
            respond_transfer,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri app failed");
}
