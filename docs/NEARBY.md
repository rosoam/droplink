# Échanger des fichiers à distance sans relais — audit des options radio

> Question : peut-on envoyer un fichier à quelqu'un « à une certaine distance » sans serveur, avec
> une technologie qui sort du LAN classique ? Réponse courte : oui sur Apple, jusqu'à quelques
> dizaines de mètres, avec **MultipeerConnectivity / AWDL** (la famille d'AirDrop). Au-delà, aucune
> radio de téléphone ne tient la charge ; il faut de l'infrastructure (relais, WebRTC/TURN) ou un
> débit minuscule (LoRa).

## 1. Contraintes physiques et plateformes

| Technologie | Portée | Débit utile | iOS | macOS | Windows/Linux | Verdict fichier |
|---|---|---|---|---|---|---|
| Wi-Fi classique (routeur) | maison | 100–1000 Mbps | oui | oui | oui | déjà géré (LAN) |
| **MultipeerConnectivity / AWDL** | 10–50 m | 10–100 Mbps | oui | oui | non | **meilleur candidat Apple** |
| Wi-Fi Direct | 10–50 m | 50–200 Mbps | **non exposé** | non | Android | impossible sur iPhone |
| Wi-Fi Aware (NAN) | 10–50 m | 10–100 Mbps | non (iOS 26 partiel) | non | Android | trop tôt |
| Bluetooth LE | 10–50 m | 0,05–2 Mbps | oui | oui | oui | appairage, petits fichiers |
| UWB (Nearby Interaction) | < 10 m | **aucun débit** | oui | non | non | localisation seule |
| LoRa / radio longue portée | km | < 0,05 Mbps | via module | via module | oui | inutilisable pour des photos |
| WebRTC data channel + STUN | Internet | 1–100 Mbps | oui | oui | oui | nécessite un signaling (petit serveur) |
| Relais DropLink | Internet | dépend du serveur | oui | oui | oui | déjà conçu, non déployé |

Ce qui n'existe pas : un protocole radio de téléphone qui traverse plusieurs kilomètres à haut débit
sans infrastructure. Les « technologies révolutionnaires » (Li-Fi, mmWave, satellite) supposent du
matériel dédié ou un opérateur.

## 2. Le candidat sérieux : MultipeerConnectivity (AWDL)

C'est le mécanisme d'AirDrop :

- découverte par Bluetooth LE + Wi-Fi peer-to-peer (AWDL) ;
- session chiffrée par Apple (TLS) entre appareils ;
- flux fiables (`MCSession.startStream`) ou messages bornés ;
- fonctionne **sans routeur**, entre deux iPhone, ou iPhone ↔ Mac ;
- portée réelle 10–30 m, parfois 50 m en visibilité directe ;
- API publique, autorisée sur l'App Store ; pas d'entitlement spécial.

Ce que ça change pour DropLink :

```
découverte MC ──> pair "nearby"
transport MC ──> flux d'octets
        │
        └── on garde Hello / X25519 / AEAD / BLAKE3 / SAS par-dessus le flux MC
```

Le chiffrement DropLink reste : MC ajoute son propre chiffrement de lien, mais le handshake et le
SAS garantissent l'identité de bout en bout, comme aujourd'hui.

### Architecture dans le code

- `droplink-net` : nouveau `transport/nearby.rs` qui expose un flux `AsyncRead + AsyncWrite`.
- Pont Swift `droplink-ios-bridge` (et un équivalent macOS) :
  - `MCNearbyServiceAdvertiser` / `MCNearbyServiceBrowser` ;
  - `MCSession` + `startStream(withPeer:)` ;
  - callbacks C vers Rust pour les octets et la pression.
- `Peer::Reach` : nouvelle variante `Nearby { peer_name }` ou transport séparé, pour ne pas
  mélanger avec `Direct { ip, port }`.
- `pick_transport` : LAN direct → nearby → manuel → relais (plus tard).
- UI : la liste `appareils` affiche aussi les pairs nearby, sans code ni IP.

### Effort et risques

- Pont Swift ↔ Rust : ~400–600 lignes, plus le transport Rust.
- Pas testable au simulateur : il faut deux appareils réels (on a un iPhone et un Mac).
- Débit variable selon la distance et l'interférence ; à mesurer.
- iOS demande les permissions Bluetooth et réseau local ; à expliquer dans l'UI.
- macOS : `MultipeerConnectivity` existe mais est moins utilisé ; à valider sur le Mac.
- Pas de Windows/Linux : le jour où ils arrivent, il faudra un autre transport (Wi-Fi Direct
  Android, WebRTC ailleurs).

### Test de validation

1. iPhone ↔ Mac, Wi-Fi et Bluetooth activés, **aucun routeur** (les deux en mode avion puis
   Wi-Fi/BT seuls, ou réseau différent).
2. Distance 1 m, 10 m, 20 m, 30 m.
3. Transfert de 10 Mo puis 100 Mo ; débit, hash, stabilité.
4. Coupure d'un appareil en cours de transfert → échec propre, paire toujours visible.

## 3. L'alternative multi-plateforme : WebRTC

Si la cible inclut Windows, Linux ou Android :

- signalisation minimale (un petit service HTTP/WebSocket qui échange les offres SDP) ;
- STUN public pour découvrir l'adresse publique ;
- hole punching UDP ; TURN en secours quand le NAT est symétrique ;
- data channel binaire, chiffré par DTLS (on peut garder notre AEAD par-dessus).

C'est standard et puissant, mais ce n'est plus « sans serveur » : il faut un signaling, et un TURN
pour les cas difficiles. C'est le relais sous une autre forme, avec un meilleur débit.

## 4. Options écartées

- **Wi-Fi Direct** : Android seulement. iOS ne l'expose pas.
- **Hotspot programmatique** : iOS ne permet pas de créer un point d'accès depuis une app.
- **AirDrop automatique** : aucune API pour déclencher un envoi AirDrop.
- **UWB** : donne la direction et la distance, pas de bande passante.
- **LoRa / satellite** : portée énorme, débit négligeable ; bon pour un ping, pas pour un fichier.
- **Ultrason / QR** : appairage uniquement.

## 5. Recommandation

**Phase 1 — Apple d'abord (réaliste, démontrable) :**
ajouter MultipeerConnectivity comme transport « nearby ». Objectif : iPhone ↔ Mac, sans routeur,
10 Mo à 20 m, hash identique, en gardant le handshake et le SAS actuels.

**Phase 2 — bascule automatique :**
`pick_transport` essaie LAN, puis nearby, puis l'adresse manuelle. L'UI montre le mode utilisé
(`lan`, `nearby`, `direct`) sans bruit.

**Phase 3 — Internet (plus tard) :**
WebRTC + signaling, ou le relais déjà spécifié. À décider quand la cible multi-plateforme sera
arrêtée.

## 6. Décision nécessaire

1. **Périmètre** : Apple uniquement (iPhone + Mac) pour la phase 1, ou multi-plateforme dès le
   départ ?
2. **Distance attendue** : quelques dizaines de mètres (MC) ou kilomètres (relais/WebRTC) ?
3. **Test réel** : deux appareils disponibles au même endroit pour valider 20–30 m ?

Tant que le périmètre est Apple, MultipeerConnectivity est la seule voie propre. Si le besoin est
« deux appareils n'importe où sur Internet », aucune radio ne le fait : il faut un serveur, et le
relais déjà spécifié reste la réponse.
