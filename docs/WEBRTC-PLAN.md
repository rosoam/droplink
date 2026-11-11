# WebRTC pour DropLink — l'idée complète

> Décision : garder le LAN direct tel quel, et ajouter WebRTC comme transport Internet.
> Les données restent pair-à-pair quand le NAT le permet ; le signaling est minuscule ; le TURN
> n'est qu'un secours. Ce document fixe l'architecture, les choix à faire et le plan.

## 1. Objectif

Deux appareils, deux réseaux, n'importe quel OS du parc (Windows, Linux, macOS, iPhone) :

1. l'un crée un code, l'autre le saisit ;
2. les deux échangent leurs descriptions WebRTC via un signaling minimal ;
3. les données passent en direct (UDP) si le NAT coopère ;
4. sinon, TURN relaie les octets chiffrés ;
5. le fichier reste chiffré de bout en bout, avec le SAS DropLink actuel.

Le LAN direct (mDNS/TCP) reste prioritaire quand les deux appareils sont sur le même réseau : aucun
serveur, aucun WebRTC.

## 2. Pourquoi WebRTC

- standard, éprouvé (navigateurs, Teams, WhatsApp) ;
- data channels binaires fiables et ordonnés (SCTP) ;
- ICE gère STUN, hole punching, TURN, IPv6 ;
- disponible sur Windows, Linux, macOS, iOS ;
- DTLS chiffre le lien ; TURN ne voit que du ciphertext ;
- pas de relais applicatif à maintenir pour le cas nominal.

Ce que WebRTC n'apporte pas :

- pas de signaling intégré : il faut un transport pour offer/answer/ICE ;
- pas de garantie de direct : NAT symétrique, CGNAT mobile, pare-feu UDP ⇒ TURN ;
- pas d'identité applicative : le SAS DropLink reste nécessaire.

## 3. Architecture

```
        signaling (WSS, quelques Ko)                STUN public
   A  ───────────────────────────────►  B          (découverte d'adresse)
      ◄───────────────────────────────
                 │
                 ▼
        ICE : tentative directe UDP
                 │
        ┌────────┴────────┐
        │                 │
     direct OK          échec NAT
        │                 │
   data channel       TURN (relay chiffré)
        │                 │
        └────────┬────────┘
                 ▼
     Hello / X25519 / AEAD / BLAKE3 / SAS
```

### Couches

| Couche | Rôle |
|---|---|
| `droplink-core` | protocole applicatif, crypto, SAS (inchangé) |
| `droplink-net::transport::webrtc` | adaptateur data channel → `AsyncRead + AsyncWrite` |
| `droplink-net::transport` | sélection : LAN, WebRTC, manuel, relais |
| `droplink-signal` (nouveau) | salles éphémères, offer/answer/ICE, TTL, limites |
| UI | onglet `code` → crée/rejoint une session WebRTC |

### Signalisation

Messages JSON sur WebSocket :

```rust
enum SignalMsg {
    Create,                       // -> Created { code, token, expires_at }
    Join { code },                // -> Joined { peers }
    Offer { to, sdp },
    Answer { to, sdp },
    Ice { to, candidate },
    Bye,
}
```

- code 8 caractères, TTL 5–10 min, token de reprise ;
- la salle ne fait que transmettre les messages ; **aucune donnée de fichier** ;
- limites : 2 pairs par salle, 5 créations/min/réseau, 512 sockets ;
- le serveur ne journalise ni SDP ni code.

### Data channel

- un canal ordonné et fiable (`ordered: true`) ;
- messages de 16–32 Kio : les 256 Kio du protocole TCP actuel sont trop gros pour SCTP sur
  certains chemins ; on baisse `DEFAULT_CHUNK` pour ce transport ou on fragmente ;
- le handshake DropLink (`Hello`, `KeyOffer`, `Accept`, SAS) passe **dans** le data channel ;
- on garde notre AEAD : DTLS protège le lien, DropLink protège l'identité et le fichier.

### Sélection

`pick_transport` :

1. direct LAN (mDNS ou adresse manuelle) ;
2. WebRTC si une session distante est active ;
3. relais applicatif (plus tard) si WebRTC échoue ou si TURN est indisponible.

