# DropLink Core App — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a cross-platform desktop app (Mac/Windows/Linux) permettant le partage de fichiers P2P par proximité réseau, sans compte ni serveur, via drag & drop et popup accept/refuse.

**Architecture:** mDNS (DNS-SD) pour la découverte des appareils sur le même réseau local, TCP chunked pour le transfert de fichiers avec vérification SHA-256. Tauri 2 comme shell (Rust backend + React/TypeScript frontend). Les intégrations right-click OS sont dans le Plan 2 (séparé).

**Tech Stack:** Tauri 2, Rust (tokio, mdns-sd, sha2, serde_json, uuid), React 18, TypeScript, Tailwind CSS

---

## Fichiers créés dans ce plan

```
droplink/
├── src-tauri/
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs          — entry point, Tauri builder, commandes + events
│       ├── protocol.rs      — types sérialisés: DeviceInfo, Message enum
│       ├── discovery.rs     — mDNS advertising + browsing, events vers frontend
│       ├── transfer.rs      — TCP server + client, chunking 64KB, SHA-256, progress
│       └── tray.rs          — system tray
├── src/
│   ├── types.ts
│   ├── App.tsx
│   ├── components/
│   │   ├── DeviceList.tsx
│   │   ├── DropZone.tsx
│   │   ├── TransferModal.tsx
│   │   └── AcceptPopup.tsx
│   └── hooks/
│       ├── useDevices.ts
│       └── useTransfer.ts
├── tailwind.config.ts
└── index.css
```

---

### Task 1 : Project scaffold

**Files:**
- Create: `droplink/` (nouveau projet Tauri)
- Modify: `src-tauri/Cargo.toml`
- Modify: `package.json`

- [ ] **Step 1 : Créer le projet Tauri 2 avec template React-TS**

```bash
cd /Users/rso/Projects
npm create tauri-app@latest droplink -- --template react-ts --manager npm
cd droplink
```

Expected output : répertoire `droplink/` créé avec `src/`, `src-tauri/`, `package.json`.

- [ ] **Step 2 : Vérifier que le scaffold compile**

```bash
cd /Users/rso/Projects/droplink
npm install
npm run tauri dev
```

Expected : fenêtre Tauri s'ouvre avec "Welcome to Tauri + React". Fermer la fenêtre.

- [ ] **Step 3 : Ajouter les dépendances Rust**

Ouvrir `src-tauri/Cargo.toml`, remplacer la section `[dependencies]` par :

```toml
[dependencies]
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-dialog = "2"
tauri-plugin-shell = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
mdns-sd = "0.11"
sha2 = "0.10"
hex = "0.4"
uuid = { version = "1", features = ["v4"] }
log = "0.4"
env_logger = "0.11"
```

- [ ] **Step 4 : Ajouter les dépendances npm**

```bash
cd /Users/rso/Projects/droplink
npm install @tauri-apps/api @tauri-apps/plugin-dialog
npm install -D tailwindcss @tailwindcss/vite
```

- [ ] **Step 5 : Configurer Tailwind**

Créer `tailwind.config.ts` :

```typescript
import type { Config } from 'tailwindcss'

export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: { extend: {} },
  plugins: [],
} satisfies Config
```

Remplacer `src/index.css` :

```css
@import "tailwindcss";
```

Modifier `vite.config.ts` pour ajouter le plugin Tailwind :

```typescript
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
});
```

- [ ] **Step 6 : Vérifier que Tailwind fonctionne**

Dans `src/App.tsx`, remplacer tout le contenu par :

```tsx
export default function App() {
  return <div className="bg-gray-900 text-white min-h-screen p-4">DropLink</div>;
}
```

```bash
npm run tauri dev
```

Expected : fenêtre avec fond sombre et texte "DropLink".

- [ ] **Step 7 : Commit initial**

```bash
git init
git add .
git commit -m "feat: scaffold Tauri 2 + React TS + Tailwind"
```

---

### Task 2 : Protocol types (Rust)

**Files:**
- Create: `src-tauri/src/protocol.rs`
- Modify: `src-tauri/src/main.rs` (ajouter `mod protocol;`)

- [ ] **Step 1 : Écrire le test du parsing DeviceInfo**

