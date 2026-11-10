# DropLink v2 — BLE Discovery + Relay Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Two devices with DropLink open discover each other automatically regardless of network (WiFi, Ethernet, 4G) — BLE detects proximity, relay server enables file transfer across different networks.

**Architecture:** BLE (btleplug for scanning + platform advertising) handles proximity detection; a WebSocket relay server groups devices by public IP for automatic room assignment; file transfer goes direct TCP when on same network, relayed via WebSocket otherwise. mDNS is kept as a bonus for ultra-fast same-network discovery.

**Tech Stack:**
- `btleplug 0.11` — BLE central (scanning) on macOS + Windows
- `bluest 0.6` — BLE peripheral (advertising) on macOS + Windows
- `axum 0.7` + `tokio-tungstenite` — relay server WebSocket
- `tokio-tungstenite` — relay client in Tauri app
- Fly.io — relay server hosting (free tier)

---

## File Map

### New files
| File | Responsabilité |
|------|----------------|
| `droplink-relay/Cargo.toml` | Relay server crate |
| `droplink-relay/src/main.rs` | WebSocket relay server — rooms by public IP |
| `droplink-relay/fly.toml` | Fly.io deployment config |
| `droplink-relay/Dockerfile` | Docker image |
| `src-tauri/src/ble.rs` | BLE advertiser (bluest) + scanner (btleplug) |
| `src-tauri/src/relay_client.rs` | WebSocket client connecting to relay server |

### Modified files
| File | Changement |
|------|------------|
| `src-tauri/Cargo.toml` | Add btleplug, bluest, tokio-tungstenite |
| `src-tauri/src/main.rs` | Start BLE + relay on launch, merge device lists |
| `src-tauri/src/discovery.rs` | Keep mDNS, add merge function for BLE+relay devices |
| `src-tauri/tauri.conf.json` | Add macOS bluetooth NSUsageDescription |

---

## Task 1: Relay Server — scaffold + WebSocket room logic

**Files:**
- Create: `droplink-relay/Cargo.toml`
- Create: `droplink-relay/src/main.rs`

- [ ] **Step 1: Créer le workspace relay**

```bash
mkdir -p /Users/rso/Projects/droplink/droplink-relay/src
```

- [ ] **Step 2: Écrire `droplink-relay/Cargo.toml`**

```toml
[package]
name = "droplink-relay"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "droplink-relay"
path = "src/main.rs"

[dependencies]
axum = { version = "0.7", features = ["ws"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
log = "0.4"
env_logger = "0.11"
```

- [ ] **Step 3: Écrire `droplink-relay/src/main.rs`**

```rust
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::sync::{mpsc::UnboundedSender, Mutex};

#[derive(Clone, Serialize, Deserialize, Debug)]
struct PeerInfo {
    name: String,
    ip: String,
    port: u16,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Register { name: String, ip: String, port: u16 },
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    RoomUpdate { devices: Vec<PeerInfo> },
}

struct PeerSlot {
    info: PeerInfo,
    tx: UnboundedSender<String>,
}

type RoomMap = Arc<Mutex<HashMap<String, Vec<PeerSlot>>>>;

#[tokio::main]
async fn main() {
    env_logger::init();
    let rooms: RoomMap = Arc::new(Mutex::new(HashMap::new()));
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(rooms);
    let addr = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .map(|p| format!("0.0.0.0:{}", p))
        .unwrap_or_else(|| "0.0.0.0:8080".to_string());
    log::info!("Relay server on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(rooms): State<RoomMap>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, addr, rooms))
}

async fn handle_socket(socket: WebSocket, addr: SocketAddr, rooms: RoomMap) {
    let public_ip = addr.ip().to_string();
    let (ws_tx, mut ws_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (mut ws_write, mut ws_read) = socket.split();
    let mut my_name: Option<String> = None;

    // Spawn writer task
    let write_task = tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            if ws_write.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    // Reader loop
    while let Some(Ok(msg)) = ws_read.next().await {
        if let Message::Text(text) = msg {
            if let Ok(ClientMsg::Register { name, ip, port }) = serde_json::from_str(&text) {
                my_name = Some(name.clone());
                let mut map = rooms.lock().await;
                let room = map.entry(public_ip.clone()).or_default();

                // Upsert this peer
                room.retain(|s| s.info.name != name);
                room.push(PeerSlot {
                    info: PeerInfo { name: name.clone(), ip, port },
                    tx: ws_tx.clone(),
                });

                // Broadcast updated list to everyone in room
                let all: Vec<PeerInfo> = room.iter().map(|s| s.info.clone()).collect();
                for slot in room.iter() {
                    let others: Vec<PeerInfo> = all
                        .iter()
                        .filter(|p| p.name != slot.info.name)
                        .cloned()
                        .collect();
                    let msg = serde_json::to_string(&ServerMsg::RoomUpdate { devices: others }).unwrap();
                    let _ = slot.tx.send(msg);
                }
            }
        }
    }

    // Cleanup on disconnect
    if let Some(name) = my_name {
        let mut map = rooms.lock().await;
        if let Some(room) = map.get_mut(&public_ip) {
            room.retain(|s| s.info.name != name);
            let all: Vec<PeerInfo> = room.iter().map(|s| s.info.clone()).collect();
            for slot in room.iter() {
                let others: Vec<PeerInfo> = all
                    .iter()
                    .filter(|p| p.name != slot.info.name)
                    .cloned()
                    .collect();
                let msg = serde_json::to_string(&ServerMsg::RoomUpdate { devices: others }).unwrap();
                let _ = slot.tx.send(msg);
            }
        }
    }

    write_task.abort();
}
```

