# DropLink — envoi à distance par code éphémère (rotatif, 5 min)

> **Pour un agent qui exécute :** lis « Contrat global » puis **uniquement** l'étape assignée.
> Chaque étape est autonome : elle donne les fichiers, les signatures, les tests et la commande de
> vérification. Ne saute pas d'étape, ne fusionne rien, ne modifie pas le périmètre.
> **Rév. 2** : intègre une revue adversariale. Les points marqués ⚠ sont des corrections de
> sécurité ou de tenue en charge, pas des options.

**Objectif :** permettre d'envoyer un fichier à une personne qui n'est **pas sur le même réseau** :
l'un affiche un code court, l'autre le saisit, et le transfert passe par un relais auto-hébergé.
Le code **tourne toutes les 5 minutes** et ne donne accès qu'à une session éphémère.

**État de départ (vérifié le 2026-11-10, commit `7a8a63d`) :**

- `crates/droplink-relay` existe : `Rooms` (salles, circuits, plafonds), `server.rs` (axum, `/ws`,
  `/healthz`), tests `ws_relay.rs`. La salle est choisie par le client (`Register { room }`) ou
  déduite de l'IP (`room_id(ip, code)`) ; **aucun code n'est émis, aucune expiration, aucun
  renouvellement**.
- `crates/droplink-net` a un client relais (`relay_client.rs`, `transport/relay.rs`),
  `pick_transport` essaie le direct puis le relais, et `engine.rs` reconnecte avec backoff.
  **`RelayClient::connect` envoie `Register` implicitement** (`relay_client.rs:169-176`) : c'est un
  obstacle direct au modèle « salle = UUID » (voir S03).
- `crates/droplink-core/src/relay.rs` porte le protocole relais ; `crypto.rs` fournit le handshake
  X25519 + AEAD et un **SAS à 6 chiffres**. ⚠ Le SAS n'est affiché **que côté récepteur**
  (`ui/accept.rs`) ; l'émetteur le reçoit en `Event::Sas` mais `ui/queue.rs` ne l'affiche pas.
  `docs/THREAT-MODEL.md` classe « SAS non comparé » en risque accepté.
- L'UI expose « Activer le relay » + « Code de room » statique (`ui/settings.rs`), et
  `Config.room_code`. `engine.rs:387` démarre le relais au boot si `relay_enabled`.
- `Config.relay_url` par défaut : `wss://droplink-relay.fly.dev/ws` — **DNS mort, vérifié**.
  `EngineConfig::new` (`engine.rs:49`) remet la même URL morte.
- ⚠ `server.rs:55-63` fait confiance à `fly-client-ip`/`x-forwarded-for` sans vérifier le proxy.
- ⚠ `room.rs` ferme le circuit quand la file du destinataire est pleine au lieu de ralentir
  l'émetteur (`route` + `try_send`), et `server.rs:167` ignore l'erreur : un récepteur lent fait
  perdre le transfert sans le dire.
- CI : `cargo llvm-cov --workspace --fail-under-lines 85` (`.github/workflows/ci.yml:63`).

**Décisions verrouillées :**

1. **Relais, pas proxy applicatif.** Le serveur ne termine pas le chiffrement : il relaie des
   trames opaques, déjà scellées de bout en bout. Il ne connaît ni nom de fichier, ni contenu, ni
   clés.
2. **Code émis par le serveur**, jamais choisi par le client : 8 caractères, alphabet sans
   ambiguïté (`23456789ABCDEFGHJKMNPQRSTVWXYZ`), affiché `XXXX-XXXX`. ~40 bits.
3. **TTL 5 min, renouvelable, plafonné.** À expiration, le code ne permet plus de rejoindre ; les
   pairs déjà connectés et les transferts en cours continuent. Le propriétaire régénère quand il
   veut ; l'ancien code meurt immédiatement. Durée de vie absolue d'une session : 1 h.
4. **Le code n'est pas une clé.** L'authentification de bout en bout reste le handshake + le SAS
   comparé par les deux personnes. ⚠ Ce n'est vrai que si l'émetteur voit aussi le SAS : c'est une
   tâche de S04, pas une hypothèse.
5. **Le serveur indexe les codes par hash BLAKE3** (pratique, pas sécurité : 40 bits se cassent
   hors ligne). La protection réelle est la limitation des tentatives + le TTL, et le plan le dit.
