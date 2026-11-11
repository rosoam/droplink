# Plan de développement — transfert à distance (WebRTC)

> **Objectif :** envoyer un fichier entre deux appareils qui ne sont pas sur le même réseau, sur
> Windows, Linux, macOS et iPhone, sans compte et sans stockage cloud. Les données passent en
> direct quand le NAT le permet ; un TURN ne sert qu'en secours. Le handshake et le SAS DropLink
> restent la vérité d'identité.

**État de départ :** commit `c42db0c`.

- LAN direct opérationnel (mDNS, adresse manuelle, TCP, AEAD, SAS).
- `webrtc-rs` prouvé sur macOS (10 Mio, 8,9 Mio/s) mais **ne compile pas pour iOS**.
- `str0m 0.25` **compile pour `aarch64-apple-ios`**.
- Aucun signaling, aucun transport WebRTC dans l'app.
- Onglet `code` masqué (`REMOTE_READY = false`).
- Le relais applicatif existe mais n'est pas déployé ; il ne fait pas partie de ce plan.

## Décisions verrouillées

1. **Un seul stack : `str0m`** (sans-IO, pur Rust, iOS OK). Si le spike iOS échoue, repli
   libwebrtc côté Apple, mais on ne part pas sur deux stacks.
2. **Signaling séparé du relais** : crate `droplink-signal`, WebSocket, salles éphémères. Il ne
   voit jamais un octet de fichier.
3. **Code de session 8 caractères**, TTL 5 min, token de reprise. Même format que le relais.
4. **STUN public par défaut**, TURN optionnel. Pas de serveur payant pour développer.
5. **Le handshake DropLink et le SAS passent dans le data channel.** On ne fait pas confiance à
   l'identité DTLS seule.
6. **Le LAN reste prioritaire** : mDNS/TCP d'abord, WebRTC seulement si le direct local échoue.
7. **Pas de fichier dans le signaling**, pas de nom de fichier dans les logs, pas de SDP loggé.

## Graphe

```
S00 ─┬─ S01 (str0m desktop) ─── S02 (str0m iOS) ─┐
     │                                           ├─ S04 (transport) ─ S05 (UI) ─┬─ S06 (STUN/TURN)
     └─ S03 (signaling) ─────────────────────────┘                              └─ S07 (cross-platform)
                                                                                         │
                                                                                         └─ S08 (durcissement)
```

| Vague | Étapes | Parallèle |
|---|---|---|
| 1 | S00 | — |
| 2 | S01, S03 | oui : spikes vs crate signaling |
| 3 | S02, S04 | S02 après S01 ; S04 après S01+S03 |
| 4 | S05 | — |
| 5 | S06, S07 | oui |
| 6 | S08 | — |

---

# S00 — Cadrage et terrain de jeu

**Modèle :** défaut · **Dépend de :** — · **Rollback :** supprimer les branches/spikes.

### Tâches
- [ ] Branche `feat/webrtc`, pas de tag, pas de CI sur push (workflow tags-only).
- [ ] Vérifier `str0m` version 0.25 et ses exemples (`chat`, `beacon`) dans le registre Cargo.
- [ ] Décider la taille des messages data channel : 16 Kio (SCTP portable), pas 256 Kio.
- [ ] Écrire `docs/WEBRTC-DECISIONS.md` : stack, signaling, STUN/TURN, code 8, SAS.
- [ ] Vérifier que `spikes/` reste hors workspace (`[workspace]` dans chaque Cargo.toml).

### Critère de sortie
Décisions écrites, aucun changement de comportement produit.

---

# S01 — Spike `str0m` desktop : data channel + 10 Mio

**Modèle :** le plus fort · **Dépend de :** S00 · **Rollback :** supprimer `spikes/str0m-data`.

### Contexte
`str0m` est sans-IO : il faut fournir le socket UDP, les timers et le routage des paquets. C'est
la partie la plus risquée ; on la valide hors de l'app.

### Tâches
- [ ] `spikes/str0m-data` : deux `Rtc` dans un process, candidats host sur `127.0.0.1`.
- [ ] Boucle d'événements : `poll_output`, `handle_receive`, timeouts via `poll_timeout`.
- [ ] Ouvrir un data channel SCTP, envoyer 10 Mio par blocs de 16 Kio, recevoir, BLAKE3.
- [ ] Mesurer débit et CPU ; noter les pièges (DTLS, SCTP, ICE, buffers).
- [ ] Documenter dans `spikes/README.md`.

### Vérification
```bash
cargo run --release --manifest-path spikes/str0m-data/Cargo.toml
# OK 10.0 MiB, hash identique
```

### Critère de sortie
10 Mio passent entre deux `Rtc` locaux, hash identique, débit noté. Aucun changement dans
`crates/`.

---

# S02 — `str0m` sur iOS

**Modèle :** le plus fort · **Dépend de :** S01 · **Rollback :** retirer la dépendance.