Créer `src-tauri/src/protocol.rs` :

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceInfo {
    pub name: String,
    pub ip: String,
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Message {
    Hello {
        sender_name: String,
        file_name: String,
        file_size: u64,
        checksum: String,
        transfer_id: String,
    },
    Accept {
        transfer_id: String,
    },
    Refuse {
        transfer_id: String,
    },
    Done,
    DoneAck,
    Error {
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_info_roundtrip() {
        let device = DeviceInfo {
            name: "MacBook de Léa".to_string(),
            ip: "192.168.1.42".to_string(),
            port: 7777,
        };
        let json = serde_json::to_string(&device).unwrap();
        let parsed: DeviceInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(device, parsed);
    }

    #[test]
    fn test_message_hello_serialization() {
        let msg = Message::Hello {
            sender_name: "Léa".to_string(),
            file_name: "photo.jpg".to_string(),
            file_size: 1024,
            checksum: "abc123".to_string(),
            transfer_id: "uuid-1234".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"HELLO\""));
        assert!(json.contains("photo.jpg"));
    }

    #[test]
    fn test_message_accept_deserialization() {
        let json = r#"{"type":"ACCEPT","transfer_id":"uuid-1234"}"#;
        let msg: Message = serde_json::from_str(json).unwrap();
        match msg {
            Message::Accept { transfer_id } => assert_eq!(transfer_id, "uuid-1234"),
            _ => panic!("Wrong variant"),
        }
    }
}
```

- [ ] **Step 2 : Ajouter `mod protocol;` dans main.rs**

Ouvrir `src-tauri/src/main.rs`, ajouter en haut :

```rust
mod protocol;
```

- [ ] **Step 3 : Lancer les tests et vérifier qu'ils passent**

```bash
cd /Users/rso/Projects/droplink/src-tauri
cargo test protocol
```

Expected : `3 tests passed`.

- [ ] **Step 4 : Commit**

```bash
cd /Users/rso/Projects/droplink
git add src-tauri/src/protocol.rs src-tauri/src/main.rs
git commit -m "feat: protocol types with serde roundtrip tests"
```

---

### Task 3 : Discovery module (mDNS)

**Files:**
- Create: `src-tauri/src/discovery.rs`
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1 : Écrire les tests unitaires du discovery**

Créer `src-tauri/src/discovery.rs` :

```rust
use crate::protocol::DeviceInfo;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

const SERVICE_TYPE: &str = "_droplink._tcp.local.";
const CHUNK_SCAN_INTERVAL_MS: u64 = 200;

pub struct Discovery {
    mdns: ServiceDaemon,
    service_name: String,
}

pub enum DiscoveryEvent {
    DeviceFound(DeviceInfo),
    DeviceLost(String),
}

impl Discovery {
    pub fn new(device_name: &str, local_ip: &str, port: u16) -> Result<Self, String> {
        let mdns = ServiceDaemon::new().map_err(|e| e.to_string())?;
        let hostname = format!("{}.local.", device_name.replace(' ', "-").to_lowercase());
        let service_name = device_name.to_string();

        let mut properties = HashMap::new();
        properties.insert("name".to_string(), device_name.to_string());

        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            device_name,
            &hostname,
            local_ip,
            port,
            Some(properties),
        )
        .map_err(|e| e.to_string())?;

        mdns.register(service_info).map_err(|e| e.to_string())?;

        Ok(Discovery { mdns, service_name })
    }

    pub fn start_browsing(&self, tx: mpsc::Sender<DiscoveryEvent>) -> Result<(), String> {
        let receiver = self
            .mdns
            .browse(SERVICE_TYPE)
            .map_err(|e| e.to_string())?;
        let own_name = self.service_name.clone();

        tokio::spawn(async move {
            loop {
                match receiver.recv_async().await {
                    Ok(ServiceEvent::ServiceResolved(info)) => {
                        let name = info
                            .get_property_val_str("name")
                            .unwrap_or(info.get_fullname())
                            .to_string();
                        if name == own_name {
                            continue;
                        }
                        if let Some(addr) = info.get_addresses().iter().next() {
                            let device = DeviceInfo {
                                name: name.clone(),
                                ip: addr.to_string(),
                                port: info.get_port(),
                            };
                            let _ = tx.send(DiscoveryEvent::DeviceFound(device)).await;
                        }
                    }
                    Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                        let _ = tx
                            .send(DiscoveryEvent::DeviceLost(fullname.clone()))
                            .await;
                    }
                    Err(_) => break,
                    _ => {}
                }
            }
        });

        Ok(())
    }
}

pub fn find_free_port(start: u16, end: u16) -> Option<u16> {
    for port in start..=end {
        if std::net::TcpListener::bind(("0.0.0.0", port)).is_ok() {
            return Some(port);
        }
    }
    None
}