6. **Aucune socket relais tant qu'aucune session n'est demandée.** La case « Activer le relay »
   disparaît : l'action « créer/rejoindre » est le consentement.
7. **Le direct reste prioritaire** : `pick_transport` garde l'ordre direct → relais.
8. **Pas de hole punching en v1.** S07 est une optimisation optionnelle.
9. ⚠ **La session appartient à un `SessionToken` aléatoire de 256 bits**, pas au `device_id`
   (public via mDNS). Aucune décision destructive ne repose sur un identifiant fourni par le
   client. Le token n'est jamais journalisé.

---

## Contrat global

### Invariants
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` verts avant chaque commit.
- `#![forbid(unsafe_code)]` sur `core`, `net`, `relay`, `app`, `xtask`. ⚠ Exceptions existantes et
  documentées : `droplink-ios` et `droplink-ios-bridge` font `#![allow(unsafe_code)]` pour la
  frontière FFI ; **aucun nouveau `unsafe`** ailleurs.
- Aucun `Mutex` tenu à travers un `await` ; les verrous de `Rooms` restent `std::sync::Mutex` et
  les opérations synchrones (le chemin binaire clone le sender puis relâche le verrou).
- Canaux bornés, `try_send` côté contrôle ; **le payload relais utilise un `send().await` hors
  verrou** pour propager la contre-pression TCP (⚠ correction C4).
- Une constante de protocole = un seul endroit : `droplink-core/src/relay.rs`.
- UI en français, code et identifiants en anglais.
- Aucun code, token, identifiant de salle ou nom de fichier dans les logs du relais.
- Chaque règle de sécurité a un test qui la casse (code expiré, code inconnu, IP usurpée, force
  brute, rotation pendant un transfert, file pleine).

### Modèle de menace (résumé)
| Acteur | Peut | Ne peut pas |
|---|---|---|
| Relais curieux | voir IP, alias, tailles, durées | lire les fichiers, voir les noms, voir les clés |
| Relais malveillant | couper, rejouer, MITM actif | déchiffrer ; le SAS diffère et l'utilisateur refuse |
| Tiers sur Internet | deviner un code | 40 bits + 5 essais/min par /64 + budget global + TTL 5 min |
| Tiers sur le même Wi-Fi | voir le mDNS local | voir le trafic relais (WSS) |
| Tiers qui obtient le token | reprendre la session | sans le token, aucune reprise possible |

### Graphe de dépendances

```
S01 (core + génération de code)
 └─ S02 (serveur : sessions, token, limites, IP, contre-pression)
     ├─ S03 (client : sessions, moteur, migration des tests relais)
     │   └─ S04 (UI + SAS émetteur)
     └─ S05 (déploiement WSS)
         └─ S06 (E2E, charge, durcissement, docs)
             └─ S07 (option : punching)
```

| Vague | Étapes | Parallélisable ? |
|---|---|---|
| 1 | S01 | — |
| 2 | S02 | — |
| 3 | S03, S05 | oui : code client vs infra |
| 4 | S04 | — |
| 5 | S06 | — |
| 6 | S07 (option) | — |

> ⚠ S03 dépend de **S02** : son test d'intégration a besoin du serveur à sessions. La version
> précédente du plan les disait parallèles à tort.

---

# S01 — `droplink-core` : code de session, messages, fonctions pures

**Modèle :** le plus fort · **Dépend de :** — · **Rollback :** revert du commit.

### Contexte
Une « room » est aujourd'hui un identifiant public calculé par le serveur à partir d'un code
fourni par le client. On passe à une **session créée par le serveur**, identifiée par un UUID
aléatoire, possédée par un **token** opaque, accessible par un code éphémère. Tout le vocabulaire
et les règles testables vivent dans `droplink-core`, sans tokio ni I/O.

### Spécification normative