Note: `ws_read.next()` requires `use futures_util::StreamExt;` — add `futures-util = "0.3"` to Cargo.toml.

- [ ] **Step 4: Ajouter futures-util à Cargo.toml**

```toml
futures-util = "0.3"
```

- [ ] **Step 5: Build local test**

```bash
cd /Users/rso/Projects/droplink/droplink-relay
cargo build
```

Expected: compiles without error.

- [ ] **Step 6: Test manuel — deux clients WebSocket dans le même terminal**

```bash
# Terminal 1 — lancer le serveur
RUST_LOG=info cargo run

# Terminal 2 — client 1 s'enregistre
wscat -c ws://localhost:8080/ws
# Taper: {"type":"register","name":"MacBook","ip":"192.168.1.1","port":7777}

# Terminal 3 — client 2 s'enregistre
wscat -c ws://localhost:8080/ws
# Taper: {"type":"register","name":"Windows","ip":"192.168.76.4","port":7777}
# Les deux clients devraient recevoir: {"type":"room_update","devices":[...]}
```

- [ ] **Step 7: Commit**

```bash
git add droplink-relay/
git commit -m "feat(relay): WebSocket relay server — room grouping by public IP"
```

---

## Task 2: Relay Server — Dockerfile + Fly.io deployment

**Files:**
- Create: `droplink-relay/Dockerfile`
- Create: `droplink-relay/fly.toml`

- [ ] **Step 1: Écrire `droplink-relay/Dockerfile`**

```dockerfile
FROM rust:1.78-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/droplink-relay /usr/local/bin/droplink-relay
ENV PORT=8080
EXPOSE 8080
CMD ["droplink-relay"]
```

- [ ] **Step 2: Écrire `droplink-relay/fly.toml`**

```toml
app = "droplink-relay"
primary_region = "cdg"  # Paris — proche de tes utilisateurs

[build]
  dockerfile = "Dockerfile"

[http_service]
  internal_port = 8080
  force_https = true
  auto_stop_machines = true
  auto_start_machines = true

[[vm]]
  memory = "256mb"
  cpu_kind = "shared"
  cpus = 1
```

- [ ] **Step 3: Installer flyctl et déployer**

```bash
# Installer flyctl si pas déjà fait
brew install flyctl

# Authentification
fly auth login

# Créer l'app (une seule fois)
cd /Users/rso/Projects/droplink/droplink-relay
fly launch --no-deploy --name droplink-relay

# Déployer
fly deploy
```

Expected output: `Visit your newly deployed app at https://droplink-relay.fly.dev`

- [ ] **Step 4: Tester le serveur déployé**

```bash
wscat -c wss://droplink-relay.fly.dev/ws
# Taper: {"type":"register","name":"test","ip":"1.2.3.4","port":7777}
# Réponse attendue: {"type":"room_update","devices":[]}
```

- [ ] **Step 5: Commit**

