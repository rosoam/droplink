# DropLink — Design Spec
**Date :** 2026-05-21
**Statut :** Approuvé

---

## Vue d'ensemble

App desktop cross-platform (Mac / Windows / Linux) de partage de fichiers par proximité, sans compte et sans serveur. Équivalent open AirDrop pour tous les OS.

**Principe :** BLE pour la découverte, TCP direct pour le transfert. Zéro serveur central. P2P pur.

---

## Stack technique

| Couche | Technologie |
|--------|-------------|
| Shell app | Tauri 2 |
| Backend | Rust |
| Frontend | React + TypeScript |
| BLE | `btleplug` (Rust crate) |
| Transfert | TCP natif (`tokio`) |
| UI styling | Tailwind CSS |

---

## Architecture

### Modules Rust

- **`discovery`** — BLE advertising du device local (nom, IP, port TCP) + scanning continu des devices à proximité. Émet des events Tauri vers le frontend à chaque changement de liste.
- **`transfer`** — TCP server (écoute entrant) + TCP client (initie sortant). Chunked streaming 64KB, progress events, vérification SHA-256 en fin de transfert.
- **`os_integration`** — enregistrement du right-click context menu par OS. Envoie le chemin fichier à l'app via IPC (si ouverte) ou socket local (si fermée).
- **`tray`** — icône système tray avec états : disponible / invisible / transfert en cours.

### Frontend React

- **`DeviceList`** — liste live des appareils BLE détectés. Statut par device : disponible / occupé / hors ligne.
- **`DropZone`** — zone drag & drop pleine fenêtre. Accepte fichiers et dossiers. Sur drop sur un device spécifique → initie transfert.
- **`TransferModal`** — progress bar, nom fichier, taille, vitesse. Affiché simultanément sender et receiver.
- **`AcceptPopup`** — popup native : nom de l'envoyeur, nom du fichier, taille. Boutons Accepter / Refuser.

---

## Note réseau

Le mode "BLE + Wi-Fi" signifie : **BLE pour la découverte** (broadcast nom + IP + port), **TCP sur le réseau local existant pour le transfert** — pas de Wi-Fi Direct hardware (P2P sans routeur), trop peu supporté sur desktop Linux/Windows. Les deux appareils doivent être sur le même réseau Wi-Fi ou câblé.

---

## Flux de découverte

```
App démarre
  → Rust scanne les ports 7777–7800, prend le premier libre
  → Rust advertise en BLE :
      { name: "MacBook de Léa", ip: "192.168.1.42", port: <port_choisi>, version: 1 }
  → Rust scan BLE en continu
  → Nouveaux appareils → Tauri event → DeviceList mis à jour
  → Appareil disparaît → retiré de la liste après 5s timeout
```

---

## Flux de transfert

```
Utilisateur sélectionne fichier(s) + device cible
  → Rust ouvre connexion TCP vers IP:port du device cible

Handshake :
  → HELLO { sender_name, file_name, file_size, checksum_sha256 }
  ← ACCEPT | REFUSE

Si ACCEPT :
  → Binary chunks (64KB chacun)
  ← ACK par chunk
  → DONE
  ← DONE_ACK

Vérification SHA-256 côté récepteur.
Fichier sauvegardé dans ~/Downloads/DropLink/.
```

---

## Initiation du partage

Deux méthodes :

1. **Drag & drop** — glisser un fichier dans la fenêtre DropLink, sur le nom d'un device
2. **Right-click OS** — clic droit sur un fichier dans l'explorateur → "Envoyer via DropLink" → sélection du device dans un petit menu

Les deux chemins convergent vers le même Rust transfer handler.

---

## Intégrations OS

| OS | Right-click | Tray icon |
|----|-------------|-----------|
| macOS | Finder Sync Extension (Swift, ~50L de glue) | NSStatusItem |
| Windows | Shell extension via Tauri plugin + registre | SystemTray Win32 |
| Linux | Script Nautilus / action `.desktop` | AppIndicator / libayatana |

Si l'app est fermée quand le right-click est déclenché : l'intégration OS lance l'app, puis lui envoie le chemin via socket local Unix/Windows named pipe.

---

## Gestion des cas limites

| Situation | Comportement |
|-----------|-------------|
| Device disparaît pendant transfert | Timeout TCP 10s → notification "Transfert interrompu" |
| Fichier déjà existant chez le récepteur | Renommage auto : `photo (1).jpg` |
| Récepteur refuse | Notification discrète côté sender : "Refusé par [nom]" |
| IP change (réseau switché) | Re-advertising BLE automatique |
| App récepteur fermée | Right-click l'ouvre avant d'initier |
| Fichier corrompu (checksum fail) | Transfert rejeté, notification erreur, fichier partiel supprimé |
| Dossier envoyé | Zippé côté sender avant transfert, dézippé côté récepteur |

---

## Sécurité

- Toute communication est locale (LAN). Aucune donnée ne sort du réseau local.
- Pas de chiffrement v1 (même réseau de confiance). Chiffrement TLS optionnel v2.
- Pas de compte, pas d'identifiant persistant — nom d'appareil OS uniquement.
- L'utilisateur accepte **chaque** transfert entrant manuellement.

---

## Tests

### Unit (Rust)
- Protocole TCP : handshake, chunking, checksum valide/invalide
- Parsing payload BLE
- Détection collision nom fichier

### Intégration
- Deux instances Tauri sur localhost → transfert fichier complet bout-en-bout
- Simulation refus → vérification que aucun fichier n'est écrit

### Manuel E2E (phase release)
- Mac → Windows
- Windows → Linux
- Fichiers > 1GB
- Drop simultané (deux transferts en parallèle)

---

## Hors scope v1

- Chiffrement TLS
- Historique des transferts
- Mode "invisible" (désactiver BLE advertising)
- Mobile (iOS / Android) — v2
- Partage de texte / clipboard
- Authentification contacts

---

## Nom provisoire

**DropLink** — nom de travail, à valider.