```rust
pub const SESSION_CODE_LEN: usize = 8;
pub const SESSION_CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTVWXYZ"; // 30 symboles
pub const SESSION_TTL_SECS: u64 = 300;
pub const SESSION_MAX_LIFETIME_SECS: u64 = 3600; // plafond absolu, refresh compris
pub const SESSION_OWNER_GRACE_SECS: u64 = 120;   // reprise possible après coupure
pub const SESSION_TOKEN_BYTES: usize = 32;
pub const SESSION_JOIN_ATTEMPTS_PER_MIN: u32 = 5;
pub const SESSION_JOIN_BURST: u32 = 5;
pub const SESSION_GLOBAL_ATTEMPTS_PER_SEC: u32 = 20;

pub type SessionId = String; // UUIDv4, jamais exposé au client

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    CreateSession { proto: u8, device_id: String, alias: String },
    JoinSession   { proto: u8, device_id: String, alias: String, code: String },
    ResumeSession { token: String },
    RefreshSession { token: String },
    LeaveSession,
    Connect { peer_id: PeerId },
    Close { circuit_id: CircuitId },
    // ⚠ `Register { room }` est supprimé : c'était le chemin « salle dérivée du code ou de l'IP ».
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg {
    SessionCreated { code: String, token: String, expires_at: u64, ttl_secs: u64 },
    SessionJoined  { expires_at: u64, ttl_secs: u64, peers: Vec<PeerSummary> },
    SessionResumed { code: String, expires_at: u64, ttl_secs: u64, peers: Vec<PeerSummary> },
    SessionEnded { reason: String },
    RoomUpdate { peers: Vec<PeerSummary> },
    // CircuitOpened / CircuitClosed / Error inchangés
}
```

Fonctions pures :

```rust
pub fn normalize_code(input: &str) -> Option<String>;
pub fn display_code(code: &str) -> String;                 // `ABCD-EFGH`
pub fn code_hash(code: &str) -> [u8; 32];                  // BLAKE3, domaine `droplink/v2/session`
pub fn generate_code(rng: &mut impl rand_core::RngCore) -> String; // ⚠ rand_core, pas rand
pub fn generate_token(rng: &mut impl rand_core::RngCore) -> String; // 32 octets -> base64url
pub fn token_hash(token: &str) -> [u8; 32];
```

Règles :
- `normalize_code` accepte minuscules, espaces, tirets ; refuse une longueur ≠ 8 ou un symbole
  hors alphabet (`I`, `L`, `O`, `U`, `0`, `1` interdits).
- `generate_code` utilise `SESSION_CODE_ALPHABET.len()` (⚠ pas `30` en dur) et rejette le biais
  modulo (`u32::MAX - (u32::MAX % len)`).
- Le token est 32 octets de `RngCore::fill_bytes`, encodé base64url sans padding.
- `PROTOCOL_VERSION` reste `2` ; `proto` dans les messages de session permet au serveur de refuser
  proprement un client d'une autre version.
- Documenter en tête de fichier : le hash du code est un **index**, pas un secret ; le token est
  une **capacité**.

### Tâches
- [ ] Constantes, `SessionId`, `ClientMsg`/`ServerMsg` ci-dessus ; suppression de `Register`.
- [ ] Les six fonctions pures + tests : alphabet, rejet de `0/1/I/L/O/U`, biais, affichage,
      stabilité du hash, token unique sur 1000 tirages, longueur/décodage base64url.
- [ ] Sérialisation JSON aller-retour de chaque message ; `proto` inconnu sérialisé tel quel.
- [ ] Test : un ancien JSON contenant `Register` ne désérialise plus en `ClientMsg` (échec net,
      pas de `continue` silencieux côté client).

### Vérification
```bash
cargo test -p droplink-core
cargo clippy -p droplink-core --all-targets -- -D warnings
cargo llvm-cov -p droplink-core --fail-under-lines 85
cargo fmt --all -- --check
```

### Critère de sortie
Types et fonctions présents, ≥ 8 tests, `cargo check --workspace` casse uniquement là où le
protocole est consommé (S02/S03), et c'est attendu.

---

# S02 — Relais : sessions, token, expiration, anti-force brute, IP, contre-pression

**Modèle :** le plus fort · **Dépend de :** S01 · **Rollback :** revert ; relais jamais déployé.

### Contexte
`Rooms` gère salles/plafonds/circuits. Il faut la création serveur, le join par code, l'expiration,
la reprise, et corriger trois défauts : l'IP est usurpable, la file pleine casse le transfert au
lieu de ralentir, et `try_send` ignore l'échec.

### Tâches

**1. Sessions (`crates/droplink-relay/src/room.rs`)**