pub fn get_local_ip() -> Option<String> {
    use std::net::UdpSocket;
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_free_port_returns_valid_port() {
        let port = find_free_port(7777, 7800);
        assert!(port.is_some());
        let p = port.unwrap();
        assert!(p >= 7777 && p <= 7800);
    }

    #[test]
    fn test_get_local_ip_returns_ipv4() {
        let ip = get_local_ip();
        assert!(ip.is_some());
        let ip_str = ip.unwrap();
        assert!(ip_str.contains('.'), "Expected IPv4, got: {}", ip_str);
    }
}
```

- [ ] **Step 2 : Ajouter `mod discovery;` dans main.rs**

```rust
mod discovery;
mod protocol;
```

- [ ] **Step 3 : Lancer les tests**

```bash
cd /Users/rso/Projects/droplink/src-tauri
cargo test discovery
```

Expected : `2 tests passed`.

- [ ] **Step 4 : Commit**

```bash
cd /Users/rso/Projects/droplink
git add src-tauri/src/discovery.rs src-tauri/src/main.rs
git commit -m "feat: mDNS discovery module with port scanner"
```

---

### Task 4 : Transfer engine (TCP)

**Files:**
- Create: `src-tauri/src/transfer.rs`
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1 : Écrire les tests du transfer engine**

Créer `src-tauri/src/transfer.rs` :

```rust
use crate::protocol::{DeviceInfo, Message};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};

const CHUNK_SIZE: usize = 65536; // 64KB

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransferEvent {
    IncomingRequest {
        transfer_id: String,
        sender_name: String,
        file_name: String,
        file_size: u64,
    },
    Progress {
        transfer_id: String,
        bytes_done: u64,
        total: u64,
        direction: String,
    },
    Complete {
        transfer_id: String,
        file_name: String,
        saved_path: String,
    },
    Failed {
        transfer_id: String,
        reason: String,
    },
}

pub type PendingMap = Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>;

pub struct TransferServer {
    port: u16,
    pending: PendingMap,
    event_tx: mpsc::Sender<TransferEvent>,
}

impl TransferServer {
    pub fn new(port: u16, event_tx: mpsc::Sender<TransferEvent>) -> Self {
        TransferServer {
            port,
            pending: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
        }
    }

    pub fn pending_map(&self) -> PendingMap {
        self.pending.clone()
    }

    pub async fn start(&self) -> Result<(), String> {
        let listener = TcpListener::bind(format!("0.0.0.0:{}", self.port))
            .await
            .map_err(|e| e.to_string())?;

        let pending = self.pending.clone();
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            loop {
                if let Ok((stream, _addr)) = listener.accept().await {
                    let pending = pending.clone();
                    let event_tx = event_tx.clone();
                    tokio::spawn(handle_incoming(stream, pending, event_tx));
                }
            }
        });

        Ok(())
    }
}

async fn handle_incoming(
    stream: TcpStream,
    pending: PendingMap,
    event_tx: mpsc::Sender<TransferEvent>,
) {
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut writer = BufWriter::new(writer);

    // Read HELLO message (newline-delimited JSON)
    let mut line = String::new();
    if tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line)
        .await
        .is_err()
    {
        return;
    }

    let hello: Message = match serde_json::from_str(line.trim()) {
        Ok(m) => m,
        Err(_) => return,
    };

    let (transfer_id, sender_name, file_name, file_size, checksum) = match hello {
        Message::Hello {
            transfer_id,
            sender_name,
            file_name,
            file_size,
            checksum,
        } => (transfer_id, sender_name, file_name, file_size, checksum),
        _ => return,
    };

    // Notify frontend — user must accept/refuse
    let _ = event_tx
        .send(TransferEvent::IncomingRequest {
            transfer_id: transfer_id.clone(),
            sender_name,
            file_name: file_name.clone(),
            file_size,
        })
        .await;

    // Wait for user decision (10s timeout)
    let (decision_tx, decision_rx) = tokio::sync::oneshot::channel::<bool>();
    pending
        .lock()
        .await
        .insert(transfer_id.clone(), decision_tx);

    let accepted = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        decision_rx,
    )
    .await
    .unwrap_or(Ok(false))
    .unwrap_or(false);

    if !accepted {
        let refuse = serde_json::to_string(&Message::Refuse {
            transfer_id: transfer_id.clone(),
        })
        .unwrap()
            + "\n";
        let _ = writer.write_all(refuse.as_bytes()).await;
        let _ = writer.flush().await;
        return;
    }

    // Send ACCEPT
    let accept =
        serde_json::to_string(&Message::Accept { transfer_id: transfer_id.clone() }).unwrap()
            + "\n";
    let _ = writer.write_all(accept.as_bytes()).await;
    let _ = writer.flush().await;

    // Receive file
    let downloads_dir = dirs_next::download_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let droplink_dir = downloads_dir.join("DropLink");
    let _ = tokio::fs::create_dir_all(&droplink_dir).await;

    let safe_name = sanitize_filename(&file_name);
    let dest_path = unique_path(&droplink_dir, &safe_name);

    let mut file = match tokio::fs::File::create(&dest_path).await {
        Ok(f) => f,
        Err(e) => {
            let _ = event_tx
                .send(TransferEvent::Failed {
                    transfer_id,
                    reason: e.to_string(),
                })
                .await;
            return;
        }
    };

    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut bytes_done: u64 = 0;
    let mut hasher = Sha256::new();

    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
                let _ = file.write_all(&buf[..n]).await;
                bytes_done += n as u64;

                let _ = event_tx
                    .send(TransferEvent::Progress {
                        transfer_id: transfer_id.clone(),
                        bytes_done,
                        total: file_size,
                        direction: "receiving".to_string(),
                    })
                    .await;

                if bytes_done >= file_size {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let actual_checksum = format!("{:x}", hasher.finalize());
    if actual_checksum != checksum {
        let _ = tokio::fs::remove_file(&dest_path).await;
        let _ = event_tx
            .send(TransferEvent::Failed {
                transfer_id,
                reason: "Checksum mismatch — file corrupted".to_string(),
            })
            .await;
        return;
    }

    let _ = event_tx
        .send(TransferEvent::Complete {
            transfer_id,
            file_name,
            saved_path: dest_path.to_string_lossy().to_string(),
        })
        .await;
}

