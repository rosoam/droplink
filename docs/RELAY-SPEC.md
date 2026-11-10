# Relais DropLink — spécification et audit avant mise en ligne

> État au commit `f562cf0` (2026-11-10). Le code et les tests existent ; le service n'est pas
> déployé. Ce document liste ce que le relais doit tenir, ce qu'il tient déjà, ce qui bloque une
> exposition publique, et les informations qu'il faut fournir pour commander l'hébergement.

## 1. Verdict

Le relais fait ce qu'on lui demande : il relaie des trames opaques, gère des sessions éphémères,
applique des limites et de la contre-pression. Preuves : `session_transfer` (2 Mio, code faux,
rotation en cours de transfert), `session_transfer_100mib` (100 Mio, hash identique), reconnexion
propriétaire/invité, 100 tentatives de join limitées, logs sans secret, 205 tests, couverture
88 %.

**Il n'est pas déployable tel quel.** Quatre points P0 :

1. il écoute sur `0.0.0.0:8080` (`crates/droplink-relay/src/main.rs:18`) : si le pare-feu n'est pas
   fermé, on contourne Caddy, le TLS et la réécriture `X-Forwarded-For` ;
2. la taille maximale d'un message WebSocket n'est pas bornée : le défaut de tungstenite est
   64 Mio, alors que le relais ne veut que 1 Mio + 16 ;
3. aucune quota d'octets par session, par appareil ou par jour : n'importe qui peut faire
   transiter des téraoctets ;
4. aucun compteur de facturation : `/healthz` donne l'instant, pas le total du jour.

## 2. Ce que le relais doit faire

| Fonction | État |
|---|---|
| WebSocket `wss://…/ws`, messages de contrôle JSON | fait |
| Trames binaires `[circuit uuid 16 octets ‖ ciphertext]` | fait |
| Créer / rejoindre / reprendre / renouveler / quitter une session | fait |
| Code 8 caractères, TTL 5 min, plafond 1 h, grâce 2 min | fait |
| Token propriétaire et invité (reprise après coupure) | fait |
| Circuits 1:1 dans une même session, aucun contenu visible | fait |
| Contre-pression de bout en bout (file pleine = émetteur ralenti) | fait |
| Limites par réseau (/64 IPv6, /32 IPv4) et globales | fait |
| Aucune persistance, aucun compte, aucun stockage de fichier | fait, par choix |
| TLS, domaine, pare-feu, quotas d'octets, métriques | **à faire** |

Hors périmètre : déchiffrement, stockage, recherche de pairs, hole punching, comptes.

## 3. Limites actuelles (extraites du code)

| Constante | Valeur | Effet |
|---|---|---|
| `MAX_RELAY_FRAME` | 1 Mio + 16 | taille max d'une trame acceptée par `decode_binary` |
| `DEFAULT_CHUNK` (client) | 256 Kio | taille réellement émise par DropLink |
| `RELAY_QUEUE_FRAMES` | 4 | trames en attente par socket |
| `MAX_GLOBAL_SOCKETS` | 512 | sockets simultanées |
| `MAX_SOCKETS_PER_IP` | 8 | sockets par IP |
| `MAX_PEERS_PER_ROOM` | 32 | pairs par session |
| `MAX_CIRCUITS_PER_PEER` | 4 | circuits simultanés par pair |
| `MAX_CONTROL_PER_SECOND` | 10 | messages de contrôle par seconde et par socket |
| `HEARTBEAT_SECS` / `IDLE_TIMEOUT_SECS` | 15 / 60 | ping et coupure d'inactivité |
| `SESSION_CODE_LEN` | 8 | 30⁸ ≈ 6,6 × 10¹¹ codes (~39 bits) |
| `SESSION_TTL_SECS` | 300 | validité d'un code |
| `SESSION_MAX_LIFETIME_SECS` | 3600 | durée de vie absolue d'une session |
| `SESSION_OWNER_GRACE_SECS` | 120 | reprise du propriétaire après coupure |
| `SESSION_JOIN_ATTEMPTS_PER_MIN` / `BURST` | 5 / 5 | essais par réseau |
| `SESSION_GLOBAL_ATTEMPTS_PER_SEC` | 20 | budget global d'essais |
| `MAX_FILE_SIZE` (client) | 64 Gio | plafond protocolaire, pas relais |

## 4. Dimensionnement

### Mémoire

Formule pire cas : `sockets × trames en file × taille de trame`.

- Avec les réglages actuels : `512 × 4 × 1 Mio = 2 Gio`, alors que `deploy/relay.service` fixe
  `MemoryMax=1G`. Un client hostile qui envoie des trames de 1 Mio peut donc provoquer un OOM.