```rust
struct Session {
    id: SessionId,
    code_hash: [u8; 32],
    token_hash: [u8; 32],
    created_at: u64,
    expires_at: u64,          // TTL du code courant
    owner: Option<PeerId>,    // None pendant la grâce
    owner_offline_at: Option<u64>,
    room: String,             // UUID aléatoire, jamais dérivé du code
}

// ⚠ signatures complètes : le pair est enregistré par la même fonction
pub fn create_session(
    &mut self, device_id: String, alias: String,
    tx: mpsc::Sender<Outbound>, now: u64, code: String, token: String,
) -> Result<(PeerId, SessionId, u64), RelayError>;

pub fn join_session(
    &mut self, code_hash: [u8; 32], device_id: String, alias: String,
    tx: mpsc::Sender<Outbound>, now: u64,
) -> Result<(PeerId, SessionId, u64), RelayError>;

pub fn resume_session(
    &mut self, token_hash: [u8; 32], tx: mpsc::Sender<Outbound>, now: u64,
    new_code: String,
) -> Result<(PeerId, SessionId, String, u64), RelayError>;

pub fn refresh_session(
    &mut self, token_hash: [u8; 32], now: u64, new_code: String,
) -> Result<(String, u64), RelayError>;

/// Ferme les codes/sessions expirés et renvoie les pairs à notifier (`SessionEnded`).
pub fn sweep(&mut self, now: u64) -> Vec<(PeerId, &'static str)>;
```

- [ ] `create_session` : refuse si `sessions.len() >= SESSION_MAX_ACTIVE` ; ⚠ si le `device_id` a
      déjà une session active, **refuse la nouvelle** (`already_active`) — ne ferme **jamais**
      l'ancienne (un ID public ne doit pas tuer un transfert).
- [ ] `join_session` : `no_session` uniforme si code inconnu ou expiré ; réutilise l'enregistrement
      interne du pair ; `duplicate_id` seulement si le `device_id` est déjà pair d'une autre salle.
- [ ] `resume_session` : token seul, dans la fenêtre `SESSION_OWNER_GRACE_SECS` ; émet un
      **nouveau code**, réarme le TTL, réattache `owner = Some(peer_id)`.
- [ ] `refresh_session` : propriétaire seulement (par token) ; nouveau code, `by_code` mis à jour,
      TTL réinitialisé, **plafonné par `created_at + SESSION_MAX_LIFETIME_SECS`** ; les pairs et
      circuits existants ne bougent pas.
- [ ] `leave_session` : l'invité part, la salle reste ; le propriétaire qui « termine » ferme la
      session, envoie `SessionEnded { reason: "closed" }`, puis désenregistre les invités.
- [ ] `unregister` : propriétaire déconnecté → `owner = None`, `owner_offline_at = now`, code
      invalidé (plus de join), session conservée pour la grâce ; invité → simple retrait.
      À l'expiration de la grâce, `sweep` ferme tout et notifie `SessionEnded`.
- [ ] `sweep(now)` : supprime les codes expirés, ferme les sessions hors grâce ou hors durée de
      vie absolue, libère `by_code`/`session_of_peer`, et renvoie les pairs à notifier.

**2. Limiteur (`crates/droplink-relay/src/limit.rs`, nouveau)**
- [ ] Bucket par **préfixe réseau** : /32 en IPv4, ⚠ **/64 en IPv6** (sinon un /64 contourne tout).
      Capacité `SESSION_JOIN_BURST`, débit `SESSION_JOIN_ATTEMPTS_PER_MIN`.
- [ ] Budget global `SESSION_GLOBAL_ATTEMPTS_PER_SEC` toutes IP confondues.
- [ ] Purge des préfixes inactifs (> 10 min) pour borner la mémoire.
- [ ] Échec uniforme `no_session` (inconnu **et** expiré). ⚠ Pas de délai artificiel : il ne tue
      aucun oracle (succès immédiat vs échec) et bloquerait la boucle de lecture synchrone.

**3. Anti-usurpation d'IP (`server.rs`)**
- [ ] Supprimer `fly-client-ip`.
- [ ] `client_ip(headers, addr, trusted_cidrs) -> IpAddr` : si `addr.ip()` n'est pas dans les CIDR
      de confiance, renvoyer `addr.ip()` et ignorer les en-têtes. Sinon lire `x-forwarded-for`
      ⚠ **de droite à gauche** en sautant les CIDR de confiance (le premier élément est contrôlé
      par le client).
- [ ] Caddy doit réécrire l'en-tête : `header_up X-Forwarded-For {remote_host}` (S05).
- [ ] Tests : en-tête forgé sans proxy → IP du socket ; en-tête forgé derrière proxy de confiance
      → IP réelle du dernier saut ; deux éléments `faux, vrai` → `vrai`.

