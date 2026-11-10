# Journal des Modifications — DropLink

Tous les changements notables de ce projet sont documentés dans ce fichier.

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/).

---

## [1.0.0] — 2026-11-10

### Ajouté
- Workspace Cargo `droplink-core` / `droplink-net` / `droplink-app` / `droplink-relay`.
- UI native egui/eframe : drag & drop OS natif, sélecteur de fichier, file de transferts, popup d'acceptation
  avec code SAS, historique persistant, réglages.
- Protocole v2 : frames length-prefixed versionnées, contrôle scellé, ACK final `Done`.
- Chiffrement bout-en-bout X25519 + HKDF-SHA256 + ChaCha20-Poly1305, clé et compteur de nonce par direction.
- Code SAS à 6 chiffres dérivé du transcript, affiché avant acceptation.
- BLAKE3 en streaming (une seule lecture disque), publication `.part` + `hard_link` atomique.
- Découverte mDNS robuste : sélection d'interface, identité `device_id`, gestion des homonymes et des pertes.
- Relay de circuits réel : frames binaires opaques, rooms, limites dures, heartbeat, reconnexion avec backoff.
- Transports unifiés direct → relay avec bascule automatique.
- Tests : 130 tests (unitaires, intégration directe, relay, mDNS réel, anti-fuite, UI state, smoke).
- CI 3 OS (fmt, clippy -D warnings, tests, build release) et workflow de release.

### Corrigé
- Drag & drop cassé (Tauri v2 ne fournit plus `File.path`).
- Relay qui listait les pairs sans jamais transférer.
- « Transfert terminé » affiché sans confirmation du récepteur.
- Débit réseau couplé au renderer (un événement IPC par chunk).
- Multi-fichiers écrasés dans une seule modale.
- Identité incohérente entre registre (`ip:port`) et UI (nom).
- Découverte morte sur un LAN sans internet (`get_local_ip` → `127.0.0.1`).
- TOCTOU sur le port et sur les noms de fichiers.
- Connexions entrantes non bornées.

### Retiré
- Stack Tauri/React/TypeScript, `node_modules`, WebView, dépendance `bluest` inutilisée.

---

## [0.1.0] — 2026-05-21

### Ajouté

#### Scaffold & Infrastructure Tauri
- Scaffold Tauri 2 + React 18 + TypeScript strict + Tailwind CSS v4
- Configuration release : fenêtre 320×500 non-resizable, identifier `com.droplink.app`
- Build release : DropLink.app + DropLink_0.1.0_aarch64.dmg générés automatiquement
- Vite config avec Tauri plugin

#### Backend Rust — Protocole & Découverte

**protocol.rs** — Sérialisation
- Types `DeviceInfo` (device_id, name, address, port) sérialisés JSON
- Enum `Message` : Hello, Accept, Refuse, Error avec payload
- Traits Serialize/Deserialize pour tous les types

**discovery.rs** — mDNS & Port
- Advertiseur mDNS avec service type `_droplink._tcp.local`
- Browseur mDNS pour découvrir pairs automatiquement
- `find_free_port()` : scan ports 7777–7800 avec fallback 7777
- `get_local_ip()` : résolution adresse IPv4 locale (IPv6 filtré)
- Intégration avec `mdns-sd` crate

#### Backend Rust — Transfert TCP

**transfer.rs** — Moteur de transfert
- Moteur TCP chunked : chunks 64 KB, lecture disque efficace
- Vérification intégrité : SHA-256 checksum calcul + validation
- `PendingMap` : HashMap oneshot channels timeout-safe
- `sanitize_filename()` : suppression séparateurs chemin + null bytes, préservation Unicode
- `unique_path()` : évite écrasement fichiers existants (suffixe `_1`, `_2`, etc.)
- Limite fichier : rejet si > 10 GB
- Gestion erreurs : Send (TCP + I/O) + Receive (TCP + I/O + Checksum)

#### Backend Rust — Système Tray & Commands

**tray.rs** — System Tray
- System tray avec icône
- Menu : "Afficher" → focus fenêtre, "Quitter" → exit app

**main.rs** — AppState & Tauri Commands
- `AppState` : devices Arc<Mutex>, pending_map Arc<Mutex>, tx pour mDNS events
- 3 commandes Tauri exposées :
  - `get_devices` → liste devices actuels
  - `send_file_to_device` → initie transfert TCP
  - `respond_transfer` → accepte/refuse incoming
- Wiring : mDNS advertiser + browserthread + TCP listener sur port libre
- Event streams : `device-discovered`, `device-lost`, `transfer-request`, `transfer-failed`
- Tokio runtime multi-thread

#### Frontend TypeScript — Types & Hooks

**types.ts** — Miroir Rust
- Interfaces TypeScript `DeviceInfo`, `Message`, `TransferState`
- Types pour tous les Tauri command payloads
- Enum-like types TransferStatus : pending | transferring | completed | failed

**useDevices.ts** — Gestion appareil
- Hook custom : sync devices depuis events Tauri
- State : devices[], selectedDevice, isLoading
- Effects : setup listeners `device-discovered`, `device-lost`
- Cleanup : removeListener au unmount
- Réconciliation : éviction device si disparu

**useTransfer.ts** — Orchestration transfert
- Hook custom : état transfert (status, progress %, error)
- `sendFile(device, file)` : invoke `send_file_to_device` + ref filename + progress events
- `respondTransfer(accepted)` : invoke `respond_transfer`
- Error handling : catch rejections, set lastError
- Reset : success banner auto-dismissed après 3s, error persiste