### Contexte
`webrtc-rs` a échoué sur iOS (`nix`/`SYSPROTO_CONTROL`). `str0m` compile déjà pour
`aarch64-apple-ios` ; il faut vérifier le link et le socket UDP dans l'app.

### Tâches
- [ ] Ajouter `str0m` à `droplink-net` derrière une feature `webrtc` (off par défaut).
- [ ] `cargo build --release --target aarch64-apple-ios -p droplink-net --features webrtc`.
- [ ] Lier la feature dans `droplink-ios` et `droplink-app` pour la cible iOS.
- [ ] Vérifier que `tokio::net::UdpSocket` fonctionne sur iOS (bind, send_to, recv_from).
- [ ] Si le link échoue : documenter et basculer sur libwebrtc pour Apple (décision S00 revue).

### Vérification
```bash
PATH="$HOME/.cargo/bin:$PATH" cargo build --release \
  --target aarch64-apple-ios -p droplink-net --features webrtc
xcodebuild -project ios/DropLink.xcodeproj -scheme DropLink -sdk iphoneos \
  -destination 'generic/platform=iOS' -configuration Debug build
```

### Critère de sortie
Le binaire iOS contient `str0m`, linke, et l'app démarre. Pas encore de transfert.

---

# S03 — Signaling `droplink-signal`

**Modèle :** défaut · **Dépend de :** S00 · **Rollback :** supprimer le crate et la route.

### Contexte
WebRTC n'inclut pas le signaling. Il faut un service minimal qui met deux pairs en relation, sans
jamais voir les fichiers.

### Tâches
- [ ] Nouveau crate `crates/droplink-signal` (axum, WS, tokio).
- [ ] Messages :
      `Create`, `Join { code }`, `Offer { sdp }`, `Answer { sdp }`, `Ice { candidate }`, `Bye`.
- [ ] Salles : 2 pairs, code 8 caractères (réutiliser `droplink-core::relay::{generate_code,
      normalize_code, code_hash}`), token, TTL 5 min, reprise.
- [ ] Limites : 5 créations/min/réseau, 512 sockets, messages de contrôle bornés.
- [ ] `GET /healthz` : salles, pairs, échecs, sans code ni SDP.
- [ ] Tests `tokio-tungstenite` : create/join, offer/answer relayés, ICE, expiration, rate limit.
- [ ] Lancer en local : `cargo run -p droplink-signal` (port 8787).

### Vérification
```bash
cargo test -p droplink-signal
cargo run -p droplink-signal &
# deux clients échangent un offer/answer en < 1 s
```

### Critère de sortie
Deux clients se rencontrent par code, l'offer/answer et les candidats ICE passent, aucun octet de
fichier, code/SDP absents des logs.

---

# S04 — Transport WebRTC dans `droplink-net`

**Modèle :** le plus fort · **Dépend de :** S01, S02, S03 · **Rollback :** feature off.

### Contexte
Il faut transformer un data channel SCTP en flux d'octets pour réutiliser `sender`/`receiver`
tels quels, et brancher la sélection de transport.