**4. Contre-pression du payload (⚠ C4)**
- [ ] `Rooms::route` devient `async` : sous verrou, retrouver le `mpsc::Sender` du destinataire et
      le cloner ; **relâcher le verrou** ; `send(frame).await`.
- [ ] `handle_socket` attend `route` sur le chemin binaire ; si le send échoue, envoyer
      `CircuitClosed { reason: "backpressure" }` à l'émetteur et fermer le circuit.
- [ ] Profondeur de file par socket : `RELAY_QUEUE_FRAMES = 4` (au lieu de 64) et budget global
      d'octets en attente `RELAY_QUEUE_BUDGET_BYTES = 64 MiB` : au-delà, fermer le circuit le plus
      lent avec `CircuitClosed` (compteur exposé dans `/healthz`).
- [ ] Test de charge : récepteur throttlé (lecture 1 Mio/s), émetteur 20 Mio, vérifier que le
      transfert aboutit (ou échoue proprement avec `CircuitClosed`), jamais de blocage muet ; et
      que la mémoire du relais reste sous le budget.

**5. Boucle de purge et santé**
- [ ] Tâche toutes les 5 s : `rooms.sweep(now)`, envoie `SessionEnded` aux pairs renvoyés.
- [ ] `/healthz` : `{ sessions, peers, circuits, queued_bytes, join_failures }` (aucun secret).

**6. Tests (`tests/ws_relay.rs` + unités)**
- [ ] Créer → rejoindre → `RoomUpdate` réciproque ; code inconnu → `no_session`.
- [ ] 6e tentative/min depuis un /64 → `rate_limited` ; un autre /64 n'est pas affecté.
- [ ] TTL : injecter `now` (⚠ pas de `sleep`), avancer de 301 s, join refusé, pairs connectés OK.
- [ ] `RefreshSession` invalide l'ancien code, en émet un nouveau, ne touche pas aux pairs.
- [ ] Propriétaire déconnecté → invités toujours là, join refusé ; `ResumeSession` dans la grâce →
      nouveau code ; après la grâce → `SessionEnded`.
- [ ] `CreateSession` avec un `device_id` déjà actif → `already_active`, l'ancienne session et son
      transfert survivent.
- [ ] Purge : après `sweep`, `by_code` et `session_of_peer` ne fuient pas.

### Vérification
```bash
cargo test -p droplink-relay
cargo clippy -p droplink-relay --all-targets -- -D warnings
cargo llvm-cov -p droplink-relay --fail-under-lines 85
# vérification des logs : aucun code/token/salle
rg -n "code|token|room" crates/droplink-relay/src
```

### Critère de sortie
Toutes les règles ont un test. `/healthz` expose les compteurs. Un test prouve qu'une file pleine
propage la contre-pression au lieu de casser le transfert.

---

# S03 — Client : cycle de vie d'une session, migration du relais

**Modèle :** défaut · **Dépend de :** S02 · **Rollback :** revert ; le relais reste désactivé.

### Contexte
`RelayClient::connect` envoie `Register` implicitement, et `engine.rs` démarre le relais au boot
si `relay_enabled`. Il faut une connexion **paresseuse** pilotée par une action, et un client qui
sait créer/rejoindre/reprendre/renouveler/quitter. Les tests `tests/relay_transfer.rs` et
`benches/relay.rs` reposent sur l'ancien chemin : ils doivent être migrés dans la même étape,
sinon `cargo test -p droplink-net` est rouge.

### Tâches
- [ ] `relay_client.rs` : supprimer le `Register` implicite de `connect` ; envoyer explicitement
      `CreateSession`/`JoinSession`/`ResumeSession`/`RefreshSession`/`LeaveSession` ; nouveaux
      `RelayEvent` ; ⚠ un `ServerMsg` inconnu doit produire une erreur visible, pas `continue`.