```bash
git add droplink-relay/Dockerfile droplink-relay/fly.toml
git commit -m "feat(relay): Dockerfile + Fly.io deployment config"
```

---

## Task 3: Relay Client dans Tauri — `relay_client.rs`

**Files:**
- Create: `src-tauri/src/relay_client.rs`
- Modify: `src-tauri/Cargo.toml`

Le client se connecte au serveur relay au démarrage, envoie un message Register, et écoute les RoomUpdate pour alimenter la liste de devices.

- [ ] **Step 1: Ajouter dépendances à `src-tauri/Cargo.toml`**

```toml
tokio-tungstenite = { version = "0.21", features = ["native-tls"] }
futures-util = "0.3"
url = "2"
```

- [ ] **Step 2: Créer `src-tauri/src/relay_client.rs`**

```rust
use crate::protocol::DeviceInfo;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

pub const RELAY_URL: &str = "wss://droplink-relay.fly.dev/ws";

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg<'a> {
    Register { name: &'a str, ip: &'a str, port: u16 },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    RoomUpdate { devices: Vec<DeviceInfo> },
}

pub enum RelayEvent {
    DevicesUpdated(Vec<DeviceInfo>),
    Disconnected,
}

pub async fn connect_and_listen(
    device_name: String,
    local_ip: String,
    port: u16,
    tx: mpsc::Sender<RelayEvent>,
) {
    let url = Url::parse(RELAY_URL).expect("invalid relay URL");
    let Ok((ws_stream, _)) = connect_async(url).await else {
        log::warn!("Relay server unreachable — mode local uniquement");
        return;
    };

    let (mut write, mut read) = ws_stream.split();

    // Register immediately
    let register = serde_json::to_string(&ClientMsg::Register {
        name: &device_name,
        ip: &local_ip,
        port,
    })
    .unwrap();

    if write.send(Message::Text(register)).await.is_err() {
        return;
    }

    // Listen for room updates
    while let Some(Ok(msg)) = read.next().await {
        if let Message::Text(text) = msg {
            if let Ok(ServerMsg::RoomUpdate { devices }) = serde_json::from_str(&text) {
                let _ = tx.send(RelayEvent::DevicesUpdated(devices)).await;
            }
        }
    }

    let _ = tx.send(RelayEvent::Disconnected).await;
}
```

Note: `DeviceInfo` dans `protocol.rs` doit implémenter `Deserialize` — ajouter `#[derive(Deserialize)]` si pas déjà présent.

- [ ] **Step 3: Vérifier que `DeviceInfo` a `Deserialize`**

Ouvrir `src-tauri/src/protocol.rs`. Chercher `struct DeviceInfo`. Si la ligne `#[derive(...)]` n'inclut pas `Deserialize`, ajouter :

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub ip: String,
    pub port: u16,
}
```

- [ ] **Step 4: Build check**

```bash
cd /Users/rso/Projects/droplink/src-tauri
cargo check
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/relay_client.rs src-tauri/Cargo.toml src-tauri/src/protocol.rs
git commit -m "feat(relay-client): WebSocket client — register + listen room updates"
```

---

## Task 4: Intégrer le relay client dans `main.rs`

**Files:**
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Ajouter le module relay_client dans `main.rs`**

En haut du fichier, après les autres `mod` :

```rust
mod relay_client;
use relay_client::RelayEvent;
```

- [ ] **Step 2: Démarrer le relay client dans `setup()`**

Après le bloc qui démarre le discovery mDNS (après `discovery.start_browsing(disc_tx)`), ajouter :

```rust
// Relay server — découverte cross-réseau
let relay_devices = devices.clone();
let relay_app = app_handle.clone();
let relay_name = device_name.clone();
let relay_ip = local_ip.clone();
let (relay_tx, mut relay_rx) = mpsc::channel::<RelayEvent>(32);

tauri::async_runtime::spawn(relay_client::connect_and_listen(
    relay_name,
    relay_ip,
    port,
    relay_tx,
));