pub async fn send_file(
    device: DeviceInfo,
    file_path: String,
    event_tx: mpsc::Sender<TransferEvent>,
    device_name: String,
) -> Result<(), String> {
    let path = Path::new(&file_path);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();

    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| e.to_string())?;
    let file_size = file.metadata().await.map_err(|e| e.to_string())?.len();

    // Compute checksum
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = file.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let checksum = format!("{:x}", hasher.finalize());

    // Reopen file for streaming
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| e.to_string())?;

    let transfer_id = uuid::Uuid::new_v4().to_string();
    let mut stream = TcpStream::connect(format!("{}:{}", device.ip, device.port))
        .await
        .map_err(|e| e.to_string())?;

    let hello = serde_json::to_string(&Message::Hello {
        transfer_id: transfer_id.clone(),
        sender_name: device_name,
        file_name: file_name.clone(),
        file_size,
        checksum,
    })
    .unwrap()
        + "\n";

    stream
        .write_all(hello.as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    // Wait for ACCEPT/REFUSE — scoped block releases borrow before stream re-use
    let response: Message = {
        let mut reader = BufReader::new(&mut stream);
        let mut response_line = String::new();
        tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut response_line)
            .await
            .map_err(|e| e.to_string())?;
        serde_json::from_str(response_line.trim()).map_err(|e| e.to_string())?
    };

    match response {
        Message::Refuse { .. } => {
            return Err("Refused".to_string());
        }
        Message::Accept { .. } => {}
        _ => return Err("Unexpected response".to_string()),
    }

    // Stream file
    let mut bytes_done: u64 = 0;
    loop {
        let n = file.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        stream
            .write_all(&buf[..n])
            .await
            .map_err(|e| e.to_string())?;
        bytes_done += n as u64;

        let _ = event_tx
            .send(TransferEvent::Progress {
                transfer_id: transfer_id.clone(),
                bytes_done,
                total: file_size,
                direction: "sending".to_string(),
            })
            .await;
    }

    stream.flush().await.map_err(|e| e.to_string())?;

    let _ = event_tx
        .send(TransferEvent::Complete {
            transfer_id,
            file_name,
            saved_path: file_path,
        })
        .await;

    Ok(())
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn unique_path(dir: &Path, name: &str) -> std::path::PathBuf {
    let base = dir.join(name);
    if !base.exists() {
        return base;
    }
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| format!(".{}", e))
        .unwrap_or_default();
    let mut i = 1;
    loop {
        let candidate = dir.join(format!("{} ({}){}", stem, i, ext));
        if !candidate.exists() {
            return candidate;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_filename("my file/path.png"), "my_file_path.png");
        assert_eq!(sanitize_filename("../evil.sh"), ".._evil.sh");
    }

    #[test]
    fn test_unique_path_no_collision() {
        let tmp = std::env::temp_dir();
        let path = unique_path(&tmp, "droplink_test_nonexistent_12345.txt");
        assert_eq!(path, tmp.join("droplink_test_nonexistent_12345.txt"));
    }
}
```

- [ ] **Step 2 : Ajouter la dépendance `dirs-next`**

Dans `src-tauri/Cargo.toml`, ajouter :

```toml
dirs-next = "2"
```

- [ ] **Step 3 : Ajouter `mod transfer;` dans main.rs**

```rust
mod discovery;
mod protocol;
mod transfer;
```

- [ ] **Step 4 : Lancer les tests**

```bash
cd /Users/rso/Projects/droplink/src-tauri
cargo test transfer
```

Expected : `2 tests passed` (`test_sanitize_filename`, `test_unique_path_no_collision`).

- [ ] **Step 5 : Commit**

```bash
cd /Users/rso/Projects/droplink
git add src-tauri/src/transfer.rs src-tauri/src/main.rs src-tauri/Cargo.toml
git commit -m "feat: TCP transfer engine with SHA-256 verification"
```

---

### Task 5 : Tauri main.rs — commands + events bridge

**Files:**
- Modify: `src-tauri/src/main.rs` (remplacer entièrement)
- Create: `src-tauri/src/tray.rs`

- [ ] **Step 1 : Créer tray.rs**

Créer `src-tauri/src/tray.rs` :

```rust
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