- Avec la trame ramenée à 256 Kio + 16 (taille réelle du client) : `512 × 4 × 256 Kio = 512 Mio`.
- Avec une file de 2 trames et 256 Kio : `256 Mio`.

Recommandation : `MAX_RELAY_FRAME = 256 Kio + 16`, `RELAY_QUEUE_FRAMES = 4`, `MemoryMax=1G`,
puis 2 Go de RAM sur la machine pour l'OS, Caddy et les pics.

### Connexions

512 sockets = **256 sessions 1:1 simultanées** (2 sockets par transfert), ou moins si des salles
ont plus de deux pairs. C'est suffisant pour un lancement privé ; pour 1 000 transferts
simultanés il faut deux instances ou un relais par région.

### Bande passante

Le relais paie l'entrée **et** la sortie : un fichier de 100 Mio consomme ~100 Mio d'ingress et
~100 Mio d'egress. En pratique, seul l'egress est facturé ou plafonné chez la plupart des hôtes.

| Volume relayé | Egress | Équivalent |
|---|---|---|
| 100 Gio/mois | 100 Gio | ~1 000 transferts de 100 Mio |
| 1 Tio/mois | 1 Tio | ~10 000 transferts de 100 Mio |
| 5 Tio/mois | 5 Tio | ~50 000 transferts de 100 Mio |

Un VPS à 20 To de trafic inclus couvre largement les débuts. Le risque n'est pas la capacité,
c'est l'abus : sans quota, un seul utilisateur peut consommer plusieurs téraoctets.

### CPU / disque

- 2 vCPU suffisent pour ~1 Gbit/s de TLS + forwarding ; le relais ne fait que copier des octets.
- 20–40 Go de disque, pas de données persistantes, journaux systemd sur 2 semaines.
- 2 Go de RAM recommandés (1 Go possible avec les limites corrigées).

## 5. Audit sécurité et abus

### P0 — avant toute exposition publique

1. **Écoute locale.** `main.rs` doit lire `BIND` (défaut `127.0.0.1`) et le service doit tourner
   derrière Caddy. Ajouter `ufw default deny incoming`, `ufw allow 80,443/tcp`, ne jamais ouvrir
   8080.
2. **Taille des messages WebSocket.** Configurer `WebSocketUpgrade::max_message_size` et
   `max_frame_size` à `MAX_RELAY_FRAME + 16`, et ramener `MAX_RELAY_FRAME` à 256 Kio + 16. Tester
   qu'un message de 1 Mio est refusé avant allocation.
3. **Quotas d'octets.** Sans eux, le relais est un proxy de fichiers gratuit et illimité.
   Ajouter : plafond par transfert (ex. 2 Gio), par appareil/jour (ex. 10 Gio), par IP/jour
   (ex. 20 Gio), global/jour (ex. 500 Gio) et global/mois (ex. 5 Tio). Refuser proprement
   (`Error { code: "quota" }`) et journaliser le total sans code ni contenu.
4. **Comptabilité.** `/healthz` ne donne que l'instant. Ajouter des compteurs monotones
   (`bytes_relayed_total`, `sessions_total`, `join_failures_total`) et un compteur journalier
   remis à zéro à minuit UTC, exposés sur `/metrics` (texte Prometheus) ou dans `/healthz`.
   Sans ça, impossible de savoir ce que le relais coûte.

### P1 — avant bêta publique

5. `MemoryMax=1G` incohérent avec le pire cas mémoire (voir §4) : corriger les constantes avant.
6. **Instance unique.** Les sessions vivent en mémoire ; un redémarrage coupe les transferts en
   cours. Les clients reprennent la session (tokens) mais pas le transfert. À documenter, et à
   garder pour la v1.
7. **Pas de drainage.** `systemctl restart` coupe tout. Acceptable en bêta ; prévoir une fenêtre.
8. **Caddy et WebSocket.** Tester un transfert de 100 Mio à travers Caddy avec `encode zstd gzip`
   activé ; si le streaming souffre, retirer `encode` sur `/ws`.
