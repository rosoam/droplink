# Architecture DropLink v0.1.0

Documentation technique de l'architecture interne de DropLink — transfert de fichiers P2P sur LAN, sans serveur central.

---

## 1. Vue d'ensemble

```
┌─────────────────────────────────────────────────────────────┐
│                    UI React (TypeScript)                     │
│   DeviceList │ DropZone │ TransferModal │ AcceptPopup        │
│   useDevices()          useTransfer()                        │
└───────────────────────┬─────────────────────────────────────┘
                        │  invoke() / listen()
                        │  Tauri IPC Bridge
                        │  (commandes + événements)
┌───────────────────────┴─────────────────────────────────────┐
│                      main.rs — AppState                      │
│   get_devices  │  send_file_to_device  │  respond_transfer   │
└──────────┬─────────────────┬───────────────────────────────-┘
           │                 │
    ┌──────┴──────┐   ┌──────┴──────┐
    │ discovery.rs│   │ transfer.rs │
    │  mDNS + UDP │   │ TCP server  │
    └──────┬──────┘   └──────┬──────┘
           │                 │
           └────────┬────────┘
                    │
         ┌──────────┴──────────┐
         │   protocol.rs       │
         │   DeviceInfo        │
         │   Message (enum)    │
         └──────────┬──────────┘
                    │
         ┌──────────┴──────────┐
         │      LAN / TCP      │
         │   port 7777–7800    │
         └─────────────────────┘
```

**3 couches :**

| Couche | Technologie | Rôle |
|--------|-------------|------|
| UI | React 18 / TypeScript | Présentation, interaction utilisateur |
| Tauri IPC bridge | Tauri 2 | Sérialisation des commandes et événements entre UI et Rust |
| Moteurs Rust | `discovery.rs`, `transfer.rs`, `protocol.rs` | Découverte des pairs, transfert TCP, protocole réseau |

---

## 2. Module `protocol.rs`

Définit les types partagés entre les modules Rust et sérialisés sur le réseau.

### `DeviceInfo`

```rust
struct DeviceInfo {
    name: String,
    ip:   String,
    port: u16,
}
```

Utilisé dans `AppState`, les événements de découverte et les commandes Tauri. Struct de référence unique pour représenter un pair.

### `Message` — enum du protocole réseau

```rust
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
enum Message {
    Hello {
        sender_name: String,
        file_name:   String,
        file_size:   u64,
        checksum:    String,
        transfer_id: String,
    },
    Accept,
    Refuse,
    Error { reason: String },
}
```

**Sérialisation JSON :**

```json
{ "type": "HELLO", "sender_name": "Alice", "file_name": "rapport.pdf", "file_size": 1048576, "checksum": "abc123...", "transfer_id": "uuid-v4" }
{ "type": "ACCEPT" }
{ "type": "REFUSE" }
{ "type": "ERROR", "reason": "file too large" }
```

Le tag `type` en `SCREAMING_SNAKE_CASE` est le discriminant sur le fil.

---

## 3. Module `discovery.rs`

Gère la présence réseau locale et la découverte des autres instances DropLink.

### Initialisation

```
Discovery::new(device_name, local_ip, port)
    └── enregistre le service mDNS : _droplink._tcp.local.
```

### Découverte des pairs

```
start_browsing(tx: mpsc::Sender<DiscoveryEvent>)
    ├── écoute ServiceResolved → DiscoveryEvent::DeviceFound(DeviceInfo)
    └── écoute ServiceRemoved → DiscoveryEvent::DeviceGone(name)
```

**Filtre IPv4 :** les adresses IPv6 sont exclues — le `HashSet` sur lequel s'appuie mDNS est non-déterministe pour IPv6 (ordre imprévisible, doublons possibles).

### Utilitaires réseau

**`find_free_port(7777..7800)`**

```
for port in 7777..=7800 {
    TcpListener::bind(("0.0.0.0", port)) → OK → retourner port
}
```

**`get_local_ip()`**

```
UdpSocket::connect("8.8.8.8:80")  // pas de paquet envoyé
→ socket.local_addr().ip()         // OS renseigne l'IP source
```

Technique UDP socket trick : le système d'exploitation renseigne l'adresse locale sans envoyer de paquet sur le réseau.

---

## 4. Module `transfer.rs`

Gère le serveur TCP entrant et l'envoi de fichiers.

### Coordination accept/refuse

```rust
type PendingMap = Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>;
```

Chaque transfert entrant est représenté par un `transfer_id` → un channel `oneshot`. La moitié `Sender` est conservée dans la map jusqu'à ce que le frontend réponde via `respond_transfer`.

### Serveur TCP