L'UI ne demande pas le transport : elle montre l'appareil et l'état.

## 4. Bibliothèques

| Option | Avantages | Risques |
|---|---|---|
| `webrtc` (webrtc-rs, Rust) | pur Rust, data channels, ICE/DTLS/SCTP | maturité, build iOS à valider, taille |
| `str0m` (Rust, sans-IO) | léger, contrôlable, adapté à un protocole maison | moins clé en main, ICE/SCTP à câbler |
| libwebrtc (Google, C++) | référence, mature, iOS officiel | bindings C++/Swift, build lourd, taille |
| WebRTC natif par OS | intégration | quatre stacks à maintenir |

**Recommandation : spike `webrtc-rs` d'abord.** Si le build iOS ou le débit ne tient pas, basculer
sur libwebrtc côté Apple et `webrtc-rs` côté desktop, derrière le même adaptateur.

## 5. Signaling : où et combien

- un petit binaire Rust (`droplink-signal`) sur un VPS à 5 €/mois ;
- `wss://signal.droplink.app/ws`, Caddy TLS, même modèle que le relais ;
- mémoire : quelques Ko par salle ;
- bande passante : quelques Ko par connexion, négligeable ;
- alternative serverless : Cloudflare Durable Objects, Supabase Realtime ; ou échange manuel
  QR/code (deux messages) ;
- ce serveur **ne voit pas les fichiers**.

Le jour où le relais applicatif existe, signaling et relais peuvent partager la même machine et
le même domaine, mais restent deux routes distinctes.

## 6. STUN et TURN

- **STUN** : public par défaut (Google `stun:stun.l.google.com:19302`, Cloudflare). Rien à héberger.
  Configurable dans les réglages avancés.
- **TURN** : optionnel au départ. `coturn` sur le VPS, UDP 3478, TLS 5349, authentification par
  secret partagé. À activer si le taux d'échec direct dépasse ~10–20 %.
- **Coût TURN** : uniquement les transferts qui échouent en direct. Un TURN à 1 To/mois coûte le
  prix de 1 To d'egress ; c'est le même ordre que le relais applicatif, mais seulement en secours.
- **Corporate** : UDP bloqué ⇒ TURN/TCP ou TURN/TLS 443.

## 7. Sécurité et vie privée

- Le signaling voit les IP et les SDP ; il ne voit ni fichier ni nom.
- Le TURN voit du DTLS chiffré ; il ne peut pas lire.
- Le code de session reste un secret de rendez-vous : 8 caractères, TTL, limite d'essais.
- Le SAS à 6 chiffres, comparé à l'écran, authentifie le pair malgré un signaling ou un TURN
  malveillant.
- Aucun nom de fichier dans le signaling, les logs ou le TURN.
- L'App Store accepte WebRTC ; iOS demande la permission réseau local pour le LAN, pas pour
  WebRTC.

## 8. UX

- L'onglet `code` devient le chemin « à distance » : `créer un code` / `rejoindre`.
- Une fois connecté, l'appareil apparaît dans `appareils`, comme un pair LAN.
- QR code optionnel pour partager le code sans le taper.
- États : `connexion…`, `négociation…`, `direct`, `relais (TURN)`, `échec`.
- Pas de jargon ICE/SDP dans l'UI.
- Sur iPhone, l'app doit rester au premier plan pendant la négociation.

## 9. Plan de mise en œuvre

### Phase 0 — spike (1–3 jours)

- deux processus Rust sur la même machine, `webrtc-rs`, data channel, 10 Mo, hash identique ;
- mesurer le débit et la taille du binaire ;
- builder pour `aarch64-apple-ios` et vérifier que la bibliothèque passe.

**Critère de sortie :** 10 Mo entre deux processus, hash identique, build iOS OK.

### Phase 1 — signaling (3–5 jours)

- crate `droplink-signal` : salles, code, token, TTL, limites, `/healthz` ;
- tests : création, join, offer/answer/ICE relayés, expiration, rate limit ;
- déploiement local Caddy.

**Critère :** deux clients échangent un offer/answer en < 1 s, sans donnée de fichier.

### Phase 2 — transport (5–8 jours)

