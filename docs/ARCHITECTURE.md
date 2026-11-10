# Architecture — DropLink 1.0

## Vue d'ensemble

```
┌─────────────────────────────────────────────────────────────┐
│                     droplink-app (egui)                     │
│  devices · dropzone · queue · accept · history · settings   │
│         UiState::apply(Event) — réducteur pur testable      │
└───────────────────────────┬─────────────────────────────────┘
                            │ Command / Event (canaux bornés)
┌───────────────────────────┴─────────────────────────────────┐
│                      droplink-net :: engine                 │
│  accept loop · command loop · registre · progression 10 Hz  │
│  sender · receiver · discovery mDNS · relay client          │
└───────────┬───────────────────────────────┬─────────────────┘
            │                               │
   ┌────────┴────────┐              ┌───────┴────────┐
   │  TCP direct     │              │  circuit relay │
   │  (LAN)          │              │  (WS chiffré)  │
   └────────┬────────┘              └───────┬────────┘
            │                               │
┌───────────┴───────────────────────────────┴─────────────────┐
│                      droplink-core                          │
│  protocole · frames · pairs · hash · chemins · registre     │
│  config · crypto (X25519, HKDF, ChaCha20-Poly1305)          │
└─────────────────────────────────────────────────────────────┘
```

`droplink-core` est pur : ni I/O, ni async, ni runtime. Tout y est testable en quelques millisecondes.
`droplink-net` possède le temps, les sockets et les fichiers. `droplink-app` ne fait que du rendu et des commandes.

## Protocole v2

### Frames

```
u8 kind | u8 version | u32 len (big-endian) | payload
```

| kind | contenu |
|---|---|
| 0 | `Control` JSON en clair (Hello, KeyOffer) |
| 1 | données scellées (ciphertext AEAD) |
| 2 | `Control` scellé (Accept, Refuse, End, Done, Error) |

Toute frame dont `version != 2` est rejetée. Les longueurs sont validées **avant** allocation.

### Handshake

```
expéditeur                                          récepteur
   │ Hello { version, transfer_id, chunk_size,          │
   │         file_name, file_size, eph_pub A, alias }   │
   ├───────────────────────────────────────────────────▶│
   │                                                    │ choisit B
   │◀───────────────────────────────────────────────────┤ KeyOffer { eph_pub B }
   │                                                    │
   │  shared     = X25519(A, B_pub)  (rejette le petit ordre)
   │  transcript = BLAKE3("droplink/v2" || hello_canon || B_pub)
   │  key_s2r    = HKDF(shared, salt=transcript, info="droplink/s2r")
   │  key_r2s    = HKDF(shared, salt=transcript, info="droplink/r2s")
   │  SAS        = 6 chiffres dérivés du transcript
   │                                                    │
   │◀───────────────────────────────────────────────────┤ Accept { } (scellé r2s #0)
   │ Data scellées (s2r #0..n)                          │
   ├───────────────────────────────────────────────────▶│
   │ End { hash BLAKE3 } (scellé s2r)                   │
   ├───────────────────────────────────────────────────▶│
   │◀───────────────────────────────────────────────────┤ Done { ok } (scellé r2s #1)
```

- `hello_canon` encode de façon non ambiguë version, transfer_id, taille, taille de chunk, nom, alias et clé A :
  modifier un champ change le transcript, donc les clés, donc le SAS.
- Chaque direction a **sa clé** et **son compteur** de nonce : aucune réutilisation possible.
- `Accept`, `Refuse`, `End`, `Done`, `Error` sont scellés : un attaquant ne peut pas forger un « terminé ».

### Intégrité du fichier

