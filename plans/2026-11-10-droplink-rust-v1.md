# DropLink v1.0 — Migration Rust native + transfert chiffré (rév. 2)

> **Pour un agent qui exécute :** lis « Contrat global » puis **uniquement** l'étape assignée.
> Chaque étape est autonome. Ne saute pas d'étape, ne fusionne rien, ne modifie pas le périmètre.
> La rév. 2 intègre une revue adversariale : les types, le format sur le fil et les commandes de vérification
> sont désormais normatifs, pas indicatifs.

**Objectif :** transformer DropLink (Tauri 2 + React, drag & drop cassé, relay sans chemin de données) en un
produit Rust natif — workspace Cargo, UI egui, transfert LAN chiffré, relay réel — qui transfère réellement un
fichier entre deux instances, avec preuve automatique.

**Décisions verrouillées :**
1. UI = egui / eframe, 100 % Rust. Aucune WebView, aucun Node.
2. Relay = chemin de données réel, chiffré bout-en-bout (le relay ne voit que du ciphertext).
3. Crypto = X25519 éphémère + HKDF-SHA256 + ChaCha20-Poly1305, **une clé par direction**.
4. Intégrité = BLAKE3 en streaming (une seule lecture disque).
5. Identité = `device_id` UUID persistant ; l'IP et le nom ne sont jamais des clés.
6. Le relay ne transporte que des octets opaques ; il ne connaît ni nom de fichier ni contenu.

**État de départ :** `docs/AUDIT-2026-11-10.md`.

---

## Contrat global

### Invariants
- `#![forbid(unsafe_code)]` dans tous les crates du workspace.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` verts avant
  chaque commit.
- Aucun `unwrap()`/`expect()`/`panic!` hors tests et hors `main()` de démarrage. Erreurs : `thiserror` dans les libs,
  `anyhow` au bord binaire seulement.
- Aucun `std::fs` bloquant dans un contexte `async`.
- Aucun `Mutex` tenu à travers un `await`.
- Canaux de payload **bornés** ; `try_send` sur tout chemin réseau ; jamais d'`unbounded_channel`.
- Une constante de protocole = un seul emplacement : `droplink-core::protocol` et `droplink-core::relay`.
- `Cargo.lock` commité et source de vérité des versions.
- UI en français, code et identifiants en anglais.
- Zéro télémétrie. Réseau sortant = mDNS local + relay explicitement activé.

### Structures cibles
```
droplink/
├── Cargo.toml                      # [workspace] members = ["crates/*"], exclude legacy
├── Cargo.lock                      # commité
├── rust-toolchain.toml
├── .cargo/config.toml              # [alias] uniquement
├── crates/
│   ├── droplink-core/              # pur : ni I/O, ni tokio, ni async
│   │   ├── src/{lib,protocol,peer,frame,hash,path,registry,config,crypto,relay,error}.rs
│   │   └── benches/{frame,hash}.rs
│   ├── droplink-net/               # tokio
│   │   ├── src/{lib,engine,sender,receiver,stream,transport/{mod,direct,relay},discovery/{mod,interfaces,mdns},relay_client}.rs
│   │   ├── benches/{loopback,relay}.rs
│   │   └── tests/{direct_transfer,mdns_loopback,relay_transfer,slow_reader,limits}.rs
│   ├── droplink-app/               # bin eframe
│   │   └── src/{main,app,state,theme,ui/{mod,devices,dropzone,queue,accept,history,settings}}.rs
│   └── droplink-relay/             # créé en S07 uniquement
│       ├── src/{main,room}.rs
│       └── tests/ws_relay.rs
├── fuzz/                           # cargo-fuzz (S10)
└── docs/
```
Les répertoires `tests/` et `benches/` sont **dans chaque crate**, jamais à la racine du workspace.

### Budget de performance (chaque ligne a une commande de mesure en S10)
| Métrique | Cible | Mesure |
|---|---|---|
| Débit loopback, 256 Mio | ≥ 250 MB/s | `cargo bench -p droplink-net --bench loopback` |
| Débit LAN 1 Gbps | ≥ 100 MB/s | manuel, `docs/PERF.md` |
| CPU à 1 Gbps | < 20 % d'un cœur | manuel (`/usr/bin/time -l`) |
| Démarrage (fenêtre visible) | < 250 ms | `cargo run -p droplink-app -- --smoke` |
| Binaire release (dépouillé) | < 20 Mo | `stat -f%z target/release/droplink` |
| RSS au repos | < 80 Mo | `--smoke` échantillonne après 3 s |
| Événements `Progress` | ≤ 20 / s | test S04 |
| Lectures disque par envoi | 1× | test S04 |

### Anti-patrons interdits
1. Réintroduire Tauri, React, Vite, WebView, `npm`.
2. Même clé AEAD dans les deux sens, ou compteur de nonce partagé.
3. Lire le fichier deux fois pour hasher.
4. Un événement `Progress` par chunk.
5. Clé de registre = nom, IP ou `ip:port`.
6. Le `File.path` JavaScript de Tauri v1 (le `DroppedFile.path: Option<PathBuf>` d'egui est, lui, correct).
7. Écrire le fichier final avant vérification du hash.
8. Faire confiance au `file_name` reçu.
9. Bloquer la boucle UI sur du réseau.
10. Dupliquer un type ou une constante de protocole hors de `droplink-core`.
11. Supprimer le fichier source de l'utilisateur, quelle qu'en soit la raison.
12. `Result<()>` sans alias défini.

### Workflow git / PR
- `git tag pre-rust-v1` puis branche `feat/s<NN>-<slug>` depuis `main`.
- Un commit minimum par étape, message conventionnel.
- PR vers `main`, CI verte, squash merge.

---

# Graphe de dépendances (normatif)

```
S00 ─┬─ S01 ─── S03 ─┬─ S04 ───────────┐
     │               │                 ├─ S06 ─┐
     │               └─ S07 ─── S08 ───┘        ├─ S09 ─ S10
     └─ S02 ─── S05 ────────────────────────────┘