#### Frontend React — Composants

**DeviceList.tsx** — Liste appareils
- Affichage liste devices avec `<select>`
- Spinner pendant isLoading
- Disabled si pas de device

**DropZone.tsx** — Drag & Drop
- Zone draggable (border pointillé quand actif)
- État drag : dragCounter useRef (évite flicker sur enfants)
- Input file caché
- Limitation : fichiers < 10 GB
- Aria-label pour accessibilité

**TransferModal.tsx** — Barre progression
- Modal affichage pendant transfert
- Barre progression : % temps réel
- Affichage nom fichier en cours
- État : "Transfert en cours..." → success → error
- Fermeture auto après succès (3s)

**AcceptPopup.tsx** — Pop-up incoming
- Affichage : "Appareil X envoie Fichier Y"
- Boutons : Accepter / Refuser
- Fermeture après réponse

**App.tsx** — Assemblage
- Composant principal : DeviceList + DropZone + TransferModal + AcceptPopup + Banner
- Layout : flex column, centré, 640px max-width
- Banner succès/erreur en haut
- Hook useDevices + useTransfer pour orchestration
- States affichés : idle / dropping / transferring / completed

#### Styling & UX

- Tailwind CSS v4 : spacing cohérent (rem), couleurs neutrales
- Responsive : mobile-first
- Transitions smooth : opacity, scale
- Icones : SVG inline (pas de dépendances)
- Focus states : keyboard navigation accessible
- Error messages : en français, clairs

### Corrigé

#### Backend Rust

**Sérialisation**
- `Message` enum manquait `PartialEq` et `Clone` dans le derive → ajout automatique

**Découverte mDNS**
- Adresse IPv6 non-déterministe depuis `HashSet` mDNS → TCP connect failures → filtre IPv4 forcé dans `get_local_ip()`
- `device-lost` match substring → éviction incorrecte de devices similaires → remplacé par correspondance exacte

**Transfert TCP**
- Race condition : `IncomingRequest` event émis avant insertion dans `pending_map` → accept ignoré → synchronisation avec mutex avant emit
- Erreur d'écriture disque silencieuse (`let _ = file.write_all(...)`) → fichier corrompu → ajout ? operator + propagation erreur
- Fuite mémoire : entrée `pending_map` jamais retirée en cas de timeout → tokio::timeout avec explicit remove
- Erreur côté émetteur (Refused, Send error) non propagée au frontend → emit `transfer-failed` event
- Limite fichier manquante → DoS potentiel → rejet si > 10 GB avant popup
- `sanitize_filename()` remplaçait tous caractères non-ASCII → noms Unicode brisés → filtre limité aux séparateurs chemin et null bytes

#### Frontend TypeScript & React

**Hook useTransfer**
- `currentFileName` closure locale à `useEffect` → inaccessible depuis `sendFile()` → TransferModal affichait "Transfert en cours..." générique → remplacé par `currentFileNameRef` (useRef)
- `lastError` non remis à null lors d'un transfert réussi → vieux messages d'erreur persistaient → ajout setLastError(null) dans sendFile start
- `selectedDevice` non invalidé quand peer disparaît → UI sélectionnait device fantôme → ajout useEffect réconciliation (filtrer devices non existants)
- Erreurs send-side dans spawn tokio non propagées → TransferModal restait bloqué indéfiniment → emit `Failed` si `send_file` retourne Err

**Composant DropZone**
- `setIsDragging(true)` dans `onDragOver` → re-render à chaque pixel → déplacé dans `onDragEnter`
- `onDragLeave` sur éléments enfants → flicker drag state → dragCounter useRef + vérification counter === 0 avant reset

**Composant TransferModal**
- Banner succès affichait "reçu dans ~/Downloads/DropLink" même pour l'émetteur → neutralisé en "Transfert terminé"
- Modal restait ouvert après erreur → fermeture après 5s ou Escape
- Progress % restait stale après completion → reset à 0 pour prochain transfert

**Erreur handling global**
- Erreurs `invoke` swallowées dans `.catch(console.error)` → utilisateur pas informé → try/catch explicite + setLastError + affichage Banner

---

## Hors scope v0.1.0 — Prévu v0.2+

### Sécurité
- [ ] TLS pour transferts TCP (actuellement plaintext + checksum intégrité)
- [ ] Authentification peer (device pairing, token)
- [ ] Chiffrement fichier (AES-256)

### Découverte & Réseau
- [ ] Transfert multi-réseau (LAN + VPN détection auto)
- [ ] Transfert P2P (holes punching, relay fallback)
- [ ] QR code pair code

### Performance & Résiliency
- [ ] Multipart upload (reprendre transfert interrompu)
- [ ] Compression (optionnelle, user toggle)
- [ ] Bandwith limiting (QoS)

### UI/UX
- [ ] Historique transferts
- [ ] Préférences app (dossier destination custom, notifications)
- [ ] Dark mode toggle
- [ ] Animations + micro-interactions polish
- [ ] Support drag & drop multiple fichiers simultanés

### Plateformes
- [ ] Support Linux (actuellement macOS seul)
- [ ] Support Windows (actuellement macOS seul)
- [ ] Android/iOS companion app

### Intégrations
- [ ] Webhook (notification transfert complété)
- [ ] API REST pour contrôle headless
- [ ] Web interface (browser-based)

---

**Statut de release :** MVP stable — cœur fonctionnel (découverte + transfert + UI) prêt production. À auditer avant usage multi-utilisateurs.