L'expéditeur lit le fichier **une seule fois** : BLAKE3 est mis à jour au fil des octets envoyés, puis envoyé
dans `End`. Le récepteur écrit dans `<nom>.part` (créé avec `create_new`), vérifie le hash et la taille, puis
publie par `hard_link` (atomique, n'écrase jamais) et supprime le `.part`. Un échec supprime toujours le `.part`.

## Moteur de transfert

- `EngineHandle` : API par commandes (`Send`, `Respond`, `Cancel`, `PeerSeen`, `PeerGone`, `Shutdown`).
- `Event` : `DevicesChanged`, `Queued`, `Incoming`, `Sas`, `Progress`, `Completed`, `Failed`, `IncomingExpired`.
- La progression est agrégée par un ticker de 100 ms : au plus 10 événements/s, quelle que soit la taille du
  fichier, et `try_send` garantit qu'un consommateur figé ne ralentit jamais le transfert.
- Un seul listener TCP est lié au démarrage (port fixe ou éphémère) ; le port réel est la seule source de vérité.
- Sémaphore `max_parallel` (4 par défaut), 2 connexions simultanées par IP.
- Sérialisation par appareil : trois `Send` vers le même pair ne créent jamais trois connexions simultanées.
- `transfer_id` est créé **avant** tout I/O : même une erreur d'ouverture est corrélée à une ligne d'UI.

## Découverte

- Sélection d'interface pure : IPv4 privée, jamais loopback, link-local, VPN ou bridge.
- Aucune interface utilisable ⇒ mDNS désactivé et bannière UI, jamais d'advertisement sur `127.0.0.1`.
- TXT : `id=<uuid>`, `alias`, `v=2`. L'identité est le `device_id`, jamais le nom ni l'IP.
- Table `fullname → PeerKey` pour résoudre correctement les pertes, y compris avec des homonymes.
- Republication si l'IP change (poll 5 s).

## Relay

- WebSocket : contrôle JSON en frames **Text**, charge utile en frames **Binary** `[uuid 16 octets || ciphertext]`.
- `peer_id` est attribué par le serveur ; le `device_id` client n'est jamais rebroadcasté.
- Rooms : `code:<hash>` si un code est configuré, sinon `ip:<hash(ip publique)>`.
- Limites : 32 pairs/room, 4 circuits/pair, 10 messages de contrôle/s, 512 sockets, 8 sockets/IP, ping 15 s,
  purge à 60 s.
- Un pair lent fait fermer **son** circuit, jamais les autres.
- Déconnexion : les circuits sont coupés, la source `Relay` est purgée du registre, puis reconnexion avec backoff
  exponentiel (1 s → 30 s, jitter).

## UI

- `eframe::run_native` sur le thread principal ; le runtime Tokio (2 workers) appartient à `App`.
- `App::update` draine les événements (`try_recv`), applique le réducteur pur, puis redessine.
- Drop natif via `ctx.input(|i| i.raw.dropped_files)` — aucun `File.path` de WebView.
- `rfd::FileDialog` tourne sur un thread worker : jamais de gel de la frame.
- `egui::Modal` bloque réellement l'interface derrière la popup d'acceptation.
- Repaint adaptatif : 100 ms pendant un transfert, 500 ms au repos.

## Décisions de performance

| Décision | Raison |
|---|---|
| BLAKE3 plutôt que SHA-256 | 4,3× plus rapide mesuré sur la machine cible |
| Une seule lecture disque | supprime un second passage complet avant l'envoi |
| Chunks de 256 Kio | bon compromis débit / latence / mémoire |
| Clés AEAD par direction | sécurité, pas seulement performance |
| Progression agrégée | découple le débit réseau du renderer |
| `hard_link` pour publier | publication atomique sans écrasement |
| Zéro unsafe | `#![forbid(unsafe_code)]` sur tous les crates |

## Sessions éphémères

Une *room* n'est plus un identifiant public : le serveur crée une **session** (UUID aléatoire,
jamais exposé) et émet un code 8 caractères + un token 256 bits.

- L'hôte crée : `CreateSession { proto, device_id, alias }` → `SessionCreated { code, token, expires_at, ttl_secs }`.
- L'invité rejoint : `JoinSession { code }` → `SessionJoined { peers, token }` (token invité).
- Rotation : `RefreshSession { token }` → nouveau code, mêmes pairs, mêmes circuits.
- Coupure : le propriétaire reprend avec `ResumeSession { token }` (nouveau code) ; l'invité avec
  son token invité. Grâce de 2 min, plafond absolu 1 h.
- Le relais démarre **à la demande**, à la première commande de session. Aucune socket sans
  action utilisateur.
- `pick_transport` garde l'ordre direct → relais.