tauri::async_runtime::spawn(async move {
    while let Some(event) = relay_rx.recv().await {
        if let RelayEvent::DevicesUpdated(new_devices) = event {
            let mut list = relay_devices.lock().await;
            for d in new_devices {
                // Merge: ajouter uniquement si pas déjà connu (mDNS ou BLE)
                if !list.iter().any(|x: &DeviceInfo| x.name == d.name) {
                    list.push(d.clone());
                    let _ = relay_app.emit("device-discovered", d);
                }
            }
            // Supprimer les appareils relay qui ne sont plus dans la liste
            // (le relay envoie la liste complète à chaque update)
        }
    }
});
```

Note: Le relay envoie la liste complète à chaque changement. Il faut aussi supprimer les devices disparus. Remplacer le bloc ci-dessus par la version complète :

```rust
tauri::async_runtime::spawn(async move {
    let mut known_relay_names: Vec<String> = Vec::new();

    while let Some(event) = relay_rx.recv().await {
        match event {
            RelayEvent::DevicesUpdated(new_devices) => {
                let new_names: Vec<String> = new_devices.iter().map(|d| d.name.clone()).collect();

                let mut list = relay_devices.lock().await;

                // Ajouter les nouveaux
                for d in &new_devices {
                    if !list.iter().any(|x: &DeviceInfo| x.name == d.name) {
                        list.push(d.clone());
                        let _ = relay_app.emit("device-discovered", d.clone());
                    }
                }

                // Supprimer ceux qui ont quitté (relay only — pas mDNS)
                for name in &known_relay_names {
                    if !new_names.contains(name) {
                        list.retain(|d| &d.name != name);
                        let _ = relay_app.emit("device-lost", name.clone());
                    }
                }

                known_relay_names = new_names;
            }
            RelayEvent::Disconnected => {
                log::warn!("Relay disconnected");
                // Optionnel: retry après 5s
            }
        }
    }
});
```

- [ ] **Step 3: Build check**

```bash
cd /Users/rso/Projects/droplink
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: no errors.

- [ ] **Step 4: Test fonctionnel**