- [ ] `engine.rs` :
  - `Command::SessionCreate`, `SessionJoin { code }`, `SessionResume { token }`,
    `SessionRefresh`, `SessionEnd` ;
  - ⚠ `start_relay` n'est appelé qu'à la première commande de session (retirer l'appel
    `engine.rs:387-389`) ; `relay_enabled` n'est plus qu'un champ de compatibilité ignoré ;
  - reconnexion avec backoff **tant que la session est active** : le propriétaire rejoue
    `ResumeSession { token }`, l'invité rejoue `JoinSession { code }` (le code peut avoir tourné :
    alors `SessionEnded`, l'UI le dit) ;
  - `Event::Session { code, token, expires_at, ttl_secs, owner }`, `Event::SessionError`,
    `Event::SessionEnded` ;
  - ⚠ `relay_url = None` → repli explicite sur `DEFAULT_RELAY_URL` dans `start_relay` ; une seule
    règle, `EngineConfig::new` aligné (plus d'URL Fly morte en dur).
- [ ] `Config` : ⚠ garder `room_code: Option<String>` avec `#[serde(default)]` (sinon un champ
      manquant régénère le `device_id` via `default_config()`) ; le vider en mémoire ; test « JSON
      sans `room_code` → `device_id` inchangé ». Ajouter `#[serde(default)]` sur tout nouveau
      champ.
- [ ] `session_transfer.rs` (nouveau) : relais local, `CreateSession` → `JoinSession` →
      transfert 2 Mio → hash identique ; code faux → `no_session` ; TTL injecté → join refusé.
- [ ] ⚠ Migrer `tests/relay_transfer.rs` et `benches/relay.rs` : `Node::start` doit créer la
      session et rejoindre, puis attendre `SessionCreated`/`SessionJoined`.

### Vérification
```bash
cargo test -p droplink-net
cargo clippy -p droplink-net --all-targets -- -D warnings
cargo llvm-cov -p droplink-net --fail-under-lines 85
git grep -n "relay_enabled" -- crates   # plus aucun démarrage au boot
```

### Critère de sortie
Un transfert passe **par le relais seul**, sans mDNS, prouvé par comparaison BLAKE3. Aucune socket
relais n'est ouverte sans commande de session.

---

# S04 — UI : sessions, compte à rebours, SAS émetteur

**Modèle :** défaut · **Dépend de :** S03 · **Rollback :** revert ; mDNS et direct intacts.

### Contexte
Il faut une surface dédiée, lisible sur iPhone comme sur desktop, et ⚠ le SAS côté émetteur, sans
lequel le modèle de menace est faux.

### Tâches
- [ ] `UiState` : `session: Option<SessionView { code, token, expires_at, ttl_secs, owner, error }>`,
      `join_code: String` ; compte à rebours calculé depuis `expires_at` et la durée annoncée
      (`ttl_secs`), jamais depuis un compteur local.
- [ ] `ui/session.rs` (nouveau), sous la section `pairs` :
  - sans session : `[ créer un code ]`, `[ rejoindre ]`, champ code (8 caractères, normalisation
    à la saisie, affichage `XXXX-XXXX`).
  - propriétaire : code en gros monospace, `expire dans 04:12`, `[ copier ]`, `[ nouveau code ]`,
    `[ terminer ]` ; iOS : `[ partager ]` via le bridge.
  - invité : `connecté`, pairs distants avec l'étiquette `relay`, `[ quitter ]`.
  - états : `connexion…`, `code expiré — génère un nouveau code`, `code inconnu ou expiré`,
    `relais injoignable`, `session reprise`. Jamais de blocage du fil.
- [ ] ⚠ **SAS émetteur** : afficher le SAS dans la ligne de transfert sortante (`ui/queue.rs`) dès
      `Event::Sas`, avec « compare ce code avec le destinataire ». Sans ça, un relais MITM n'est
      pas détecté.
- [ ] Retirer « Activer le relay » et « Code de room » de `ui/settings.rs` (garder l'URL du relais
      en champ avancé) ; supprimer `room_code` de `SettingsDraft`.
- [ ] `droplink-ios-bridge` : `share_text(text)` (UIActivityViewController) ; compilation iOS
      vérifiée, pas de test UI.
- [ ] `app.rs` : actions UI → `Command::Session*` ; fermer la session à la fermeture de la
      fenêtre ; documenter que l'app doit rester au premier plan sur iOS (la session survit
      2 min à une coupure, le transfert en cours non).
- [ ] `ui/render_tests.rs` : session propriétaire, invitée, code expiré, formulaire vide, SAS
      sortant.

### Vérification
```bash
cargo test -p droplink-app
cargo clippy -p droplink-app --all-targets -- -D warnings
cd ios && xcodebuild -project DropLink.xcodeproj -scheme DropLink -sdk iphoneos \
  -destination 'generic/platform=iOS' -configuration Debug -allowProvisioningUpdates \
  -derivedDataPath build-device build
```