```

| Vague | Étapes | Note |
|---|---|---|
| 1 | S00 | |
| 2 | S01, S02 | fichiers source disjoints ; `Cargo.toml` du crate est écrit entièrement en S00 |
| 3 | S03, S05 | |
| 4 | S04, S07 | crates différents (`net` / `relay`), fichiers disjoints |
| 5 | S08 | dépend de S04 + S07 |
| 6 | S06 | dépend de S04, S05, S08 |
| 7 | S09 | |
| 8 | S10 | |

---

# S00 — Workspace Cargo + CI + tag de retour

**Modèle :** défaut · **Dépend de :** — · **Rollback :** supprimer les fichiers ajoutés, `git tag -d pre-rust-v1`.

### Contexte
On crée un workspace racine sans toucher à l'app Tauri existante, qui doit continuer à builder jusqu'à S09. Le
`Cargo.toml` de chaque crate est écrit **intégralement ici** (dépendances, dev-dependencies, blocs `[[bench]]`) pour
que S01 et S02 puissent travailler en parallèle sans se disputer le même fichier.

### Tâches
- [ ] `git tag pre-rust-v1 && git push origin pre-rust-v1`
- [ ] `Cargo.toml` racine : `[workspace] members = ["crates/*"]`, `exclude = ["src-tauri", "droplink-relay"]`,
      `resolver = "2"`, `[workspace.package]` (version `1.0.0-dev`, edition 2021, rust-version 1.92, license MIT),
      `[workspace.dependencies]` **complet** (voir liste), `[profile.release]` (`lto = "fat"`, `codegen-units = 1`,
      `strip = "symbols"`, `panic = "abort"`, `opt-level = 3`).
- [ ] `rust-toolchain.toml` : channel 1.92, composants clippy/rustfmt.
- [ ] `.cargo/config.toml` : **`[alias]` seulement** (`fmt-check`, `lint`, `test-all`). Aucun `rustflags` — il
      s'appliquerait aussi à `src-tauri/` et casserait l'ancienne app.
- [ ] Créer `crates/droplink-core`, `crates/droplink-net`, `crates/droplink-app` (bin) et `crates/xtask` (bin,
      zéro dépendance). **Ne pas** créer `crates/droplink-relay` : il naît en S07.
- [ ] `.cargo/config.toml` : alias `xtask = "run --package xtask --"` (permet `cargo xtask perf`).
- [ ] `droplink-core/Cargo.toml` : deps = serde, serde_json, thiserror, blake3, zeroize, indexmap,
      x25519-dalek (feature `static_secrets`), hkdf, sha2, chacha20poly1305, rand_core (feature `getrandom`) ;
      dev-deps = proptest, criterion ; `[[bench]] name = "frame"` et `[[bench]] name = "hash"` avec
      `harness = false`. **Pas** de tokio, **pas** de feature.
- [ ] `droplink-net/Cargo.toml` : deps = droplink-core, tokio, tokio-util, mdns-sd, if-addrs, tracing, serde,
      serde_json, thiserror, blake3, tokio-tungstenite, futures-util, url, uuid, bytes ; dev-deps = tokio-test,
      criterion ; `[[bench]] name = "loopback"` et `"relay"`.
- [ ] `droplink-app/Cargo.toml` : droplink-core, droplink-net, eframe, egui, tokio, tracing,
      tracing-subscriber, anyhow, rfd, serde, serde_json, directories. `eframe`/`egui` épinglés `=x.y.z` à la
      version résolue (≥ 0.31, requis pour `egui::Modal`) et la valeur est écrite dans le plan.
- [ ] `Cargo.lock` généré et **commité** (`cargo generate-lockfile`).
- [ ] `droplink-core/src/lib.rs` : `#![forbid(unsafe_code)]` + `pub mod protocol; peer; frame; hash; path; registry;
      config; crypto; relay; error;` avec fichiers vides annotés `// S01` / `// S02` / `// S03`.
- [ ] `.github/workflows/ci.yml` : matrice ubuntu/macos/windows ; sur ubuntu, `apt-get install -y
      libgtk-3-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev libasound2-dev` **dès maintenant** ;
      `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
      `cargo build --workspace --release` ; cache `swatinem/rust-cache`.
- [ ] `.gitignore` : ajouter `target/`, `/fuzz/target`, `/fuzz/corpus` (le corpus commité va dans `fuzz/corpus/`
      via exception explicite).
- [ ] `README.md` : section build mise à jour, mention « ancienne app Tauri en cours de retrait ».

### Vérification
```bash
cargo metadata --no-deps >/dev/null && echo ok
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
( cd src-tauri && cargo check )        # l'ancienne app doit toujours compiler
```

### Critère de sortie
Workspace vert, ancienne app intacte, tag poussé, CI verte sur les 3 OS, `Cargo.lock` commité.

---

# S01 — `droplink-core` : protocole v2, frames, pairs

**Modèle :** le plus fort · **Dépend de :** S00 · **Rollback :** revert.

### Contexte
Le protocole actuel est du JSON newline-delimited sans framing binaire ni version, avec une lecture `read_line` sur
un flux qui contient ensuite des octets bruts. On le remplace par un codec unique length-prefixed. **Tous les types
et constantes de protocole vivent ici ; aucun autre crate ne les redéfinit.**

### Spécification normative

```rust
// protocol.rs
pub type DeviceId = String;   // UUIDv4, propriété du client
pub type PeerId   = String;   // UUIDv4, attribué par le relay
pub type TransferId = String; // UUIDv4

pub const PROTOCOL_VERSION: u8 = 2;
pub const SERVICE_TYPE: &str = "_droplink._tcp.local.";
pub const MAX_CONTROL_LEN: u32 = 64 * 1024;
pub const MAX_CHUNK: u32 = 1024 * 1024;
pub const DEFAULT_CHUNK: u32 = 256 * 1024;
pub const MAX_FILE_SIZE: u64 = 64 * 1024 * 1024 * 1024;
pub const HASH_LEN: usize = 32;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction { Send, Receive }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportMode { Direct, Relay }

// peer.rs — un pair n'est pas toujours joignable en TCP
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Source { Mdns, Relay }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reach {
    Direct  { ip: std::net::IpAddr, port: u16 },
    Relay   { peer_id: PeerId },
    Both    { ip: std::net::IpAddr, port: u16, peer_id: PeerId },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub device_id: Option<DeviceId>, // None tant qu'aucun handshake ne l'a prouvé
    pub alias: String,               // libellé affiché, ≤ 64 chars, non authentifié
    pub reach: Reach,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PeerKey { Direct(DeviceId), Relay(PeerId) }
// Validation : Direct/Both ⇒ port != 0 et ip non unspecified ; Relay ⇒ peer_id non vide ;
// alias trim non vide. device_id, s'il est Some, doit parser en UUID.
```

```rust
// protocol.rs — contrôle
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Control {
    Hello { version: u8, transfer_id: TransferId, chunk_size: u32,
            file_name: String, file_size: u64, eph_pub: [u8; 32], alias: String },
    KeyOffer { eph_pub: [u8; 32] },
    Accept { transfer_id: TransferId },
    Refuse { transfer_id: TransferId, reason: String },
    End    { transfer_id: TransferId },                  // payload AEAD = BLAKE3 32 o.
    Done   { transfer_id: TransferId, ok: bool, reason: Option<String> },
    Error  { transfer_id: Option<TransferId>, reason: String },
}
```
**Règles de scellement :** `Hello` et `KeyOffer` sont en clair. **Toutes** les frames de contrôle suivantes
(`Accept`, `Refuse`, `End`, `Done`, `Error`) sont scellées AEAD. `Accept`/`Refuse` sont les premières frames scellées
de chaque direction et consomment le compteur 0 de leur direction.

### Codec de frames (`frame.rs`), API synchrone, sans I/O
Format : `u8 kind | u8 version | u32 len (big-endian) | payload`.
```rust
pub const KIND_CONTROL: u8 = 0;
pub const KIND_DATA: u8 = 1;

pub enum Frame<'a> { Control(Control), Data(&'a [u8]) }