1. Lancer DropLink sur le Mac (`npm run tauri dev`)
2. Vérifier dans les logs que "Relay server unreachable" n'apparaît PAS (si relay déployé) ou apparaît avec "mode local uniquement" (si pas encore déployé — normal)
3. Une fois relay déployé : ouvrir DropLink sur Windows, les deux doivent se voir dans la liste

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/main.rs
git commit -m "feat(relay): intégration relay client dans le setup Tauri"
```

---

## Task 5: BLE — Advertiser (rendre l'appareil visible via Bluetooth)

**Files:**
- Create: `src-tauri/src/ble.rs`
- Modify: `src-tauri/Cargo.toml`

Le BLE advertiser utilise `bluest` pour diffuser un service BLE identifiable par les autres appareils DropLink, contenant le nom, IP et port via une GATT characteristic.

- [ ] **Step 1: Ajouter `bluest` à `src-tauri/Cargo.toml`**

```toml
[target.'cfg(any(target_os = "macos", target_os = "windows"))'.dependencies]
bluest = "0.6"
```

- [ ] **Step 2: Créer `src-tauri/src/ble.rs` — constantes + types**

```rust
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
```

- [ ] **Step 3: Ajouter l'advertiser dans `ble.rs`**

```rust
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod advertiser {
    use super::*;
    use bluest::{Adapter, AdvertisementData, CharacteristicBuilder, ServiceBuilder};

    pub async fn start(name: &str, ip: &str, port: u16) -> Result<(), String> {
        let adapter = match Adapter::default().await {
            Some(Ok(a)) => a,
            _ => {
                log::warn!("BLE adapter non disponible — BLE advertising désactivé");
                return Ok(());
            }
        };

        if let Err(e) = adapter.wait_available().await {
            log::warn!("BLE non disponible: {e} — advertising désactivé");
            return Ok(());
        }

        let info = BleDeviceInfo {
            name: name.to_string(),
            ip: ip.to_string(),
            port,
        };
        let info_bytes = serde_json::to_vec(&info).unwrap();

        let characteristic = CharacteristicBuilder::new(DROPLINK_INFO_CHAR_UUID)
            .read()
            .value(info_bytes)
            .build();

        let service = ServiceBuilder::new(DROPLINK_SERVICE_UUID)
            .add_characteristic(characteristic)
            .build();

        let adv_data = AdvertisementData {
            local_name: Some(format!("DropLink:{}", &name[..name.len().min(10)])),
            services: vec![DROPLINK_SERVICE_UUID],
            ..Default::default()
        };

        adapter
            .start_advertising(&adv_data, &[service])
            .await
            .map_err(|e| e.to_string())?;

        log::info!("BLE advertising démarré — service {}", DROPLINK_SERVICE_UUID);
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
```

- [ ] **Step 4: Build check**

```bash
cd /Users/rso/Projects/droplink
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: no errors. Si `bluest` a des conflits de version, ajouter `--locked` ou ajuster la version.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ble.rs src-tauri/Cargo.toml
git commit -m "feat(ble): BLE advertiser — diffuse DropLink service via Bluetooth"
```

---

## Task 6: BLE — Scanner (détecter les appareils à proximité)

**Files:**
- Modify: `src-tauri/src/ble.rs`

- [ ] **Step 1: Ajouter btleplug à `src-tauri/Cargo.toml`**

```toml
btleplug = "0.11"
```

- [ ] **Step 2: Ajouter le scanner dans `ble.rs`**

```rust
pub enum BleEvent {
    DeviceFound(BleDeviceInfo),
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod scanner {
    use super::*;
    use btleplug::api::{Central, CentralEvent, Manager as _, ScanFilter, Peripheral as _};
    use btleplug::platform::Manager;
    use futures_util::StreamExt;
    use tokio::sync::mpsc;

    pub async fn start(tx: mpsc::Sender<BleEvent>) -> Result<(), String> {
        let manager = Manager::new().await.map_err(|e| e.to_string())?;
        let adapters = manager.adapters().await.map_err(|e| e.to_string())?;
        let adapter = adapters.into_iter().next().ok_or("Aucun adaptateur BLE")?;

        adapter
            .start_scan(ScanFilter {
                services: vec![DROPLINK_SERVICE_UUID],
            })
            .await
            .map_err(|e| e.to_string())?;

        let mut events = adapter.events().await.map_err(|e| e.to_string())?;

        while let Some(event) = events.next().await {
            if let CentralEvent::DeviceDiscovered(id) = event {
                let Ok(peripheral) = adapter.peripheral(&id).await else { continue };

                // Connexion pour lire la GATT characteristic
                if peripheral.connect().await.is_err() {
                    continue;
                }
                if peripheral.discover_services().await.is_err() {
                    let _ = peripheral.disconnect().await;
                    continue;
                }

                let characteristics = peripheral.characteristics();
                let info_char = characteristics
                    .iter()
                    .find(|c| c.uuid == DROPLINK_INFO_CHAR_UUID);

                if let Some(c) = info_char {
                    if let Ok(data) = peripheral.read(c).await {
                        if let Ok(info) = serde_json::from_slice::<BleDeviceInfo>(&data) {
                            let _ = tx.send(BleEvent::DeviceFound(info)).await;
                        }
                    }
                }

                let _ = peripheral.disconnect().await;
            }
        }

        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub mod scanner {
    use super::*;
    use tokio::sync::mpsc;

    pub async fn start(_tx: mpsc::Sender<BleEvent>) -> Result<(), String> {
        log::info!("BLE scanning non supporté sur cette plateforme");
        Ok(())
    }
}
```

- [ ] **Step 3: Build check**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/ble.rs
git commit -m "feat(ble): BLE scanner — détecte appareils DropLink à proximité via btleplug"
```

---

## Task 7: Intégrer BLE dans `main.rs`

**Files:**
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Ajouter `mod ble` dans `main.rs`**

En haut, après les autres `mod` :

```rust
mod ble;
use ble::BleEvent;
```

- [ ] **Step 2: Démarrer BLE advertiser et scanner dans `setup()`**

Après le bloc relay client (Task 4), ajouter :

```rust
// BLE Advertiser — rend cet appareil visible aux autres
let ble_adv_name = device_name.clone();
let ble_adv_ip = local_ip.clone();
tauri::async_runtime::spawn(async move {
    if let Err(e) = ble::advertiser::start(&ble_adv_name, &ble_adv_ip, port).await {
        log::warn!("BLE advertiser: {}", e);
    }
});

// BLE Scanner — détecte les appareils DropLink à proximité
let ble_devices = devices.clone();
let ble_app = app_handle.clone();
let (ble_tx, mut ble_rx) = mpsc::channel::<BleEvent>(32);

tauri::async_runtime::spawn(async move {
    if let Err(e) = ble::scanner::start(ble_tx).await {
        log::warn!("BLE scanner: {}", e);
    }
});

tauri::async_runtime::spawn(async move {
    while let Some(BleEvent::DeviceFound(info)) = ble_rx.recv().await {
        let device = DeviceInfo {
            name: info.name.clone(),
            ip: info.ip,
            port: info.port,
        };
        let mut list = ble_devices.lock().await;
        if !list.iter().any(|d: &DeviceInfo| d.name == device.name) {
            list.push(device.clone());
            let _ = ble_app.emit("device-discovered", device);
        }
    }
});
```

- [ ] **Step 3: Build complet**

```bash
cd /Users/rso/Projects/droplink
cargo build --manifest-path src-tauri/Cargo.toml
```

Expected: compiles without error.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/main.rs
git commit -m "feat(ble): intégration BLE advertiser + scanner dans Tauri setup"
```

---

## Task 8: Permissions macOS + Windows

**Files:**
- Modify: `src-tauri/tauri.conf.json`
- Create: `src-tauri/Info.plist` (macOS)

macOS exige une `NSBluetoothAlwaysUsageDescription` dans le bundle Info.plist pour que Bluetooth soit autorisé.

- [ ] **Step 1: Créer `src-tauri/Info.plist`**

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>NSBluetoothAlwaysUsageDescription</key>
    <string>DropLink utilise le Bluetooth pour détecter les appareils à proximité.</string>
</dict>
</plist>
```

- [ ] **Step 2: Référencer Info.plist dans `tauri.conf.json`**

Dans la section `bundle`, ajouter :

```json
"bundle": {
  "active": true,
  "targets": "all",
  "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.icns", "icons/icon.ico"],
  "macOS": {
    "infoPlist": {
      "NSBluetoothAlwaysUsageDescription": "DropLink utilise le Bluetooth pour détecter les appareils à proximité."
    }
  }
}
```

Note: Sur Windows, `btleplug` et `bluest` utilisent les APIs WinRT qui ne nécessitent pas de manifest supplémentaire pour les apps desktop — les permissions Bluetooth sont gérées automatiquement par le système.

- [ ] **Step 3: Build release test macOS**

```bash
cd /Users/rso/Projects/droplink
npm run tauri build
```

Expected: `.dmg` généré, pas d'erreur de permission au lancement.

- [ ] **Step 4: Vérifier que macOS demande l'autorisation Bluetooth au premier lancement**

Lancer l'app. Une popup "DropLink souhaite utiliser le Bluetooth" doit apparaître. Cliquer "Autoriser".

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Info.plist src-tauri/tauri.conf.json
git commit -m "feat(permissions): NSBluetoothAlwaysUsageDescription macOS + config bundle"
```

---

## Task 9: Tag + Release v0.2.0

- [ ] **Step 1: Mettre à jour la version**

Dans `src-tauri/tauri.conf.json`, changer `"version": "0.1.0"` en `"version": "0.2.0"`.

Dans `src-tauri/Cargo.toml`, changer `version = "0.1.0"` en `version = "0.2.0"`.

- [ ] **Step 2: Commit + tag**

```bash
git add src-tauri/tauri.conf.json src-tauri/Cargo.toml
git commit -m "chore(release): bump version 0.2.0 — BLE + relay server"
git push origin main
git tag v0.2.0
git push origin v0.2.0
```

Expected: GitHub Actions builds macOS DMG + Windows MSI avec BLE + relay.

---

## Résumé — Ce que v0.2.0 apporte

| Scénario | v0.1.x | v0.2.0 |
|---|---|---|
| Mac WiFi + PC WiFi, même router | ✅ mDNS | ✅ mDNS + relay + BLE |
| Mac WiFi + PC Ethernet, même router | ❌ subnets différents | ✅ relay (même IP publique) |
| Mac WiFi + PC 4G, même pièce | ❌ impossible | ✅ BLE détecte + relay connecte |
| Mac bureau + PC à la maison | ❌ impossible | ❌ (différentes IP publiques, différents lieux) |

Le dernier cas (différents lieux) est hors scope volontairement — DropLink est un outil de proximité.