### Critère de sortie
Code affiché, copié, partagé ; compte à rebours juste après une mise en veille ; SAS visible des
deux côtés ; aucun champ « room » résiduel.

---

# S05 — Déploiement du relais (WSS, TLS, proxy de confiance)

**Modèle :** défaut · **Dépend de :** S02 · **Rollback :** arrêter le service et retirer le DNS ;
puis, si des clients pointent déjà dessus, remettre `DEFAULT_RELAY_URL` à `None` et livrer.

### Contexte
`wss://droplink-relay.fly.dev/ws` est mort. Il faut une cible réelle avec TLS, un proxy qui réécrit
`X-Forwarded-For`, et un dimensionnement mémoire cohérent avec les files.

### Tâches
- [ ] `deploy/relay.service` : `DynamicUser=yes`, `NoNewPrivileges=yes`, `ProtectSystem=strict`,
      `MemoryMax=1G` (⚠ cohérent avec `RELAY_QUEUE_FRAMES=4` × 512 sockets + budget 64 Mio),
      `Environment=PORT=8080`, `Environment=TRUSTED_PROXY_CIDRS=127.0.0.1/32`.
- [ ] `deploy/Caddyfile` :
      `relay.droplink.app { reverse_proxy 127.0.0.1:8080 { header_up X-Forwarded-For {remote_host} flush_interval -1 } }`.
      ⚠ `flush_interval -1` est requis pour le streaming WebSocket ; la limite de corps HTTP ne
      s'applique pas après le 101.
- [ ] `deploy/README.md` : DNS, `systemctl`, `journalctl`, rotation, mise à jour, vérification
      `curl -fsS https://…/healthz`.
- [ ] Client : `DEFAULT_RELAY_URL = "wss://relay.droplink.app/ws"`, utilisé seulement à la demande.
- [ ] Test de fumée : deux réseaux, d'abord relais local, puis déployé ; transférer 20 Mio à
      travers Caddy (pas seulement `/healthz`) et vérifier le hash.

### Vérification
```bash
curl -fsS https://relay.droplink.app/healthz
cargo test -p droplink-relay --test ws_relay
# transfert réel à travers Caddy, hash comparé
```

### Critère de sortie
`/healthz` HTTPS public, `TRUSTED_PROXY_CIDRS` limité à la loopback, transfert de 20 Mio à travers
Caddy réussi.

---

# S06 — Bout en bout, charge, durcissement, documentation

**Modèle :** le plus fort · **Dépend de :** S03, S04, S05 · **Rollback :** revert de l'UI ; le
relais reste déployé mais inutilisé.

### Tâches
- [ ] `tests/session_e2e.rs` : relais local, deux moteurs au niveau `droplink-net` (⚠ la découverte
      est lancée par `droplink-app`, pas par `spawn`), session, code, transfert 100 Mio, BLAKE3
      identique, progression monotone, annulation à mi-parcours laisse le pair utilisable.
- [ ] ⚠ Test de contre-pression : récepteur throttlé, transfert 100 Mio, mémoire relais sous le
      budget, aucun blocage muet.
- [ ] Test de coupure : relais arrêté pendant le transfert → `Event::Failed` ; relais relancé →
      `ResumeSession` (propriétaire) ou `SessionEnded` (invité) ; nouveau transfert possible.
- [ ] ⚠ Test rotation pendant transfert : 8 Mio en cours + `RefreshSession` → hash identique,
      circuit non fermé.
- [ ] Test force brute : 100 joins faux depuis un /64 → ≤ 5 traités puis `rate_limited`, autre /64
      intact ; budget global respecté.
- [ ] Test logs : capturer `tracing` sur un cycle complet (création, join, transfert, expiration)
      et vérifier qu'aucun code/token/nom de fichier n'apparaît.