- `transport/webrtc.rs` : data channel → flux `AsyncRead + AsyncWrite` ;
- handshake DropLink par-dessus ;
- `pick_transport` + pair `Reach::WebRtc` ;
- tests locaux 100 Mo, coupure, reprise de session.

**Critère :** transfert 100 Mo, hash identique, même code que le LAN.

### Phase 3 — UI et intégration (3–5 jours)

- onglet `code` branché sur signaling + WebRTC ;
- états, erreurs, QR ;
- build iOS/macOS/Windows/Linux.

**Critère :** iPhone ↔ Windows et macOS ↔ Linux à travers deux réseaux.

### Phase 4 — STUN/TURN et fiabilité (2–4 jours)

- STUN public par défaut ;
- `coturn` si nécessaire ;
- mesures : taux de direct, temps de connexion, débit, échecs par opérateur ;
- décision TURN.

**Critère :** ≥ 90 % de connexions directes sur les réseaux testés, sinon TURN validé.

### Phase 5 — durcissement

- quotas, métriques, logs sans secret ;
- tests de charge signaling ;
- documentation utilisateur.

## 10. Tests

| Test | Attendu |
|---|---|
| Local, deux processus | 100 Mo, hash identique |
| Même LAN | direct, pas de WebRTC |
| Deux NAT différents | direct ou TURN, jamais d'échec muet |
| NAT symétrique / 4G | TURN, débit stable |
| UDP bloqué | TURN/TLS 443 |
| Coupure signaling après connexion | le transfert continue |
| Coupure data channel | échec propre, reprise de session |
| iPhone en arrière-plan | suspension documentée |

## 11. Coûts

| Poste | Ordre de grandeur |
|---|---|
| Signaling (VPS 2 vCPU/2 Go) | 5–10 €/mois |
| STUN public | 0 |
| TURN (si activé) | egress au Go, seulement les cas difficiles |
| Développement | phases 0–5 ci-dessus |

## 12. Risques

| Risque | Parade |
|---|---|
| `webrtc-rs` immature sur iOS | spike phase 0, repli libwebrtc |
| NAT symétrique | TURN |
| Pare-feu UDP | TURN/TLS 443 |
| Taille binaire | feature flag, strip, LTO |
| Complexité ICE | s'appuyer sur la bibliothèque, pas réimplémenter |
| Signaling indisponible | échange manuel QR en secours |
| iOS suspend l'app | documenter, limiter la négociation au premier plan |

## 13. Décisions à prendre

1. **Bibliothèque** : `webrtc-rs` (recommandé pour le spike) ou libwebrtc.
2. **Signaling** : crate Rust auto-hébergé, ou serverless (Cloudflare/Supabase).
3. **TURN** : tout de suite, ou seulement après mesure du taux d'échec.
4. **Code** : le même code 8 caractères que le relais, ou un code plus court (6) pour l'UX.
5. **Conserver notre AEAD/SAS** par-dessus le data channel : recommandé, pour ne pas dépendre de
   l'identité DTLS.

Une fois 1 et 2 tranchés, la phase 0 peut commencer sans dépendance externe.

## 14. Résultat du spike (2026-11-11)

- Deux pairs `webrtc-rs` dans un process, SDP en mémoire, candidats host : **10 Mio en 1,13 s
  (8,9 Mio/s), BLAKE3 identique**.
- `webrtc-rs 0.21` **ne compile pas pour iOS** : `rtc-shared` dépend de `nix`, qui référence
  `libc::SYSPROTO_CONTROL` (macOS seulement).
- `str0m 0.25` **compile pour `aarch64-apple-ios`**.

Décision qui en découle : soit `str0m` partout (sans-IO, un seul stack, plus de travail
d'intégration), soit `webrtc-rs` sur desktop + `str0m`/libwebrtc sur iOS (deux stacks). La première
option est plus saine pour un produit multi-plateforme.

Code des spikes : `spikes/webrtc-data`, `spikes/str0m-check`. Aucun serveur, aucun coût.

Le plan d'exécution pas à pas (S00–S08, dépendances, critères) est dans
[`plans/2026-11-11-webrtc-remote.md`](../plans/2026-11-11-webrtc-remote.md).
