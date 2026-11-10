# DropLink

Transfert de fichiers P2P, chiffré de bout en bout. Un binaire Rust natif, sans compte, sans WebView, sans télémétrie.

![Rust](https://img.shields.io/badge/rust-1.92-orange)
![Plateformes](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey)
![Licence](https://img.shields.io/badge/license-MIT-green)

---

## Ce que fait DropLink

1. Les appareils du même réseau se découvrent via mDNS en quelques secondes.
2. Une fenêtre native liste les pairs disponibles (badge `LAN` ou `relay`).
3. Glisser un fichier sur un appareil (ou utiliser « Choisir un fichier… »).
4. Le destinataire voit une popup avec le nom, la taille et un code de sécurité à 6 chiffres.
5. Le fichier est chiffré, transféré, vérifié (BLAKE3), puis publié dans `~/Downloads/DropLink/`.

Aucune donnée ne quitte les deux machines. Le relay optionnel ne transporte que du ciphertext.

---

## Fonctionnalités

- **Découverte LAN** : mDNS/DNS-SD, sélection d'interface IPv4 privée, republication en cas de changement d'IP.
- **Transfert P2P** : TCP direct, chunks de 256 Kio, BLAKE3 calculé pendant l'envoi (une seule lecture disque).
- **Chiffrement bout-en-bout** : X25519 éphémère, HKDF-SHA256, ChaCha20-Poly1305, **une clé par direction**.
- **Code SAS** : 6 chiffres identiques des deux côtés, affichés avant d'accepter — détection d'homme du milieu.
- **Relay** : circuits WebSocket chiffrés pour les réseaux différents ; désactivé par défaut, activable dans les réglages ; le serveur ne peut rien déchiffrer.
- **Multi-fichiers** : une ligne de progression par transfert, annulation, file sérialisée par appareil.
- **Robustesse** : écriture `.part` + publication atomique, hash vérifié avant renommage, limites de connexions.
- **100 % Rust** : UI egui/eframe, aucun Node, aucune WebView.

---

## Installation

```bash
cargo build --release -p droplink-app
./target/release/droplink
```

Paquets natifs (optionnel) :

```bash
cargo xtask bundle        # macOS .app/.dmg via cargo-bundle
cargo xtask bundle-msi    # Windows .msi via cargo-wix
```

---

## Développement

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo run -p droplink-app
cargo run -p droplink-app -- --smoke   # démarrage sans fenêtre, mesure startup_ms
cargo xtask perf                        # mesures de perf
```

Tests réseau réels :

```bash
cargo test -p droplink-net --test direct_transfer
cargo test -p droplink-net --test relay_transfer
cargo test -p droplink-net --test mdns_loopback -- --ignored --nocapture
```

---

## Architecture

```
crates/
├── droplink-core/     protocole, frames, pairs, hash, chemins, registre, config, crypto
├── droplink-net/      moteur de transfert, framing async, mDNS, transports direct/relay
├── droplink-app/      UI native egui/eframe
└── droplink-relay/    serveur axum de circuits opaques
```

Détails : [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) · Sécurité : [`docs/SECURITY.md`](docs/SECURITY.md) ·
Performances : [`docs/PERF.md`](docs/PERF.md) · Menaces : [`docs/THREAT-MODEL.md`](docs/THREAT-MODEL.md).

### Protocole en un coup d'œil

```
Hello (métadonnées + clé éphémère A)  ─────▶
                                      ◀─────  KeyOffer (clé éphémère B)
        [dérivation X25519 + HKDF + SAS des deux côtés]
                                      ◀─────  Accept / Refuse  (scellé)
Data scellés (ChaCha20-Poly1305)      ─────▶
End { hash BLAKE3 } (scellé)          ─────▶
                                      ◀─────  Done { ok }  (scellé)
```

---

## Sécurité

- Confidentialité et intégrité de bout en bout, y compris en passant par le relay.
- Le relay ne voit ni nom de fichier, ni contenu, ni clé : uniquement des identifiants de circuit et du ciphertext.
- Aucune télémétrie, aucun compte, aucun appel réseau hors mDNS et relay configuré.
- L'utilisateur doit comparer le code SAS ; sans comparaison, un homme du milieu reste possible (comme partout).

Voir [`docs/SECURITY.md`](docs/SECURITY.md).

---

## Limites connues

- mDNS peut être bloqué par certains réseaux d'entreprise ; le relay prend le relais si activé.
- La room du relay est dérivée de l'IP publique par défaut (un code de room permet de l'isoler).
- Pas encore de reprise de transfert interrompu ni de chiffrement au repos.
- Les icônes de bundle doivent être ajoutées dans `crates/droplink-app/icons/` avant une distribution signée.

---

## Licence

MIT.