pub fn setup_tray(app: &AppHandle) -> Result<(), tauri::Error> {
    let show = MenuItem::with_id(app, "show", "Afficher DropLink", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let _tray = TrayIconBuilder::new()
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}
```

- [ ] **Step 2 : Réécrire main.rs complet**

Remplacer `src-tauri/src/main.rs` entièrement :

```rust
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
        let _ = send_file(device, file_path, event_tx, device_name).await;
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
                            list.retain(|d| !fullname.contains(&d.name));
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
```

- [ ] **Step 3 : Ajouter la dépendance `hostname`**

Dans `src-tauri/Cargo.toml`, ajouter :

```toml
hostname = "0.4"
```

- [ ] **Step 4 : Vérifier que ça compile**

```bash
cd /Users/rso/Projects/droplink/src-tauri
cargo build 2>&1
```

Expected : compilation sans erreur (warnings OK).

- [ ] **Step 5 : Commit**

```bash
cd /Users/rso/Projects/droplink
git add src-tauri/src/main.rs src-tauri/src/tray.rs src-tauri/Cargo.toml
git commit -m "feat: Tauri commands bridge + system tray"
```

---

### Task 6 : TypeScript types + hooks

**Files:**
- Create: `src/types.ts`
- Create: `src/hooks/useDevices.ts`
- Create: `src/hooks/useTransfer.ts`

- [ ] **Step 1 : Créer types.ts**

Créer `src/types.ts` :

```typescript
export interface Device {
  name: string;
  ip: string;
  port: number;
}

export interface TransferProgress {
  transfer_id: string;
  bytes_done: number;
  total: number;
  direction: 'sending' | 'receiving';
}

export interface IncomingRequest {
  transfer_id: string;
  sender_name: string;
  file_name: string;
  file_size: number;
}

export interface TransferComplete {
  transfer_id: string;
  file_name: string;
  saved_path: string;
}

export interface TransferFailed {
  transfer_id: string;
  reason: string;
}
```

- [ ] **Step 2 : Créer useDevices.ts**

Créer `src/hooks/useDevices.ts` :

```typescript
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Device } from '../types';

export function useDevices() {
  const [devices, setDevices] = useState<Device[]>([]);

  useEffect(() => {
    // Charger la liste initiale
    invoke<Device[]>('get_devices').then(setDevices).catch(console.error);

    // Écouter les updates
    const unlistenFound = listen<Device>('device-discovered', (event) => {
      setDevices((prev) => {
        if (prev.some((d) => d.name === event.payload.name)) return prev;
        return [...prev, event.payload];
      });
    });

    const unlistenLost = listen<string>('device-lost', (event) => {
      setDevices((prev) =>
        prev.filter((d) => !event.payload.includes(d.name))
      );
    });

    return () => {
      unlistenFound.then((f) => f());
      unlistenLost.then((f) => f());
    };
  }, []);

  return devices;
}
```

- [ ] **Step 3 : Créer useTransfer.ts**

Créer `src/hooks/useTransfer.ts` :

```typescript
import { useEffect, useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
  Device,
  IncomingRequest,
  TransferComplete,
  TransferFailed,
  TransferProgress,
} from '../types';

export interface ActiveTransfer {
  transferId: string;
  fileName: string;
  fileSize: number;
  bytesDone: number;
  direction: 'sending' | 'receiving';
}