- [ ] Docs : `SECURITY.md` (relais, token, SAS), `THREAT-MODEL.md` (énumération, DoS, coût de
      bande passante, ⚠ partage du code par messagerie = rétention tierce), `ARCHITECTURE.md`
      (sessions vs rooms), `README.md`, `docs/REMOTE.md` (procédure, dépannage, quand préférer le
      direct, garder l'app au premier plan sur iOS).
- [ ] Observabilité : compteurs `join_failures`, `sessions_active`, `bytes_relayed` exposés dans
      `/healthz` ; pas de nouvelle dépendance.

### Vérification
```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo llvm-cov --workspace --fail-under-lines 85 --ignore-filename-regex 'main\.rs$|app\.rs$|xtask/'
# manuel : Mac Wi-Fi + iPhone 4G, 100 Mio, hash comparé, date/version consignées
```

### Critère de sortie
Parcours prouvé automatiquement et manuellement, mémoire bornée sous charge, logs propres.

---

# S07 — Option : passage direct après rendez-vous (hole punching)

**Modèle :** le plus fort · **Dépend de :** S06 · **Rollback :** supprimer la tentative directe.

### Tâches
- [ ] `ServerMsg::PeerAddress { ip, port }` (adresse observée), seulement pour les pairs d'une
      même session.
- [ ] Le client tente **un** direct avec délai court (1,5 s) avant d'ouvrir le circuit relais ;
      ⚠ jamais deux handshakes simultanés (sinon double `Incoming` et double réception).
- [ ] `Reach::Both` construit après succès ; `pick_transport` déjà prêt.
- [ ] Test : même machine → `Direct` ; relais coupé avant le handshake → `Relay`.
- [ ] Documenter la limite : NAT symétrique / CGNAT mobile → relais.

### Critère de sortie
Direct quand possible, relais sinon, hash identique, un seul transfert par fichier.

---

## Catalogue d'anti-patrons (à rejeter en revue)

1. Code choisi par le client → collisions et énumération. Le serveur génère.
2. Code dans l'URL, le chemin, un log ou une capture → fuite. Jamais.
3. Salle dérivée du code ou de l'IP → code corrélable, voisins du NAT mélangés. Salle = UUID.
4. Code en clair côté serveur → dump mémoire. Index par hash (sans prétendre que c'est un secret).
5. Rotation qui coupe les pairs connectés → un transfert meurt à 5 min. Rotation = join seulement.
6. `X-Forwarded-For` cru par défaut ou premier élément lu → plafonds contournés.
7. Réponses distinctes « inconnu » / « expiré » → oracle d'énumération.
8. Relais activé au démarrage → viole « zéro réseau par défaut ».
9. Nom de fichier ou taille utile en clair côté relais → le handshake est chiffré, ne rien ajouter.
10. Code utilisé comme authentification → MITM. Le SAS comparé des **deux** côtés est la preuve.
11. Timer UI local → dérive. Compter depuis `expires_at` + `ttl_secs`.
12. Test E2E avec mDNS actif → chemin ambigu.
13. `try_send` sur le payload → perte silencieuse. `send().await` hors verrou.
14. Décision destructive sur un `device_id` public → kill à distance. Le token possède la session.
15. File de 64 trames × 512 sockets → OOM. Profondeur 4 + budget d'octets.

## Critères de complétion (mesurables)

1. Deux réseaux distincts (Mac Wi-Fi + iPhone 4G) : code créé, saisi, fichier de 100 Mio, BLAKE3
   identique des deux côtés.
2. Code expiré à 300 s (± 5 s) : join → `no_session`. Pairs déjà connectés : transfert toujours
   possible.
3. `RefreshSession` invalide l'ancien code immédiatement et ne coupe aucun transfert en cours
   (test 8 Mio).
4. 100 joins faux depuis un /64 : ≤ 5 traités puis `rate_limited`, autre /64 intact ; budget global
   respecté ; mémoire relais < budget.
5. Aucun log ne contient code, token, salle ou nom de fichier (test + `rg`).
6. `cargo test --workspace`, clippy `-D warnings`, fmt, couverture ≥ 85 % verts.
7. Relais en `wss://` derrière TLS, `/healthz` public, redémarrage automatique, transfert de
   20 Mio à travers Caddy.
8. Aucune socket vers le relais sans action de session (test « 10 s sans commande »).

## Risques et parades

| Risque | Parade |
|---|---|
| Coût de bande passante | plafond de sessions, budget d'octets, direct prioritaire, S07 punching |
| Force brute de codes | 40 bits + 5/min par /64 + budget global + TTL + échec uniforme |
| Relais compromis | AEAD de bout en bout + SAS comparé des deux côtés |
| Kill à distance | session possédée par token, `already_active` au lieu de fermer |
| OOM par files | profondeur 4, budget 64 Mio, `MemoryMax=1G`, test de charge |
| iOS en arrière-plan | grâce propriétaire 2 min (le transfert en cours ne survit pas) ; documenté |
| Code partagé par messagerie | fuite tierce assumée et documentée ; le code expire en 5 min |