pub fn encode_control(dst: &mut Vec<u8>, c: &Control) -> Result<()>;
pub fn encode_data(dst: &mut Vec<u8>, payload: &[u8]) -> Result<()>;
/// Ok(None) si le buffer ne contient pas une frame complète (jamais de panic, jamais d'allocation
/// quand le header seul est présent).
pub fn decode(buf: &[u8]) -> Result<Option<(Frame<'_>, usize)>>;
```
- `version != PROTOCOL_VERSION` ⇒ `Error::VersionMismatch`, **dans le header**, applicable à toute frame.
- CONTROL : `len <= MAX_CONTROL_LEN`. DATA : `len <= MAX_CHUNK + 16` (tag AEAD). Au-delà : `Error::FrameTooLarge`
  **avant toute allocation**.
- Un payload CONTROL qui ne désérialise pas ⇒ `Error::Protocol`, jamais de panic.

### Tâches
- [ ] `error.rs` : `Error { Io, Json, FrameTooLarge, Truncated, VersionMismatch, InvalidFileName, InvalidPeer,
      HashMismatch, Crypto, Protocol, Timeout, Refused, Busy, Cancelled }`.
- [ ] `protocol.rs` : types ci-dessus + validations + `#[cfg(test)]` fixtures.
- [ ] `peer.rs` : `Peer`, `Reach`, `Source`, `PeerKey`.
- [ ] `frame.rs` : codec. DATA = zéro copie (retourne une slice du buffer d'entrée).
- [ ] Tests : roundtrip control/data ; buffer fourni 1 octet à la fois ; deux frames dans un buffer ; longueur
      0xFFFFFFFF rejetée sans allocation ; version 1 rejetée ; JSON inconnu ⇒ `Protocol` ; `Peer` relay sans IP
      valide ; `Peer` direct avec port 0 rejeté.
- [ ] `proptest` (config inline `ProptestConfig { cases: 10_000, .. }`) : `decode(encode(x)) == x` pour DATA ;
      aucune panique sur `decode(arbitrary_bytes)`.
- [ ] `benches/frame.rs` : encode/decode DATA 256 KiB (criterion, `harness = false`).

### Vérification
```bash
cargo test -p droplink-core
cargo bench -p droplink-core --bench frame -- --sample-size 10
cargo clippy -p droplink-core --all-targets -- -D warnings
```

### Critère de sortie
Tests + proptest 10 000 cas verts, bench compile et s'exécute, décodage DATA sans allocation (claim limité à DATA :
CONTROL possède des `String`, il alloue, c'est attendu).

---

# S02 — `droplink-core` : chemins, BLAKE3, registre, config

**Modèle :** défaut · **Dépend de :** S00 · **Parallèle avec :** S01.

### Contexte
Trois bugs deviennent des modules testés : `sanitize_filename` incomplet, écriture non atomique, registre indexé par
`ip:port` alors que l'UI déduplique par nom. **`droplink-core` reste pur** : aucune I/O fichier ici, seulement des
fonctions de calcul et de validation. La création atomique de fichier vit en S04.

### Tâches
- [ ] `path.rs` (pur) :
  - `sanitize_file_name(raw: &str) -> Result<String>` : retire séparateurs et contrôles, refuse `""`, `.`, `..`,
    tronque à 200 octets en préservant l'UTF-8, retire points/espaces finaux, préfixe `_` les noms réservés Windows
    (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`, casse ignorée, testé avant l'extension).
  - `candidate_names(base: &str) -> impl Iterator<Item = String>` : `photo.jpg`, `photo (1).jpg`, `photo (2).jpg`…
    Générateur pur ; la boucle `create_new` est en S04.
- [ ] `hash.rs` : wrapper BLAKE3 (`Hasher::new/update/finalize -> [u8;32]`, `finalize_hex`), `HASH_ALGO_NAME`.
- [ ] `registry.rs` :
  ```rust
  // PeerKey est défini en S01 (peer.rs) — ne pas le redéclarer ici.
  pub enum Change { Added(Peer), Updated(Peer), Removed(PeerKey) }
  pub struct DeviceRegistry { /* IndexMap<PeerKey, (Peer, SourceSet)> */ }
  impl DeviceRegistry {
      pub fn upsert(&mut self, p: Peer, s: Source) -> Option<Change>;
      pub fn remove_source(&mut self, k: &PeerKey, s: Source) -> Option<Change>;
      pub fn purge_source(&mut self, s: Source) -> Vec<Change>;
      pub fn snapshot(&self) -> Vec<Peer>;
      pub fn by_device(&self, id: &DeviceId) -> Option<&Peer>;
      /// Appelé après un handshake réussi : associe un device_id prouvé à une entrée relay.
      /// Fusionne avec l'entrée Direct(device_id) si elle existe (Reach::Both).
      /// Refuse si ce device_id est déjà lié à une autre entrée.
      pub fn bind_identity(&mut self, k: &PeerKey, id: DeviceId) -> Option<Change>;
  }
  ```
  Clé : `PeerKey::Direct(device_id)` pour mDNS, `PeerKey::Relay(peer_id)` pour le relay. `upsert` met à jour le
  `Reach` de la même clé. **Un `device_id` déjà présent sous une autre clé n'écrase pas** : la nouvelle entrée est
  ignorée et `None` est retourné (anti-usurpation). La fusion Direct+Relay n'a lieu que via `bind_identity`, après
  preuve cryptographique — jamais sur simple annonce réseau.
- [ ] `config.rs` : `Config { device_id, device_name, receive_dir, port_pref: Option<u16>, max_parallel: usize,
      accept_timeout_secs: u64, relay_enabled: bool, relay_url: Option<String>, room_code: Option<String>,
      history_len: usize }` + `load_or_create(dir) -> Result<Config>` (UUID v4 au premier lancement, écriture 0600) +
      `default()` (nom d'hôte, `~/Downloads/DropLink`, `max_parallel = 4`, `accept_timeout_secs = 60`).
- [ ] Tests : proptest 10 000 cas sur `sanitize_file_name` (jamais de séparateur, jamais vide, ≤ 200 octets, jamais
      `.`/`..`) ; `candidate_names` 5 premiers noms ; registre — **`registry::duplicate_device_id_rejected`**,
      `bind_identity_merges_direct_and_relay`, `bind_identity_rejects_already_bound`, `remove_last_source_removes`,
      `purge_source_keeps_shared` ; config roundtrip + permissions 0600.
- [ ] `benches/hash.rs` : BLAKE3 vs SHA-256 sur 64 MiB, avec `assert!(blake3_mbps >= 3.0 * sha256_mbps)` dans le
      corps du bench (échec visible si la supériorité disparaît).

### Vérification
```bash
cargo test -p droplink-core
cargo bench -p droplink-core --bench hash -- --sample-size 10
cargo clippy -p droplink-core --all-targets -- -D warnings
```

### Critère de sortie
Tests verts, proptest 10 000 cas, bench avec assertion de ratio, `droplink-core` sans une seule ligne d'I/O.

---

# S03 — `droplink-core` : chiffrement E2E + SAS

**Modèle :** le plus fort · **Dépend de :** S01.

### Contexte
Aucune protection aujourd'hui. On introduit un canal authentifié par échange éphémère et comparaison d'un code à
6 chiffres (style Signal). **Toutes les valeurs ci-dessous sont normatives** : deux implémentations doivent produire
exactement les mêmes octets.

### Spécification exacte
```
domaine    = b"droplink/v2"
A          = X25519 éphémère expéditeur (Secret A, Public A)
B          = X25519 éphémère récepteur   (Secret B, Public B)
shared     = X25519(A_secret, B_public)                  // doit passer was_contributory()
                                                            // sinon Error::Crypto
hello_canon = u8(version) || len_prefixed(transfer_id) || u64_be(file_size)
              || u32_be(chunk_size) || len_prefixed(file_name)
              || len_prefixed(alias) || A_public
              // len_prefixed(x) = u32_be(x.len()) || x.as_bytes()
transcript = BLAKE3(domaine || hello_canon || B_public)
key_s2r    = HKDF-SHA256(ikm = shared, salt = transcript, info = b"droplink/s2r", len = 32)
key_r2s    = HKDF-SHA256(ikm = shared, salt = transcript, info = b"droplink/r2s", len = 32)
sas_hash   = BLAKE3(domaine || transcript || b"/sas")     // 32 octets
sas_code   = format!("{:06}", u32::from_be_bytes(sas_hash.as_bytes()[0..4]) % 1_000_000)
```
Notation : `u8(x)`, `u32_be(x)`, `u64_be(x)` = encodage big-endian fixe ; `len_prefixed(x)` = `u32_be(x.len())` suivi
des octets de `x`. `A_public`/`B_public` = 32 octets bruts.
- AEAD : `ChaCha20Poly1305`. Nonce 12 o. = `[0,0,0,0] || counter_u64_be`. Compteur strictement croissant, **un
  compteur par direction**. AAD = `transcript` (32 o.).
- Le récepteur calcule `transcript` **après** avoir choisi `B` ; il ne peut donc afficher le SAS qu'après `KeyOffer`.
- `Accept`/`Refuse` sont les premières frames scellées de la direction r2s ; `End`/`Done` continuent le compteur.
- Toute erreur AEAD ⇒ `Error::Crypto`, connexion coupée, aucun fichier final.

### Tâches
- [ ] `crypto.rs` : `EphemeralKey`, `Handshake { transcript, key_s2r, key_r2s, sas }`, `Sealer`, `Opener`.
      Secrets non `Clone`, `Drop` via `zeroize`, pas de `Debug` exposant la clé.
- [ ] `relay.rs` : types et constantes du protocole relay (voir S07) — **source unique**, importée par le serveur
      (S07) et le client (S08).
- [ ] Tests : échange A↔B ⇒ `key_s2r(A) == key_r2s(B)` et croisé, SAS identique des deux côtés ;
      **`crypto::direction_keys_differ`** (garde-fou : `key_s2r != key_r2s`) ; altération d'un bit du ciphertext ⇒
      `Crypto` ; rejeu du même compteur ⇒ `Crypto` ; clé publique de petit ordre (tous zéros) ⇒ `Crypto` ; SAS
      déterministe, 6 chiffres, valeur figée en known-answer test ; `transcript` change si `file_name`, `file_size`
      ou `alias` change (3 tests distincts).
- [ ] `docs/SECURITY.md` : ce qui est protégé (confidentialité, intégrité, détection MITM par SAS), ce qui ne l'est
      pas (utilisateur qui ne compare pas), et pourquoi le relay ne peut rien déchiffrer.

### Vérification
```bash
cargo test -p droplink-core crypto
cargo clippy -p droplink-core --all-targets -- -D warnings
```

### Critère de sortie
Tous les tests verts, known-answer test figé, `docs/SECURITY.md` rédigé, aucun `Debug` ne révèle une clé
(doc-test `compile_fail`).

---

# S04 — `droplink-net` : moteur de transfert direct

**Modèle :** le plus fort · **Dépend de :** S01, S02, S03 · **Rollback :** revert.

### Contexte
Cœur du produit. L'existant : double lecture pour le hash, événement par chunk, pas d'ACK final, écriture non
atomique, 10 s de timeout, aucune borne. On réécrit avec une API commandes/événements : l'UI n'a aucun verrou.

### API normative (`droplink-net::engine`)
```rust
pub struct EngineConfig {
    pub device_id: DeviceId,
    pub device_name: String,
    pub receive_dir: PathBuf,
    pub port_pref: Option<u16>,      // Some(p) = port fixe ; None = éphémère
    pub max_parallel: usize,          // défaut 4
    pub accept_timeout: Duration,     // défaut 60 s
    pub handshake_timeout: Duration,  // défaut 5 s
    pub progress_interval: Duration,  // défaut 100 ms
}

/// Retour de spawn : le port réellement lié est la SEULE source de vérité.
/// L'IP n'est pas connue de l'engine (bind 0.0.0.0) : la découverte la fournit.
pub struct Bound { pub port: u16 }
pub fn spawn(cfg: EngineConfig) -> Result<(EngineHandle, Bound, mpsc::Receiver<Event>)>;

pub enum Command {
    /// Un seul chemin par commande. L'engine sérialise par device.
    Send { peer: Peer, path: PathBuf },
    Respond { transfer_id: TransferId, accept: bool },
    Cancel { transfer_id: TransferId },
    /// Ingestion de la découverte : l'engine possède le registre et émet DevicesChanged.
    PeerSeen { peer: Peer, source: Source },
    PeerGone { key: PeerKey, source: Source },
    Shutdown,
}

pub struct IncomingTransfer {
    pub transfer_id: TransferId,
    pub sender_alias: String,
    pub file_name: String,
    pub file_size: u64,
    pub transport: TransportMode,
    pub sas: String,                 // disponible dès l'émission (après KeyOffer)
}

pub enum Event {
    DevicesChanged(Vec<Peer>),
    Queued { transfer_id: TransferId, file_name: String, direction: Direction },
    Incoming(IncomingTransfer),
    IncomingExpired { transfer_id: TransferId },
    Sas { transfer_id: TransferId, code: String },
    Progress { transfer_id: TransferId, file_name: String, bytes: u64, total: u64,
               direction: Direction, transport: TransportMode },
    Completed { transfer_id: TransferId, direction: Direction, file_name: String,
                path: Option<PathBuf>, transport: TransportMode },
    Failed { transfer_id: TransferId, direction: Direction, file_name: String,
             reason: String, transport: TransportMode },
}

impl EngineHandle {
    pub fn command(&self, cmd: Command);
    pub fn port(&self) -> u16;
}
```
`transfer_id` est **créé par l'engine avant tout I/O** et `Queued` est émis immédiatement : même une erreur
d'ouverture de fichier produit un `Failed` corrélable.

### Tâches
- [ ] `stream.rs` : `FramedStream<S: AsyncRead + AsyncWrite + Unpin + Send>` — buffer de lecture réutilisable
      (`MAX_CHUNK + 64`), `read_frame()`, `write_frame()`, `read_data()` sans allocation par frame.
- [ ] `sender.rs` :
  - une seule ouverture de fichier, **une seule lecture** ; BLAKE3 mis à jour pendant l'envoi ;
  - séquence : `Hello` → `KeyOffer` reçu → calcul du SAS → attente de `Accept`/`Refuse` scellé → data scellées →
    `End { hash }` scellé → attente de `Done` scellé ;
  - `Completed` émis **uniquement** sur `Done { ok: true }` ;
  - annulation via `CancellationToken` ;
  - **le fichier source n'est jamais supprimé ni modifié**.
- [ ] `receiver.rs` :
  - accept loop + `Semaphore(max_parallel)` ; `handshake_timeout` (5 s) **avant** d'émettre `Incoming` ;
    cap 2 connexions simultanées par IP source (HashMap d'IP → compteur, nettoyé à la fermeture) ;
  - validation : `version`, `file_size <= MAX_FILE_SIZE`, `file_name` via `sanitize_file_name`, `chunk_size` borné ;
  - écriture dans `<nom>.part` créé par `create_new` (boucle sur `candidate_names`), puis `sync_all` + `rename`
    vers le nom final **seulement après** vérification du hash ;
  - échec ⇒ supprimer le `.part`, `Done { ok: false }`, `Failed` ;
  - succès ⇒ `Done { ok: true }` puis `Completed` avec le chemin final.
- [ ] `create_unique_async(dir, base_name) -> Result<(PathBuf, tokio::fs::File)>` dans `droplink-net::fs_util`
      (boucle `OpenOptions::create_new`, async, jamais `exists()`).
- [ ] Progression : tâche de coalescence, tick `progress_interval`, dernier état par transfert, `try_send` sur un
      canal borné — un consommateur figé ne ralentit **jamais** le transfert.
- [ ] `engine.rs` : un seul `TcpListener::bind` (port fixe ou éphémère), jamais de scan TOCTOU ; `Bound.port` exposé ;
      `Shutdown` annule tout et ferme le listener. L'engine possède le `DeviceRegistry` : `PeerSeen`/`PeerGone`
      mettent à jour le registre et émettent `DevicesChanged` (dédupliqué : seulement si la liste a changé).
- [ ] Tests `tests/direct_transfer.rs` :
  - 1 Mio et 64 Mio loopback ⇒ contenu et BLAKE3 identiques ;
  - refus ⇒ `Failed` chez l'expéditeur, aucun fichier écrit chez le récepteur, source intacte ;
  - **`no_completed_without_done_ok`** : récepteur forcé à répondre `Done { ok: false }` ⇒ aucun `Completed` ;
  - **`hash_mismatch`** : expéditeur instrumenté qui scelle un hash d'un autre contenu ⇒ récepteur `Failed`,
    `.part` supprimé, aucun fichier final ;
  - **`tampered_ciphertext`** : octet flippé sur le flux ⇒ `Crypto`, aucun fichier final ;
  - flux tronqué (connexion coupée) ⇒ `Failed`, aucun fichier final (le `.part` est supprimé) ;
  - deux transferts simultanés du même nom ⇒ deux fichiers distincts ;
  - annulation ⇒ `Failed`, `.part` supprimé ;
  - **`per_device_serialized`** : 3 `Send` vers le même pair ⇒ jamais 3 connexions simultanées ;
  - **`progress_rate_capped`** : 256 Mio en loopback ⇒ ≤ 20 `Progress`/s mesurés ;
  - **`stalled_consumer_does_not_stall_transfer`** : ne pas lire le canal d'`Event` ⇒ le transfert se termine quand
    même ;
  - **`bind_once`** : deux `spawn` successifs ⇒ ports distincts, aucun `expect`/panic ;
  - **`single_read_proof`** : envoi d'un fichier 64 Mio via un wrapper de fichier instrumenté ⇒ exactement une
    lecture séquentielle complète, aucune seconde ouverture ;
  - **`create_unique_concurrent_10`** : 10 tâches réceptrices écrivent le même nom ⇒ 10 fichiers distincts, aucun
    écrasement.
- [ ] `benches/loopback.rs` : 256 Mio, `assert!(mbps >= 250.0)`.

### Vérification
```bash
cargo test -p droplink-net --test direct_transfer -- --nocapture
for i in 1 2 3; do cargo test -p droplink-net --test direct_transfer -- --test-threads=1; done
cargo bench -p droplink-net --bench loopback -- --sample-size 10
cargo clippy -p droplink-net --all-targets -- -D warnings
```

### Critère de sortie
Les 14 tests verts et stables sur 3 passes, débit ≥ 250 MB/s, aucune lecture disque double (test
`single_read_proof` compte les `read` via un wrapper de fichier instrumenté).

---

# S05 — `droplink-net` : découverte mDNS + interfaces

**Modèle :** défaut · **Dépend de :** S01, S02 · **Parallèle avec :** S03.

### Contexte
`get_local_ip()` dépend d'une route vers `8.8.8.8` : sur un LAN isolé il renvoie `127.0.0.1` et personne ne voit
l'appareil. Le mapping `fullname → ip:port` est approximé par préfixe de nom. On réécrit sur `device_id`, et le port
réellement lié est fourni par l'appelant (jamais deviné).

### Tâches
- [ ] `discovery/interfaces.rs` : `select_advertisable_ip(&[Iface]) -> Option<Ipv4Addr>` — préfère IPv4 privée
      (10/8, 172.16/12, 192.168/16), exclut loopback, link-local et préfixes `utun`, `docker`, `vmnet`, `vboxnet`,
      `bridge`. Fonction **pure**, `if-addrs` uniquement dans le wrapper.
      `None` ⇒ **mDNS désactivé**, émission de `DiscoveryEvent::Unavailable`, relay seul (jamais `127.0.0.1`).
- [ ] `discovery/mdns.rs` : API `spawn(advert: AdvertSpec, events: mpsc::Sender<DiscoveryEvent>) -> Result<DiscoveryHandle>`
      avec `AdvertSpec { device_id, alias, ip: Ipv4Addr, port: u16 }` — **le port est un paramètre**, la découverte
      ne connaît ni `Bound` ni l'engine (le câblage se fait en S06 avec `Bound.port`).
  - advertise : instance `<alias>-<6 premiers hex du device_id>`, TXT = `id=<device_id>`, `alias=<alias>`, `v=2` ;
  - browse : `ServiceResolved` ⇒ parse TXT, ignore son propre `device_id`, émet
    `DiscoveryEvent::Found(Peer { Reach::Direct, .. })` ; `ServiceRemoved` ⇒ `DiscoveryEvent::Lost(fullname)` ;
    table `fullname → PeerKey` interne (plus de préfixe de nom) ;
  - re-publication si l'IP change (poll toutes les 5 s).
- [ ] `DiscoveryEvent { Found(Peer), Lost(String), Unavailable }` — l'engine (S04) ingère via
      `Command::PeerSeen` / `PeerGone` ; la découverte ne touche pas au registre.
- [ ] Tests : **`interfaces::isolated_lan_returns_none`** et 5 autres jeux d'interfaces injectés (VPN seule ⇒ `None`,
      docker+en0 ⇒ `en0`, link-local seule ⇒ `None`, deux privées ⇒ la non-VPN) ; parsing TXT (id manquant, id
      invalide, v≠2 ⇒ ignoré) ; **`same_name_two_devices`** via l'API de l'engine ⇒ 2 entrées ; perte résout le bon
      `device_id`.
- [ ] Test `tests/mdns_loopback.rs` `#[ignore]` : deux daemons sur la même machine se découvrent.

### Vérification
```bash
cargo test -p droplink-net discovery
cargo test -p droplink-net --test mdns_loopback -- --ignored --nocapture
```

### Critère de sortie
Deux devices du même nom coexistent, perte correcte, aucun `127.0.0.1` annoncé, comportement `None` explicite.

---

# S06 — `droplink-app` : UI egui native

**Modèle :** défaut · **Dépend de :** S04, S05, S08 · **Rollback :** revert.

### Contexte
La seule voie d'envoi actuelle (drag & drop HTML5 lisant `file.path`) ne fonctionne pas sous Tauri 2. En egui, le
drop natif arrive par `ctx.input(|i| i.raw.dropped_files.clone())`, avec `DroppedFile.path: Option<PathBuf>`.
L'UI ne contient **aucune** logique de transfert : elle consomme `Event`, émet `Command`.

### Intégration runtime (normative)
- `main.rs` : `eframe::run_native` occupe le thread principal ; **jamais** de `#[tokio::main]`.
- `App` possède `rt: tokio::runtime::Runtime` (multi-thread, 2 workers) construit dans `App::new`.
- `App::new` fait `rt.block_on(droplink_net::engine::spawn(cfg))`, garde `EngineHandle`, `Receiver<Event>`.
- `App::update` : draine `rx.try_recv()` en boucle (max 1000/trame) avant le rendu, puis
  `ctx.request_repaint_after(Duration::from_millis(100))` si un transfert est actif, sinon
  `request_repaint_after(1 s)` pour rafraîchir la liste.
- Aucun appel bloquant dans `update()` : `rfd::FileDialog` tourne sur un thread worker (`std::thread::spawn`),
  résultat récupéré par un `mpsc::Receiver<Option<PathBuf>>` drainé en début de trame.

### Tâches
- [ ] `main.rs`, `theme.rs` : fenêtre 380×560 non redimensionnable, dark (#0f1115 / #3b82f6), `--smoke` qui démarre,
      mesure le temps jusqu'à la première trame et le RSS, puis quitte.
- [ ] `state.rs` (testable sans GUI) :
  ```rust
  pub struct UiState {
      pub devices: Vec<Peer>,
      pub selected: Option<PeerKey>,
      pub queue: Vec<TransferRow>,                  // une ligne par transfert
      pub incoming: HashMap<TransferId, IncomingTransfer>, // multi-entrant
      pub sas: HashMap<TransferId, String>,
      pub history: Vec<HistoryEntry>,
      pub discovery_available: bool,
      pub settings_open: bool,
  }
  pub struct TransferRow { pub transfer_id: TransferId, pub file_name: String,
                           pub direction: Direction, pub bytes: u64, pub total: u64,
                           pub transport: TransportMode, pub started: Instant }
  ```
  `fn apply(&mut self, ev: Event)` pur. Événement pour un `transfer_id` inconnu ⇒ ignoré, jamais de panic.
- [ ] `ui/devices.rs` : liste, sélection, badge LAN/relay, état vide, bannière si `discovery_available == false`.
- [ ] Câblage au démarrage (dans `App::new`) : `spawn(cfg)` → `Bound` ; démarrer `mdns::spawn(AdvertSpec { port:
      bound.port, .. })` si une IP est disponible ; une tâche relaie `DiscoveryEvent` → `Command::PeerSeen/PeerGone`.
      Si `mdns` indisponible : `Command::PeerGone` pour tous les pairs mDNS et bannière UI.
- [ ] `ui/dropzone.rs` : zone pleine largeur, surbrillance via `i.raw.hovered_files`, drop ⇒ **une `Command::Send` par
      chemin** (filtre `path.is_some()`), bouton « Choisir un fichier… » via `rfd` (worker thread).
- [ ] `ui/queue.rs` : ligne par transfert (nom, barre, débit lissé 1 s, direction, transport, Annuler).
- [ ] `ui/accept.rs` : `egui::Modal` (bloquant, non contournable), une popup par entrée de `incoming` (pile si
      plusieurs), nom, fichier, taille, mode, **SAS en 3-3**, Accepter/Refuser.
- [ ] `ui/history.rs` : 100 derniers transferts persistés (config dir), succès/échec, chemin, « Révéler ».
- [ ] `ui/settings.rs` : nom, dossier de réception, port fixe/auto, relay on/off + URL + code de room, sauvegarde.
- [ ] `tracing-subscriber` : stdout + fichier rotatif.
- [ ] Tests `src/state.rs` : **`drop_three_paths_enqueue_three_rows`** ; `queued_out_of_order_still_matches` ;
      progression décroissante ignorée ; `completed_moves_to_history` ; `failed_is_correlated` ;
      `three_incoming_three_modals` ; `incoming_expired_removes_only_that` ; `same_name_two_devices` ; événement
      inconnu ignoré.
- [ ] Tests `tests/ui_smoke.rs` : lance `--smoke` en sous-processus, vérifie code de sortie 0 et durée < 250 ms
      (tolérance CI ×3).

### Vérification
```bash
cargo test -p droplink-app
cargo run -p droplink-app -- --smoke
cargo run -p droplink-app      # manuel : glisser un fichier, vérifier la barre
cargo clippy -p droplink-app --all-targets -- -D warnings
```

### Critère de sortie
Démo manuelle complète, `--smoke` vert, 9 tests de réducteur verts, aucune référence à Tauri dans le crate.

---

# S07 — Relay v2 : appairage + circuits de données

**Modèle :** le plus fort · **Dépend de :** S01, S03 · **Parallèle avec :** S04.

### Contexte
Le relay actuel liste des pairs et ne transfère rien ; identité = nom ; pas de heartbeat ; pas de borne. On le
transforme en relais de circuits : il achemine des octets opaques et applique des limites dures. **Les types et
constantes viennent de `droplink_core::relay` (S03) : le serveur ne les redéfinit pas.**

### Protocole (normatif, `droplink_core::relay`)
Contrôle = frames WS **Text** JSON. Données = frames WS **Binary**.
```
TEXT  client→serveur : register { device_id, alias }
TEXT  serveur→client : registered { peer_id }
TEXT  serveur→client : room_update { peers: [{ peer_id, alias }] }      // jamais d'IP, jamais de device_id
TEXT  client→serveur : connect { peer_id }
TEXT  serveur→client : circuit_opened { circuit_id, peer: { peer_id, alias }, initiator: bool }
TEXT  serveur→client : circuit_closed { circuit_id, reason }
TEXT  client→serveur : close { circuit_id }
TEXT  serveur→client : error { code, message }

BINARY (les deux sens) : [0..16] circuit_id (UUID brut, 16 octets) | payload
```
- `MAX_RELAY_FRAME = 1 MiB + 16` appliqué au **payload seul** (hors 16 octets d'en-tête).
- `peer_id` attribué par le serveur ; `device_id` déclaré par le client est **non vérifié** et n'est jamais
  rebroadcasté. Un `device_id` déjà enregistré par un autre pair est refusé (`error{code:"duplicate_id"}`).
- Room : `room_code` explicite si configuré (`code:<blake3(code)[..8]>`), sinon `ip:<blake3(ip_publique)[..8]>`.
  Le hash empêche de lire l'IP dans les logs ; les pairs d'une même room ne reçoivent que `peer_id` + `alias`.
- Limites : 32 pairs/room, 4 circuits/pair, 10 messages de contrôle/s/pair (token bucket), handshake ≤ 5 s,
  ping WS 15 s, purge à 60 s d'inactivité, **cap global de 512 sockets** et **8 sockets/IP** (semaphore + compteur).
- `route()` copie le `Sender` cible sous le verrou puis `try_send` **hors du verrou** ; file pleine ⇒ circuit fermé.
- Le serveur ne journalise jamais `payload`, ni `file_name` (il ne les reçoit pas).

### Tâches
- [ ] Créer `crates/droplink-relay` (S00 ne l'a pas créé) avec `Cargo.toml` : deps = droplink-core, axum
      (feature `ws`), tokio, serde, serde_json, tracing, tracing-subscriber, blake3, uuid, futures-util ;
      dev-deps = tokio-tungstenite ; `[[test]] name = "ws_relay"`. Supprimer `droplink-relay/Cargo.lock` et
      `droplink-relay/target/` ; mettre à jour `Dockerfile` (`rust:1.92-slim`, `COPY Cargo.toml Cargo.lock`,
      `COPY crates/*/Cargo.toml`, `cargo build -p droplink-relay --release`) et `fly.toml`.
- [ ] `room.rs` : `Rooms` **sans axum** — `register`, `connect`, `route`, `drop_peer`, `drop_circuit`, `stats`, toutes
      les limites ici, testables sans réseau.
- [ ] `main.rs` : handler WS mince, `tokio::signal::ctrl_c` graceful shutdown, `tracing` JSON.
- [ ] Tests `room.rs` : appairage bidirectionnel ; routage data ; `duplicate_id` refusé ; fermeture ⇒ circuits
      fermés + `room_update` ; 33ᵉ pair ⇒ `limit` ; 5ᵉ circuit ⇒ `limit` ; token bucket ; room vide supprimée ;
      **slow reader** ⇒ file pleine ⇒ circuit fermé, les autres rooms ne sont pas bloquées.
- [ ] Test `tests/ws_relay.rs` : deux clients `tokio-tungstenite` réels, 1 Mio d'octets routés intacts,
      déconnexion brutale ⇒ nettoyage < 100 ms.

### Vérification
```bash
cargo test -p droplink-relay
cargo clippy -p droplink-relay --all-targets -- -D warnings
docker build -f crates/droplink-relay/Dockerfile -t droplink-relay .   # si docker présent
```

### Critère de sortie
Tests verts, aucune identité fournie par le client n'est utilisée comme clé, limites testées, payload jamais loggé.

---

# S08 — Transport unifié : direct → relay + E2E

**Modèle :** le plus fort · **Dépend de :** S04, S07.

### Contexte
Les deux chemins utilisent le **même** moteur. On abstrait le transport en `AsyncRead + AsyncWrite` ; le chiffrement
de S03 s'applique indifféremment. Le relay ne voit que du ciphertext.

### Tâches
- [ ] `transport/mod.rs` : `pub trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}` +
      `impl<T: ...> AsyncReadWrite for T {}` ; alias `BoxTransport`.
- [ ] `transport/direct.rs` : `TcpStream`, connect timeout 2 s.
- [ ] `transport/relay.rs` : `RelayStream` implémentant `AsyncRead`/`AsyncWrite` sur un circuit WS :
      écriture ⇒ frame Binary `[circuit_id;16] | bytes` ; lecture depuis un `mpsc::Receiver<Bytes>` borné (cap 64)
      alimenté par la tâche WS ; `read` partiel correct (buffer interne) ; `Close` propre.
- [ ] `relay_client.rs` : connexion WS, `register`, `ping/pong`, `connect` à la demande. Le client est **possédé par
      l'engine** (démarré par `spawn` si `relay_enabled`) et alimente le registre via `PeerSeen`/`PeerGone` ;
      l'engine émet `DevicesChanged`. **Sur déconnexion : drop le `Sender` du circuit (les `read` retournent `None`),
      `Failed` immédiat pour chaque transfert en cours via relay, puis `purge_source(Relay)`.** Reconnexion backoff
      1→30 s avec jitter.
- [ ] `engine.rs` : sélection du transport — direct (2 s) d'abord, sinon relay si activé et pair présent, sinon
      `Failed { reason: "pair injoignable" }` ; `TransportMode` reporté dans **chaque** `Event`.
- [ ] `Command::Send` accepte un `Peer` dont `Reach::Relay` n'a pas d'IP : c'est ce chemin qui rend `Peer` non-TCP
      utilisable.
- [ ] Tests `tests/relay_transfer.rs` :
  - **`relay_to_relay_8mib`** : relay en tâche (port éphémère) + 2 engines forcés relay ⇒ 8 Mio intègres ;
  - **`relay_no_plaintext_leak`** : fichier fixture = marqueur ASCII de 64 octets répété ; un proxy WS dans le test
    capture toutes les frames Binary ; assertion `!frames.concat().windows(64).any(|w| w == MARKER)` ; assertion
    que le nombre d'octets relayés ≈ taille du fichier + overhead AEAD ;
  - **`relay_drop_fails_fast`** : coupure du relay en cours ⇒ `Failed` rapide, aucun fichier final ;
  - **`relay_reconnect_recovers`** : retour du relay ⇒ reconnexion et nouveau transfert OK.
- [ ] `benches/relay.rs` : débit informatif.

### Vérification
```bash
cargo test -p droplink-net --test relay_transfer -- --nocapture
cargo clippy --workspace --all-targets -- -D warnings
```

### Critère de sortie
Transfert relayé sans fuite de clair (test falsifiable), bascule direct → relay, purge correcte à la déconnexion.

---

# S09 — Retrait de l'ancien stack + packaging + docs

**Modèle :** défaut · **Dépend de :** S06, S08 · **Rollback :** `git reset --hard pre-rust-v1`.

### Tâches
- [ ] Checklist de parité **avant** toute suppression (manuelle) :
      découverte LAN < 5 s · drag & drop multi-fichiers OS · sélecteur de fichier · accept/refus/timeout 60 s ·
      progression par fichier + annulation · historique persistant · réglages persistants · transfert relay ·
      SAS identique des deux côtés.
- [ ] Supprimer `src/`, `src-tauri/`, `package.json`, `package-lock.json`, `node_modules/`, `dist/`, `index.html`,
      `vite.config.ts`, `tsconfig*.json`, `tailwind.config.ts`, `.vscode/`, `.github/workflows/release.yml`,
      l'ancien `droplink-relay/` (racine).
- [ ] `.gitignore` nettoyé ; `README.md`, `docs/ARCHITECTURE.md`, `docs/CHANGELOG.md` (entrée 1.0.0),
      `docs/PERF.md` réécrits.
- [ ] Version `1.0.0` dans `[workspace.package]` ; identifiant bundle macOS `com.droplink.desktop`.
- [ ] Packaging : `cargo-bundle` (macOS), `cargo-wix` (Windows), `.desktop` + AppImage (Linux), `xtask` unifié.
- [ ] CI release : 3 OS, tests d'abord, artefacts sur tag `v*`.

### Vérification
```bash
cargo test --workspace
cargo build --workspace --release
cargo run -p droplink-app       # checklist manuelle 9/9
git ls-files | grep -E '\.(ts|tsx|js|jsx)$|package\.json' && exit 1 || true
test ! -e node_modules && test ! -e src-tauri && echo "legacy removed"
```

### Critère de sortie
Aucun fichier JS/TS/Tauri dans le dépôt suivi, checklist 9/9, binaire < 20 Mo, artefacts macOS + Windows produits.

---

# S10 — Durcissement, fuzz, perf continue

**Modèle :** défaut · **Dépend de :** S09.

### Tâches
- [ ] `cargo-fuzz` : `fuzz_frame` (frames arbitraires, aucun panic, aucune allocation > MAX), `fuzz_txt` (TXT mDNS),
      `fuzz_control` (JSON) — 10 min chacune, corpus commité.
- [ ] `proptest` sur le réducteur UI : séquences d'événements arbitraires ⇒ état cohérent, aucune panique.
- [ ] `tests/slow_reader.rs` : récepteur à 1 MB/s ⇒ contre-pression, RSS stable < 100 Mo sur 512 Mio.
- [ ] `tests/limits.rs` : `max_parallel_rejects_fifth` (5ᵉ connexion entrante parallèle ⇒ refus propre),
      `per_ip_cap` (3ᵉ connexion depuis la même IP ⇒ refus), 33ᵉ pair ⇒ refus, 5ᵉ circuit ⇒ refus, frame 2 Mio ⇒
      coupure, `file_size` menteur (annonce 1 Mio, envoie 10 Mio) ⇒ coupure.
- [ ] `docs/THREAT-MODEL.md` (STRIDE : handshake, data, relay, fichiers reçus, config locale).
- [ ] CI : `cargo llvm-cov --workspace --fail-under-lines 85`, `cargo audit`, `cargo deny check`, gate de taille
      binaire (> 22 Mo ⇒ échec ; la cible est 20 Mo, la marge absorbe les variations de toolchain).
- [ ] Les commandes de mesure du tableau de perf sont scriptées dans `xtask perf` et leurs résultats écrits dans
      `docs/PERF.md`.

### Vérification
```bash
cargo +nightly fuzz run fuzz_frame -- -max_total_time=600
cargo test --workspace --release
cargo llvm-cov --workspace --fail-under-lines 85
cargo deny check
cargo xtask perf
```

### Critère de sortie
Aucun crash sur 10 min de fuzz par cible, couverture ≥ 85 %, `docs/THREAT-MODEL.md` complet, CI verte 3 OS.

---

## Definition of Done (chaque ligne a un test nommé)

| # | Exigence | Preuve |
|---|---|---|
| 1 | Drag & drop natif fonctionne | `droplink-app` test `drop_three_paths_enqueue_three_rows` + checklist manuelle S09 |
| 2 | Relay transfère réellement | `relay_transfer::relay_to_relay_8mib` |
| 3 | Pas de « terminé » sans ACK | `direct_transfer::no_completed_without_done_ok` |
| 4 | Progression non bloquante | `progress_rate_capped` + `stalled_consumer_does_not_stall_transfer` |
| 5 | Multi-fichiers correct | `drop_three_paths_enqueue_three_rows` + `per_device_serialized` |
| 6 | Identité par `device_id` | `registry::duplicate_device_id_rejected` + `same_name_two_devices` |
| 7 | LAN isolé fonctionnel | `interfaces::isolated_lan_returns_none` + `mdns_loopback` |
| 8 | Zéro TOCTOU port/fichier | `bind_once` + `create_unique_concurrent_10` |
| 9 | Entrant borné | `limits::max_parallel_rejects_fifth` + `per_ip_cap` |
| 10 | Chiffrement E2E | `crypto::direction_keys_differ` + `relay_no_plaintext_leak` |
| 11 | Le relay ne voit rien | `relay_transfer::relay_no_plaintext_leak` |
| 12 | Perf tenue | `cargo xtask perf` (toutes les lignes du tableau) |
| 13 | Plus aucun legacy | `S09` : `git ls-files | grep -E '\.(ts|tsx)$'` vide |

**Definition of Done globale :** `cargo test --workspace` vert, `cargo llvm-cov --fail-under-lines 85` vert,
deux instances transfèrent 1 Gio en < 6 s sur loopback avec hash identique, et le même transfert passe par le relay
sans qu'aucun octet en clair n'apparaisse dans les frames relayées.

## Registre des risques

| Risque | Probabilité | Mitigation |
|---|---|---|
| `eframe`/`egui` API instable | moyenne | version épinglée en S00, rendu encapsulé dans `ui/`, réducteur testé hors GUI |
| mDNS bloqué par le réseau | élevée | relay en repli, bannière `DiscoveryUnavailable`, mode IP directe en réglages |
| AEAD plus lent que le disque sur ARM | faible | benchs S02/S04, bascule `aws-lc-rs` possible sans changer le protocole |
| Relay Fly.io indisponible | moyenne | relais local en test, URL configurable |
| Perte de polish en réécrivant l'UI | moyenne | thème défini en S06, revue visuelle avant S09 |
| Divergence client/relay | faible | types uniques dans `droplink-core::relay`, test S08 |

---

# Journal d'exécution (rév. 2)

Écarts assumés par rapport au plan initial, tous motivés par une contrainte réelle rencontrée à l'exécution :

| Écart | Raison | Impact |
|---|---|---|
| `eframe`/`egui` épinglés à **0.33.3** au lieu de la dernière version | la 0.35 remplace `App::update` par `App::ui` et supprime `TopBottomPanel`/`Context::style` — réécriture d'API non justifiée pour ce produit | aucune fonctionnalité perdue ; `Modal` et le drop natif sont disponibles |
| La config et l'historique sont persistés dans `droplink-app` (`store.rs`), pas dans `droplink-core` | préserver l'invariant « core pur, sans I/O » | core reste testable en millisecondes ; permissions 0600 gérées côté app |
| Le relay tourne en tâche dans les tests (`serve`) plutôt qu'en processus séparé | tests hermétiques et rapides | `relay_drop_fails_fast` et `relay_reconnect_recovers` utilisent un proxy WS pour couper réellement les connexions |
| Fuzz exécuté via `rustup run nightly cargo fuzz` | `cargo` installé par Homebrew n'est pas le shim rustup, donc `cargo +nightly` échoue | cibles et commandes documentées dans le README |
| `cargo xtask perf` mesure la taille du binaire et le startup en plus des benchs | le plan demandait des mesures scriptées | les budgets sont vérifiés par une commande unique |

Preuves d'exécution :

- `cargo test --workspace` : 154 tests verts (core 74, net 19 + 14 + 6 + 4 + 1, relay 13 + 2, app 20 + 1).
- `cargo clippy --workspace --all-targets -- -D warnings` : vert.
- `cargo fmt --all -- --check` : vert.
- Débit loopback : 436 MB/s (budget 250), BLAKE3 4,26× SHA-256, démarrage 4 ms.
- Binaire release : 5,98 Mo (budget 20 Mo).
- `relay_no_plaintext_leak` : aucun marqueur en clair dans les frames relayées.
- `mdns_loopback` : deux daemons se découvrent réellement sur la machine.
- Fuzz : 181,5 M exécutions cumulées sur 3 cibles (10 min chacune), aucun crash.
- Couverture : 87,45 % hors points d'entrée binaires et xtask (gate CI à 85 %).