export function useTransfer() {
  const [incoming, setIncoming] = useState<IncomingRequest | null>(null);
  const [activeTransfer, setActiveTransfer] = useState<ActiveTransfer | null>(null);
  const [lastComplete, setLastComplete] = useState<TransferComplete | null>(null);
  const [lastError, setLastError] = useState<string | null>(null);

  useEffect(() => {
    const unlistenIncoming = listen<IncomingRequest>('incoming-transfer', (e) => {
      setIncoming(e.payload);
    });

    const unlistenProgress = listen<TransferProgress>('transfer-progress', (e) => {
      const p = e.payload;
      setActiveTransfer({
        transferId: p.transfer_id,
        fileName: '',
        fileSize: p.total,
        bytesDone: p.bytes_done,
        direction: p.direction as 'sending' | 'receiving',
      });
    });

    const unlistenComplete = listen<TransferComplete>('transfer-complete', (e) => {
      setLastComplete(e.payload);
      setActiveTransfer(null);
      setIncoming(null);
    });

    const unlistenFailed = listen<TransferFailed>('transfer-failed', (e) => {
      setLastError(e.payload.reason);
      setActiveTransfer(null);
      setIncoming(null);
    });

    return () => {
      unlistenIncoming.then((f) => f());
      unlistenProgress.then((f) => f());
      unlistenComplete.then((f) => f());
      unlistenFailed.then((f) => f());
    };
  }, []);

  const sendFile = useCallback(async (device: Device, filePath: string) => {
    setLastError(null);
    await invoke('send_file_to_device', { device, filePath });
  }, []);

  const respondTransfer = useCallback(
    async (transferId: string, accepted: boolean) => {
      await invoke('respond_transfer', { transferId, accepted });
      if (!accepted) setIncoming(null);
    },
    []
  );

  return { incoming, activeTransfer, lastComplete, lastError, sendFile, respondTransfer };
}
```

- [ ] **Step 4 : Vérifier que TypeScript compile**

```bash
cd /Users/rso/Projects/droplink
npx tsc --noEmit
```

Expected : aucune erreur.

- [ ] **Step 5 : Commit**

```bash
git add src/types.ts src/hooks/
git commit -m "feat: TypeScript types + useDevices + useTransfer hooks"
```

---

### Task 7 : Composants DeviceList + DropZone

**Files:**
- Create: `src/components/DeviceList.tsx`
- Create: `src/components/DropZone.tsx`

- [ ] **Step 1 : Créer DeviceList.tsx**

Créer `src/components/DeviceList.tsx` :

```tsx
import type { Device } from '../types';

interface Props {
  devices: Device[];
  onDeviceSelect: (device: Device) => void;
  selectedDevice: Device | null;
}

export function DeviceList({ devices, onDeviceSelect, selectedDevice }: Props) {
  if (devices.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-8 text-gray-500 text-sm">
        <div className="w-8 h-8 border-2 border-gray-600 border-t-transparent rounded-full animate-spin mb-3" />
        Recherche d'appareils à proximité…
      </div>
    );
  }

  return (
    <ul className="space-y-1">
      {devices.map((device) => {
        const isSelected = selectedDevice?.name === device.name;
        return (
          <li key={device.name}>
            <button
              onClick={() => onDeviceSelect(device)}
              className={`w-full flex items-center gap-3 px-3 py-2 rounded-lg text-left transition-colors ${
                isSelected
                  ? 'bg-blue-600 text-white'
                  : 'hover:bg-gray-700 text-gray-200'
              }`}
            >
              <span className="w-2 h-2 rounded-full bg-green-400 shrink-0" />
              <span className="text-sm font-medium truncate">{device.name}</span>
            </button>
          </li>
        );
      })}
    </ul>
  );
}
```

- [ ] **Step 2 : Créer DropZone.tsx**

Créer `src/components/DropZone.tsx` :

```tsx
import { useState, useCallback } from 'react';
import type { Device } from '../types';

interface Props {
  selectedDevice: Device | null;
  onFilesDropped: (device: Device, filePaths: string[]) => void;
  disabled?: boolean;
}

export function DropZone({ selectedDevice, onFilesDropped, disabled }: Props) {
  const [isDragging, setIsDragging] = useState(false);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    if (!disabled) setIsDragging(true);
  }, [disabled]);

  const handleDragLeave = useCallback(() => {
    setIsDragging(false);
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      setIsDragging(false);
      if (!selectedDevice || disabled) return;

      // Tauri expose les chemins absolus via dataTransfer
      const paths: string[] = [];
      for (const file of Array.from(e.dataTransfer.files)) {
        // @ts-expect-error — Tauri étend File avec path
        if (file.path) paths.push(file.path);
      }
      if (paths.length > 0) {
        onFilesDropped(selectedDevice, paths);
      }
    },
    [selectedDevice, onFilesDropped, disabled]
  );

  const active = !disabled && selectedDevice;

  return (
    <div
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
      className={`
        flex flex-col items-center justify-center rounded-xl border-2 border-dashed
        py-8 px-4 transition-colors text-center select-none
        ${isDragging ? 'border-blue-400 bg-blue-950/30' : 'border-gray-600'}
        ${active ? 'cursor-copy' : 'opacity-40 cursor-not-allowed'}
      `}
    >
      <div className="text-3xl mb-2">📂</div>
      {selectedDevice ? (
        <>
          <p className="text-sm text-gray-300">
            Glisse ici pour envoyer à
          </p>
          <p className="text-sm font-semibold text-white mt-1">
            {selectedDevice.name}
          </p>
        </>
      ) : (
        <p className="text-sm text-gray-500">
          Sélectionne un appareil ci-dessus
        </p>
      )}
    </div>
  );
}
```

- [ ] **Step 3 : Vérifier TypeScript**

```bash
npx tsc --noEmit
```

Expected : aucune erreur.

- [ ] **Step 4 : Commit**

```bash
git add src/components/DeviceList.tsx src/components/DropZone.tsx
git commit -m "feat: DeviceList + DropZone components"
```

---

### Task 8 : Composants TransferModal + AcceptPopup

**Files:**
- Create: `src/components/TransferModal.tsx`
- Create: `src/components/AcceptPopup.tsx`

- [ ] **Step 1 : Créer TransferModal.tsx**

Créer `src/components/TransferModal.tsx` :

```tsx
import type { ActiveTransfer } from '../hooks/useTransfer';

