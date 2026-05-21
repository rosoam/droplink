# DropLink

![Version](https://img.shields.io/badge/version-0.1.0-blue)
![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey)
![License](https://img.shields.io/badge/license-MIT-green)

Transfert de fichiers P2P sur réseau local — AirDrop pour tout le monde. Sans compte, sans serveur, sans internet.

---

## Utilisation

![Screenshot placeholder](docs/screenshot-placeholder.png)

1. Ouvrir DropLink sur deux machines connectées au même réseau Wi-Fi ou Ethernet
2. Les appareils apparaissent automatiquement dans la liste
3. Glisser un fichier sur l'appareil cible
4. L'autre machine reçoit un popup Accept / Refuser
5. Le fichier arrive dans `~/Downloads/DropLink/`

---

## Fonctionnement

```
Appareil A                          Appareil B
─────────                           ─────────
mDNS advertise ─── LAN ──────────▶  mDNS browse
                                     (apparaît dans la liste)

[glisser fichier sur B]

TCP connect ──────────────────────▶  TCP server (port 7777–7800)
Send Hello (DeviceInfo + metadata)
                  ◀─────────────────  Popup Accept / Refuser
                  
[si accepté]
Chunks 64KB ──────────────────────▶  Reçoit chunks
                                     SHA-256 verify
                                     Sauvegarde ~/Downloads/DropLink/
Progress bar temps réel ◀──────────  Events Tauri → frontend
```

**Découverte** : mDNS / DNS-SD sur le LAN — aucun routeur ou serveur externe requis.  
**Transfert** : TCP chunked (64 KB / chunk) + vérification SHA-256 en fin de transfert.  
**Sauvegarde** : `~/Downloads/DropLink/<nom_fichier>` sur la machine réceptrice.

---

## Stack

| Couche | Technologie |
|--------|------------|
| Shell desktop | Tauri 2 |
| Backend Rust | tokio, mdns-sd, sha2, serde_json, uuid, dirs-next, hostname |
| Frontend | React 18 + TypeScript strict |
| Style | Tailwind CSS v4 |
| Découverte réseau | mDNS / DNS-SD |
| Transfert | TCP chunked 64KB + SHA-256 |

---

## Structure du projet

```
droplink/
├── src-tauri/
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs          — entry point, AppState, 3 commandes Tauri
│       ├── protocol.rs      — DeviceInfo, Message enum (Hello/Accept/Refuse/Error)
│       ├── discovery.rs     — mDNS advertising + browsing, find_free_port, get_local_ip
│       ├── transfer.rs      — TCP server/client, chunking 64KB, SHA-256, PendingMap
│       └── tray.rs          — system tray (Afficher / Quitter)
├── src/
│   ├── types.ts             — interfaces TypeScript miroir des types Rust
│   ├── App.tsx              — assemblage UI principal
│   ├── components/
│   │   ├── DeviceList.tsx   — liste appareils à proximité
│   │   ├── DropZone.tsx     — zone drag & drop
│   │   ├── TransferModal.tsx — progress bar transfert actif
│   │   └── AcceptPopup.tsx  — popup accept/refuser transfert entrant
│   └── hooks/
│       ├── useDevices.ts    — sync appareils via events Tauri
│       └── useTransfer.ts   — orchestration transferts, sendFile, respondTransfer
├── docs/
│   ├── ARCHITECTURE.md
│   └── CHANGELOG.md
└── tauri.conf.json          — fenêtre 320×500 non-resizable, identifier com.droplink.app
```

---

## Commandes de développement

```bash
# Frontend seulement (Vite dev server)
npm run dev

# App Tauri complète en mode développement
npm run tauri dev

# Vérification TypeScript
npx tsc --noEmit

# Tests Rust
cd src-tauri && cargo test

# Build release (génère l'installateur natif)
npm run tauri build
```

---

## Prérequis

- **Node.js** ≥ 18
- **Rust** (stable, via [rustup](https://rustup.rs/))
- **Tauri CLI v2** : `npm install -D @tauri-apps/cli`
- Sur Linux : `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`

---

## Notes

### Identifiant macOS

L'identifiant actuel `com.droplink.app` se termine par `.app`, ce qui peut créer des conflits avec le bundle macOS lors de la signature ou d'une distribution App Store. Changer en `com.droplink.desktop` si une distribution signée est nécessaire.

### Réseau

La découverte mDNS fonctionne uniquement sur le réseau local. Les machines doivent être sur le même sous-réseau. Certains réseaux d'entreprise ou hotspots Wi-Fi bloquent le multicast mDNS.

---

## Hors scope — Plan v0.2

- Extension contextuelle OS (Finder macOS, Shell Windows, Nautilus Linux)
- Mode invisible (désactiver mDNS advertising)
- Historique des transferts
- Chiffrement TLS du canal TCP
- Support mobile (iOS / Android)

---

## License

MIT