### Tâches
- [ ] `transport/webrtc.rs` : `WebRtcTransport` (Rtc + UdpSocket + boucle d'événements) et
      `WebRtcStream` (`AsyncRead + AsyncWrite`, buffer 16 Kio, contre-pression).
- [ ] `Reach::WebRtc { peer_id }` ou `RemoteSession`, distinct de `Reach::Relay`.
- [ ] Engine : `Command::RemoteCreate`, `RemoteJoin { code }`, `RemoteEnd`.
- [ ] Signaling client : WebSocket vers `droplink-signal`, offer/answer/ICE.
- [ ] `pick_transport` : direct LAN → WebRTC → manuel → relais (plus tard).
- [ ] Tests : deux moteurs, signaling local, transfert 100 Mio, hash ; le LAN choisit direct.
- [ ] Coupure signaling après connexion : le transfert continue.

### Vérification
```bash
cargo test -p droplink-net --test webrtc_transfer
cargo test -p droplink-net --test direct_transfer
```

### Critère de sortie
100 Mio par WebRTC entre deux moteurs, BLAKE3 identique, LAN toujours direct, feature `webrtc`
désactivable.

---

# S05 — UI : rallumer l'onglet `code`

**Modèle :** défaut · **Dépend de :** S04 · **Rollback :** `REMOTE_READY = false`.

### Contexte
L'onglet `code` existe déjà, masqué. Il doit maintenant créer/rejoindre une session WebRTC, pas
une session relais.

### Tâches
- [ ] `REMOTE_READY = true` dans `ui/connect.rs`.
- [ ] `code` : créer un code → signaling, afficher code + compte à rebours ; rejoindre par code.
- [ ] États : `connexion…`, `négociation…`, `direct`, `relais (TURN)`, `échec`.
- [ ] QR optionnel pour partager le code (génération seulement).
- [ ] `UiAction`/`Command` : `RemoteCreate`, `RemoteJoin`, `RemoteEnd`.
- [ ] Tests de rendu : code, session active, échec, rejoindre.
- [ ] Docs : `docs/REMOTE.md` mise à jour.

### Critère de sortie
Sur un Mac et un iPhone, code créé sur l'un, saisi sur l'autre, transfert de 10 Mio, hash identique.

---

# S06 — STUN, TURN, fiabilité

**Modèle :** le plus fort · **Dépend de :** S05 · **Rollback :** STUN public seulement.

### Tâches
- [ ] STUN public par défaut (`stun.l.google.com:19302`), configurable.
- [ ] Mesurer le taux de direct sur : même LAN, deux NAT, 4G, réseau d'entreprise.
- [ ] Si < 90 % : `coturn` auto-hébergé, UDP 3478 + TLS 5349, auth par secret.
- [ ] Gérer l'échec ICE : timeout, message clair, repli TURN.
- [ ] Ne jamais bloquer l'UI : négociation en tâche de fond.
- [ ] Tests : UDP bloqué → TURN/TCP ; coupure réseau → échec propre.

### Critère de sortie
≥ 90 % de direct sur les réseaux testés, ou TURN validé et documenté. Aucun échec muet.

---

# S07 — Cross-platform réel

**Modèle :** défaut · **Dépend de :** S05 · **Rollback :** artefacts précédents.

### Tâches
- [ ] Binaires Windows/Linux/macOS (CI manuelle), app iOS.
- [ ] Test iPhone ↔ Windows, iPhone ↔ Linux, à travers deux réseaux.
- [ ] Pare-feu Windows (UDP + TCP), profils privés, interfaces virtuelles ignorées.
- [ ] Vérifier que le LAN reste sans serveur et que le mode avion/hotspot fonctionne.
- [ ] Documenter les limites (entreprise, UDP bloqué, CGNAT).

### Critère de sortie
Les trois combinaisons demandées passent un transfert de 10 Mio minimum, hash identique, avec
preuve datée.

---

# S08 — Durcissement et documentation

**Modèle :** défaut · **Dépend de :** S06, S07 · **Rollback :** retirer les métriques.

### Tâches
- [ ] Logs : ni code, ni SDP, ni nom de fichier ; test de capture.
- [ ] Métriques signaling : salles, échecs, durée de négociation.
- [ ] Modèle de menace : signaling honnête/curieux, TURN, relais, NAT.
- [ ] Quotas signaling (création, join) et budget TURN.
- [ ] `docs/WEBRTC-PLAN.md` : section « état final ».
- [ ] Décider si le relais applicatif reste ou disparaît (le WebRTC le rend souvent inutile).

### Critère de sortie
Docs à jour, tests verts, aucun secret dans les logs.

---

## Anti-patrons à refuser

1. Réimplémenter ICE, DTLS ou SCTP à la main. Utiliser `str0m`.
2. Faire transiter un fichier par le signaling. Il ne porte que du SDP/ICE.
3. Supprimer le SAS parce que « DTLS chiffre déjà ». Le SAS authentifie l'appareil.
4. Envoyer des blocs de 256 Kio sur SCTP. Rester à 16 Kio.
5. Supposer que le direct marchera toujours. TURN obligatoire en secours.
6. Bloquer le fil UI pendant la négociation ICE.
7. Logger un code, un SDP ou un nom de fichier.
8. Rallumer l'onglet `code` avant que S04 soit vert.
9. Ajouter un serveur pour le LAN. Le LAN reste sans serveur.
10. Déployer un TURN payant avant d'avoir mesuré le taux d'échec.

## Critères de complétion

1. iPhone ↔ Windows et iPhone ↔ Linux, deux réseaux, 100 Mio, BLAKE3 identique.
2. Le LAN reste direct et sans serveur ; le code n'apparaît pas dans les logs.
3. La négociation aboutit en < 5 s sur un réseau correct, ou échoue avec un message clair.
4. ≥ 90 % de direct sur les réseaux testés, sinon TURN validé.
5. Aucun fichier ni nom de fichier ne transite par le signaling ou le TURN en clair.
6. Tests workspace, clippy, fmt verts ; feature `webrtc` désactivable.
7. Coût de développement : zéro service payant. Le seul coût futur est le VPS signaling/TURN.

## Estimation

| Étape | Jours (solo) |
|---|---|
| S00 | 0,5 |
| S01 | 2–3 |
| S02 | 1–2 |
| S03 | 3–4 |
| S04 | 5–7 |
| S05 | 3–4 |
| S06 | 3–5 |
| S07 | 2–3 |
| S08 | 2 |

Total : **3 à 4 semaines** à plein temps, 5 à 6 semaines à temps partiel, hors imprévus NAT.

## Risques

| Risque | Parade |
|---|---|
| `str0m` difficile à câbler | spike S01 isolé, exemples `chat`/`beacon` |
| iOS refuse un socket UDP | test S02, repli libwebrtc |
| NAT symétrique | TURN |
| Pare-feu d'entreprise | TURN/TCP 443 |
| Signaling indisponible | échange manuel SDP en secours |
| Dérive de périmètre | le relais applicatif reste hors scope |