```
TransferServer::new(port, event_tx)
    └── start()
            └── boucle TcpListener::accept
                    ├── spawn task → handle_incoming(stream)
                    └── handle_incoming :
                            ├── lire HELLO JSON
                            ├── pending.insert(transfer_id, tx)
                            ├── emit IncomingRequest → frontend
                            ├── attendre oneshot (réponse UI)
                            ├── envoyer ACCEPT ou REFUSE JSON
                            └── si ACCEPT : recevoir chunks → vérifier SHA-256
```

### Envoi de fichier — two-pass

```
send_file(device, file_path, event_tx, device_name)
    ├── Pass 1 : lire le fichier entier → calculer SHA-256
    ├── TCP connect → envoyer HELLO JSON
    ├── lire réponse : ACCEPT ou REFUSE
    └── si ACCEPT :
            Pass 2 : lire + envoyer par chunks de 64 KB
                     émettre Progress à chaque chunk
            → fin : émettre Complete
```

### Streaming

```
chunk_size = 64 KB
for chunk in file {
    stream.write_all(chunk)
    sha256_hasher.update(chunk)
    emit Progress { bytes_done, file_size }
}
// récepteur vérifie : sha256_final == checksum du HELLO
```

### Sécurité des noms de fichier

**`sanitize_filename(name)`**
- Supprime `/`, `\`, `\0`
- Supprime les points initiaux (prévient les fichiers cachés)
- Préserve l'unicode (noms non-ASCII valides)

**`unique_path(base_path)`**
- Si le chemin existe : ajoute `_1`, `_2`, … jusqu'à trouver un chemin libre

### `TransferEvent` — événements vers le frontend

| Variant | Payload | Déclencheur |
|---------|---------|-------------|
| `IncomingRequest` | `sender_name`, `file_name`, `file_size`, `transfer_id` | Réception d'un HELLO valide |
| `Progress` | `bytes_done`, `file_size`, `transfer_id` | À chaque chunk reçu ou envoyé |
| `Complete` | `transfer_id`, `file_name` | Checksum vérifié |
| `Failed` | `transfer_id`, `reason` | Erreur réseau, checksum KO, refus |

---

## 5. Module `tray.rs`

Icône système (system tray) pour accéder à l'app sans la garder au premier plan.

| Action | Comportement |
|--------|-------------|
| `MenuItem` "Afficher DropLink" | `show_window` — ramène la fenêtre principale |
| `MenuItem` "Quitter" | `app.exit(0)` |
| Double-clic sur l'icône tray | `show_window` |

---

## 6. `main.rs` — orchestration

### `AppState`

```rust
struct AppState {
    devices:           Arc<Mutex<Vec<DeviceInfo>>>,
    pending_transfers: PendingMap,
    device_name:       String,
    event_tx:          mpsc::Sender<TransferEvent>,
}
```

### Séquence d'initialisation

```
1. get_local_ip()                    → String
2. find_free_port(7777..7800)        → u16
3. TransferServer::start(port, tx)   → spawn task
4. Discovery::new(name, ip, port)    → enregistre mDNS
5. start_browsing(disc_tx)           → spawn task
6. Tauri Builder::run(AppState, ...)
```

### Boucles async

**Boucle 1 — événements de découverte**
```
disc_rx.recv() → DeviceFound  → state.devices.push(device)
              → DeviceLost   → state.devices.retain(|d| d.name != name)
```

**Boucle 2 — événements de transfert → Tauri emit**
```
transfer_event_rx.recv()
    → IncomingRequest → window.emit("incoming-transfer", payload)
    → Progress        → window.emit("transfer-progress", payload)
    → Complete        → window.emit("transfer-complete", payload)
    → Failed          → window.emit("transfer-failed", payload)
```

### Commandes Tauri

| Commande | Signature | Description |
|----------|-----------|-------------|
| `get_devices` | `() → Vec<DeviceInfo>` | Retourne la liste courante des pairs découverts |
| `send_file_to_device` | `(device: DeviceInfo, file_path: String) → Result<()>` | Spawn `send_file` en tâche async |
| `respond_transfer` | `(transfer_id: String, accepted: bool) → Result<()>` | Résout le oneshot channel du transfert en attente |

---

## 7. Frontend — hooks

### `useDevices()`

```
mount → invoke('get_devices') → setDevices(result)

listen('device-discovered') → setDevices(prev => [...prev, device])
listen('device-lost')       → setDevices(prev => prev.filter(d => d.name !== name))
```

### `useTransfer()`

```
listen('incoming-transfer')  → setIncoming(payload) → affiche AcceptPopup
listen('transfer-progress')  → setProgress({ bytesDone, fileSize })
listen('transfer-complete')  → setLastComplete(payload); resetProgress()
listen('transfer-failed')    → setLastError(reason); resetProgress()