9. **Pare-feu et mises à jour.** Documenter `ufw`, `unattended-upgrades`, rotation journald,
   renouvellement du certificat (Caddy le fait, vérifier l'alerte).

### P2 — plus tard

10. Métriques par région, latence p95, taux de coupure.
11. Deux instances + répartition par région (les salles sont en mémoire, il faut un routage
    sticky ou un registre partagé).
12. Quota par appareil (pas seulement par IP) si les comptes arrivent.

## 6. Quotas proposés (à valider)

| Quota | Proposition | Raison |
|---|---|---|
| Taille d'un fichier | 2 Gio | au-delà, le relais coûte trop cher ; le direct reste possible |
| Par session | 5 Gio | évite un transfert monstre unique |
| Par appareil / jour | 10 Gio | freine un utilisateur |
| Par IP / jour | 20 Gio | freine un petit botnet |
| Global / jour | 500 Gio | ~5 000 transferts de 100 Mio |
| Global / mois | 5 Tio | plafond de facture |
| Sessions actives | 256 (512 sockets) | limite actuelle |
| Création de code | 5/min/réseau | déjà en place |

Les seuils sont des propositions de départ, pas des dogmes. Ils doivent être configurables par
variables d'environnement, avec des valeurs par défaut dans le code.

## 7. Hébergement

| Option | Avantages | Inconvénients | Ordre de prix |
|---|---|---|---|
| Hetzner (DE/FI) | 20 To inclus, 2 vCPU/4 Go, fiable | hors Suisse | ~5–8 €/mois |
| Infomaniak (CH) | résidence suisse, facturation CHF | trafic souvent plus cher | ~15–30 CHF/mois |
| Exoscale (CH) | suisse, API propre | prix au-dessus | ~15–30 CHF/mois |
| Fly.io | déploiement rapide, régions | egress facturé ~0,02 $/Go | variable, ~10–30 $/mois |
| Scaleway (FR) | bon rapport prix | hors Suisse | ~5–10 €/mois |

Le relais ne voit que des IP, des alias, des tailles et des durées. Le contenu est chiffré de bout
en bout. La résidence suisse est donc un choix de conformité/commercial, pas une nécessité
technique.

**Configuration minimale recommandée :** 2 vCPU, 2–4 Go RAM, 40 Go SSD, 1 Gbit/s, 20 To de trafic,
Ubuntu 24.04 LTS, IPv4 + IPv6, Caddy 2.8+, systemd.

## 8. Déploiement (une fois l'hôte fourni)

```bash
# 1. binaire
cargo build --release -p droplink-relay
install -m 0755 target/release/droplink-relay /usr/local/bin/droplink-relay

# 2. service et proxy
install -m 0644 deploy/relay.service /etc/systemd/system/droplink-relay.service
install -m 0644 deploy/Caddyfile /etc/caddy/Caddyfile
systemctl daemon-reload && systemctl enable --now droplink-relay
systemctl reload caddy

# 3. pare-feu
ufw default deny incoming
ufw allow 22/tcp
ufw allow 80,443/tcp
ufw enable

# 4. vérification
curl -fsS https://relay.droplink.app/healthz
```

Puis mettre `DEFAULT_RELAY_URL` sur `wss://relay.droplink.app/ws` et livrer une version.

## 9. Plan de test avant ouverture

1. **Local** : `session_transfer` (2 Mio), `--ignored` (100 Mio), `relay_transfer` (8 Mio,
   reconnexion), `session_e2e`.
2. **Charge** : 256 sockets ouverts, 20 transferts simultanés de 10 Mio, mémoire < 1 Gio,
   aucun drop.
3. **Lecture lente** : récepteur throttlé à 1 Mio/s, transfert de 100 Mio, contre-pression,
   pas de coupure muette.
4. **Abus** : 100 joins depuis un /64 (5 traités), message WebSocket de 1 Mio refusé, quota
   dépassé → `quota`.
5. **Inter-réseaux** : Mac Wi-Fi + iPhone 5G, code, transfert de 100 Mio, hash identique, temps
   mural et octets relevés sur le serveur (`vnstat`, `/metrics`).
6. **Panne** : redémarrage du relais en cours de transfert → échec propre, reprise de session
   au retour, nouveau transfert possible.

## 10. Informations nécessaires pour commander

1. **Domaine** : `relay.droplink.app` (ou un autre). Qui contrôle le DNS ? Si le domaine n'est
   pas acheté, il faut le choisir.
2. **Hôte** : fournisseur, région, accès SSH (ou création par toi puis clé). Hébergement suisse
   obligatoire ou non ?
3. **Budget** : plafond mensuel (5 €, 20 €, 50 € ?) et volume de trafic toléré.
4. **Quotas** : valider ou ajuster ceux du §6.
5. **Public** : bêta privée (quota serré, quelques utilisateurs) ou ouverture (métriques et
   alertes avant).
6. **Contact abus** : adresse à publier dans la politique d'usage et pour les signalements.
7. **Observabilité** : `/healthz` suffit au début, ou on ajoute `/metrics` Prometheus et une
   sonde externe ?

## 11. Ce qui est déjà prouvé

```
cargo test --workspace          205 tests
cargo llvm-cov …               88,03 %
session_transfer_100mib        ok (100 Mio, BLAKE3 identique)
one_hundred_join_attempts      ok (5 no_session, 95 rate_limited)
logs_do_not_leak               ok
curl /healthz                  {"status":"ok",…,"bytes_relayed":0}
```