interface Props {
  transfer: ActiveTransfer;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function TransferModal({ transfer }: Props) {
  const pct = transfer.fileSize > 0
    ? Math.round((transfer.bytesDone / transfer.fileSize) * 100)
    : 0;

  const label = transfer.direction === 'sending' ? 'Envoi en cours' : 'Réception en cours';

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div className="bg-gray-800 rounded-2xl p-6 w-72 shadow-2xl">
        <p className="text-xs text-gray-400 uppercase tracking-wider mb-1">{label}</p>
        <p className="text-sm font-medium text-white mb-4 truncate">
          {transfer.fileName || 'Fichier en cours…'}
        </p>

        <div className="w-full bg-gray-700 rounded-full h-2 mb-2">
          <div
            className="bg-blue-500 h-2 rounded-full transition-all duration-100"
            style={{ width: `${pct}%` }}
          />
        </div>

        <div className="flex justify-between text-xs text-gray-400">
          <span>{formatBytes(transfer.bytesDone)}</span>
          <span>{pct}%</span>
          <span>{formatBytes(transfer.fileSize)}</span>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2 : Créer AcceptPopup.tsx**

Créer `src/components/AcceptPopup.tsx` :

```tsx
import type { IncomingRequest } from '../types';

interface Props {
  request: IncomingRequest;
  onAccept: () => void;
  onRefuse: () => void;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function AcceptPopup({ request, onAccept, onRefuse }: Props) {
  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div className="bg-gray-800 rounded-2xl p-6 w-72 shadow-2xl">
        <p className="text-sm font-semibold text-white mb-1">
          {request.sender_name}
        </p>
        <p className="text-xs text-gray-400 mb-4">
          veut t'envoyer un fichier
        </p>

        <div className="bg-gray-700 rounded-lg px-3 py-2 mb-5">
          <p className="text-sm text-white truncate">{request.file_name}</p>
          <p className="text-xs text-gray-400 mt-0.5">
            {formatBytes(request.file_size)}
          </p>
        </div>

        <div className="flex gap-3">
          <button
            onClick={onRefuse}
            className="flex-1 py-2 rounded-lg bg-gray-700 hover:bg-gray-600 text-sm text-gray-300 transition-colors"
          >
            Refuser
          </button>
          <button
            onClick={onAccept}
            className="flex-1 py-2 rounded-lg bg-blue-600 hover:bg-blue-500 text-sm text-white font-medium transition-colors"
          >
            Accepter
          </button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 3 : Vérifier TypeScript**

```bash
npx tsc --noEmit
```

Expected : aucune erreur.

- [ ] **Step 4 : Commit**

```bash
git add src/components/TransferModal.tsx src/components/AcceptPopup.tsx
git commit -m "feat: TransferModal + AcceptPopup components"
```

---

### Task 9 : App.tsx — assemblage final

**Files:**
- Modify: `src/App.tsx` (remplacer entièrement)

- [ ] **Step 1 : Réécrire App.tsx**

Remplacer `src/App.tsx` entièrement :

```tsx
import { useState, useCallback } from 'react';
import { useDevices } from './hooks/useDevices';
import { useTransfer } from './hooks/useTransfer';
import { DeviceList } from './components/DeviceList';
import { DropZone } from './components/DropZone';
import { TransferModal } from './components/TransferModal';
import { AcceptPopup } from './components/AcceptPopup';
import type { Device } from './types';

export default function App() {
  const devices = useDevices();
  const { incoming, activeTransfer, lastComplete, lastError, sendFile, respondTransfer } =
    useTransfer();
  const [selectedDevice, setSelectedDevice] = useState<Device | null>(null);

  const handleFilesDropped = useCallback(
    (device: Device, filePaths: string[]) => {
      for (const path of filePaths) {
        sendFile(device, path).catch(console.error);
      }
    },
    [sendFile]
  );

  return (
    <div className="bg-gray-900 text-white min-h-screen flex flex-col select-none">
      {/* Header */}
      <header className="flex items-center gap-2 px-4 py-3 border-b border-gray-800">
        <span className="text-lg font-bold tracking-tight">DropLink</span>
        <span className="ml-auto text-xs text-gray-500">
          {devices.length} appareil{devices.length !== 1 ? 's' : ''}
        </span>
      </header>

      {/* Body */}
      <main className="flex-1 flex flex-col gap-4 p-4 overflow-y-auto">
        <section>
          <p className="text-xs text-gray-500 uppercase tracking-wider mb-2">
            À proximité
          </p>
          <DeviceList
            devices={devices}
            selectedDevice={selectedDevice}
            onDeviceSelect={setSelectedDevice}
          />
        </section>

        <DropZone
          selectedDevice={selectedDevice}
          onFilesDropped={handleFilesDropped}
          disabled={!!activeTransfer}
        />

        {lastComplete && (
          <div className="rounded-lg bg-green-900/40 border border-green-700 px-3 py-2 text-sm text-green-300">
            ✓ {lastComplete.file_name} reçu dans ~/Downloads/DropLink
          </div>
        )}

        {lastError && (
          <div className="rounded-lg bg-red-900/40 border border-red-700 px-3 py-2 text-sm text-red-300">
            ✗ {lastError}
          </div>
        )}
      </main>

      {/* Modals */}
      {activeTransfer && <TransferModal transfer={activeTransfer} />}

      {incoming && !activeTransfer && (
        <AcceptPopup
          request={incoming}
          onAccept={() => respondTransfer(incoming.transfer_id, true)}
          onRefuse={() => respondTransfer(incoming.transfer_id, false)}
        />
      )}
    </div>
  );
}
```

- [ ] **Step 2 : Lancer l'app en mode dev**

```bash
cd /Users/rso/Projects/droplink
npm run tauri dev
```

Expected : fenêtre DropLink s'ouvre, header visible, spinner "Recherche d'appareils à proximité" actif.

- [ ] **Step 3 : Test manuel — découverte**

Lancer une deuxième instance de l'app sur le même réseau (autre machine ou VM). Vérifier que les deux appareils s'affichent mutuellement dans la DeviceList.

- [ ] **Step 4 : Test manuel — transfert complet**

1. Sélectionner l'autre appareil dans la liste
2. Glisser un fichier <5MB dans la DropZone
3. Sur l'autre machine : AcceptPopup s'affiche → cliquer Accepter
4. TransferModal progress → fichier apparaît dans `~/Downloads/DropLink/`

- [ ] **Step 5 : Commit**

```bash
git add src/App.tsx
git commit -m "feat: App.tsx — assemble full DropLink UI"
```

---

### Task 10 : Build release + vérification finale

**Files:**
- Modify: `src-tauri/tauri.conf.json` (icône, nom app)

- [ ] **Step 1 : Configurer tauri.conf.json**

Ouvrir `src-tauri/tauri.conf.json`, modifier :

```json
{
  "productName": "DropLink",
  "version": "0.1.0",
  "identifier": "com.droplink.app",
  "app": {
    "windows": [
      {
        "title": "DropLink",
        "width": 320,
        "height": 500,
        "resizable": false,
        "decorations": true
      }
    ]
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/icon.icns", "icons/icon.ico"]
  }
}
```

- [ ] **Step 2 : Lancer tous les tests Rust**

```bash
cd /Users/rso/Projects/droplink/src-tauri
cargo test 2>&1
```

Expected : tous les tests passent (`protocol::tests`, `discovery::tests`, `transfer::tests`).

- [ ] **Step 3 : Build TypeScript**

```bash
cd /Users/rso/Projects/droplink
npx tsc --noEmit
```

Expected : aucune erreur.

- [ ] **Step 4 : Build release**

```bash
npm run tauri build
```

Expected : binaire généré dans `src-tauri/target/release/drop-link`.

- [ ] **Step 5 : Commit final**

```bash
cd /Users/rso/Projects/droplink
git add src-tauri/tauri.conf.json
git commit -m "feat: release config — DropLink v0.1.0 core complete"
```

---

## Hors scope de ce plan (Plan 2)

- Intégration right-click OS : Finder Extension macOS, shell extension Windows, script Nautilus Linux
- Mode invisible (désactiver mDNS advertising)
- Historique des transferts
- Support mobile (iOS/Android) — v2
- Chiffrement TLS