sendFile(device, filePath):
    currentFileNameRef.current = basename(filePath)  // partagé avec le listener incoming
    invoke('send_file_to_device', { device, filePath })
    .catch(err => setLastError(err))

respondTransfer(transferId, accepted):
    invoke('respond_transfer', { transferId, accepted })
    if (!accepted) clearIncoming()
```

**`currentFileNameRef`** est un `useRef` partagé entre l'envoyeur (`sendFile`) et le listener `incoming-transfer` côté récepteur. Cela permet au `TransferModal` d'afficher le nom du fichier dans les deux directions sans re-rendu supplémentaire.

---

## 8. Frontend — composants

### `DeviceList`

- Affiche un spinner si la liste est vide (découverte en cours)
- Clic sur un pair → `setSelectedDevice(device)`
- Pair sélectionné : surbrillance bleue
- Reçoit `devices: DeviceInfo[]` et `selectedDevice` en props

### `DropZone`

```
dragCounter ref (number)
    onDragEnter → dragCounter++; setDragOver(true)
    onDragLeave → dragCounter--; if (0) setDragOver(false)
    onDrop      → dragCounter = 0; handleFilesDropped(files)
```

Le compteur évite le flicker lorsque le curseur glisse sur un élément enfant (l'événement `dragleave` est déclenché même en restant dans la zone).

Prop `disabled` activé pendant un transfert en cours — empêche de lancer un second envoi simultané.

### `TransferModal`

```
<progress value={bytesDone} max={fileSize} />
{formatBytes(bytesDone)} / {formatBytes(fileSize)}
Direction : "Envoi" ou "Réception" selon context
```

`formatBytes` : conversion automatique en B / KB / MB / GB.

### `AcceptPopup`

```
Fichier : {fileName}   Taille : {formatBytes(fileSize)}
Envoyé par : {senderName}

[Refuser]   [Accepter]
```

Bouton Accepter → `respondTransfer(transferId, true)`
Bouton Refuser → `respondTransfer(transferId, false)`

---

## 9. Flux complet — envoi d'un fichier

```
Expéditeur (Alice)                          Destinataire (Bob)
──────────────────                          ──────────────────

1. Glisser rapport.pdf sur DropZone
   → handleFilesDropped(["rapport.pdf"])
   → sendFile(bobDevice, "/path/rapport.pdf")

2. currentFileNameRef.current = "rapport.pdf"

3. invoke('send_file_to_device')
   → main.rs spawn → send_file()

4. Pass 1 : SHA-256 de rapport.pdf
   TCP connect → HELLO JSON ──────────────────────────────────▶

                                          5. handle_incoming() lit HELLO
                                             pending.insert(transfer_id, tx)
                                             emit "incoming-transfer" ──▶ UI

                                          6. AcceptPopup s'affiche :
                                             "rapport.pdf (1.2 MB) — Alice"

                                          7. Bob clique Accepter
                                             respondTransfer(transfer_id, true)
                                             invoke('respond_transfer')
                                             → oneshot.send(true)

                                ◀── ACCEPT JSON ─────────────────

8. Sender reçoit ACCEPT
   Pass 2 : stream 64 KB chunks
   → emit Progress à chaque chunk ─────────────────────────────▶
                                          → emit Progress → TransferModal

9. Tous les chunks envoyés                9. Écriture chunks terminée
                                             SHA-256 accumulé == checksum HELLO ?
                                             ✓ → emit "transfer-complete" ──▶ UI

                                          10. Banner "Transfert terminé"
                                              fichier sauvegardé (unique_path)

10. emit "transfer-complete" ──▶ UI
    Banner "Transfert terminé"
```

---

## 10. Sécurité

### Path traversal

`sanitize_filename` appliqué à chaque `file_name` reçu dans un HELLO avant toute opération disque :

- Supprime `/` et `\` (séparateurs de chemin)
- Supprime `\0` (null byte)
- Supprime les points initiaux (empêche la création de fichiers cachés comme `.bashrc`)
- Préserve les caractères unicode valides

### Limite de taille

Avant d'afficher l'`AcceptPopup`, `file_size` est vérifié :

```
si file_size > 10 GB → émettre Failed("file too large") sans présenter le popup
```

Évite les attaques par épuisement de disque ou de mémoire avant toute décision utilisateur.

### Transport

- **v0.1 :** pas de chiffrement (TLS hors scope)
- **v0.2 (prévu) :** TLS mutuel sur le canal TCP, vérification d'empreinte de pair

### Intégrité

Le checksum SHA-256 calculé en two-pass côté expéditeur est vérifié par le récepteur après l'assemblage complet. Tout écart déclenche un `TransferEvent::Failed` et la suppression du fichier partiel.
