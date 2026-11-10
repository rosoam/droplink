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


## 7. Contrainte ajoutée : Windows, Linux, macOS + iPhone

Le périmètre est maintenant : ordinateurs Windows/Linux/macOS, téléphones iPhone. MultipeerConnectivity
ne couvre que les appareils Apple ; il ne peut donc pas être la technologie unique.

| Combinaison | mDNS/TCP LAN | MultipeerConnectivity | Wi-Fi Direct | Bluetooth LE | WebRTC / relais |
|---|---|---|---|---|---|
| iPhone ↔ iPhone | oui (même LAN) | oui | non | lent | oui |
| iPhone ↔ macOS | oui | oui | non | lent | oui |
| iPhone ↔ Windows | oui (même LAN) | **non** | non | lent | oui |
| iPhone ↔ Linux | oui (même LAN) | **non** | non | lent | oui |
| Windows ↔ Linux | oui | non | partiel | lent | oui |
| Windows ↔ macOS | oui | non | partiel | lent | oui |
| macOS ↔ macOS | oui | oui | partiel | lent | oui |

Conclusion : dès qu'on inclut Windows et Linux, **aucune radio peer-to-peer native ne couvre tout**.
Il ne reste que deux familles :

1. **le LAN** (mDNS + TCP), qui marche sur les trois OS et l'iPhone quand tout le monde est sur le
   même réseau — c'est déjà le cas dans le code ;
2. **le transport par Internet**, qui exige un rendez-vous : relais applicatif, ou WebRTC avec
   signaling + STUN/TURN.

### Pas de solution « sans serveur » à distance

- Wi-Fi Direct n'est pas exposé par iOS.
- L'iPhone ne crée pas de hotspot depuis une app.
- L'ad-hoc Wi-Fi (IBSS) n'existe pas sur iOS.
- Le Bluetooth plafonne à quelques Mbps : bon pour appairer, pas pour une photo de 10 Mo.
- L'UWB ne transporte pas de données.
- LoRa/satellite : portée énorme, débit dérisoire.

Donc, à distance et sans serveur, il n'y a rien de crédible pour des fichiers. Le choix réel est :

| Approche | Serveur | Quand les données passent | Coût |
|---|---|---|---|
| LAN direct | aucun | même réseau | 0 |
| Relais applicatif | oui, data path | toujours | bande passante |
| WebRTC + hole punching | signaling + STUN, TURN en secours | P2P la plupart du temps, TURN sinon | signaling quasi nul, TURN en secours |
| VPN/Tailscale | oui (existant) | toujours | dépend du VPN |

**Recommandation :** garder le LAN direct tel quel (il fonctionne déjà sur macOS, iOS, et compile
sur Windows/Linux), puis ajouter WebRTC pour Internet. Le relais applicatif déjà spécifié peut
servir de signaling et de secours TURN, ce qui évite d'exploiter deux infrastructures.

Décision à prendre : accepte-t-on un **petit serveur de signaling** (quelques kilo-octets par
connexion, pas les fichiers) ? Si non, la seule alternative à distance reste le VPN ou la
redirection de port manuelle.

## 8. Recherche 2025–2026 : Wi-Fi Aware (NAN), la vraie nouveauté

Wi-Fi Aware (Neighbor Awareness Networking) est le standard Wi-Fi Alliance derrière les
« AirDrop alternatifs ». iOS 26 l'expose aux apps.

Ce qu'Apple documente :

- découverte et connexion directe **sans point d'accès ni Internet** ;
- iPhone 12 et plus, iPad récents ;
- appairage obligatoire, code PIN à 6 chiffres (`DeviceDiscoveryUI`) ;
- connexions chiffrées au niveau Wi-Fi, haut débit, faible latence ;
- cohabite avec le Wi-Fi normal (l'iPhone reste connecté à son réseau) ;
- entitlement `com.apple.developer.wifi-aware` à demander à Apple, services déclarés dans
  `Info.plist` ;
- transport via `NetworkListener` / `NetworkBrowser` / `NetworkConnection`, sockets IPv6.

C'est exactement le « sans routeur, haute vitesse, quelques dizaines de mètres » recherché. Mais le
support par OS est incomplet :

| OS | Wi-Fi Aware (NAN) | Source |
|---|---|---|
| iOS 26+ (iPhone 12+) | **oui**, entitlement requis | Apple WWDC25 / docs |
| iPadOS 26+ | oui | Apple |
| macOS 26 (app native) | **non** : l'API publique `WACapabilities` n'est pas disponible au type-check ; seuls iOS/iPadOS (et Mac Catalyst) sont exposés | test SDK macOS 26 |
| Windows 10/11 | **non** : aucune API NAN publique, pas de roadmap | Microsoft Q&A |
| Linux | **émergent** : patches cfg80211/wpa_supplicant NAN 2025–2026, matériel compatible requis | linux-wireless / hostap |

Conséquence pour le parc demandé (Windows, Linux, macOS, iPhone) : Wi-Fi Aware ne peut pas être la
technologie unique. Il couvre iPhone ↔ iPhone, et à terme iPhone ↔ Linux avec une carte NAN. Pas
Windows, pas macOS natif.

### Ce que ça vaut quand même

- iPhone ↔ iPhone : le meilleur transport local, sans routeur, haut débit.
- iPhone ↔ Linux (matériel NAN) : possible à moyen terme.
- iPhone ↔ macOS : retomber sur MultipeerConnectivity/AWDL.
- Windows : LAN ou WebRTC.

## 9. Les idées « code » et « Bluetooth seul »

### Un code peut-il porter le signaling ?

Oui, partiellement. Un QR/code peut transporter une offre SDP et des candidats ICE (version
« non-trickle »), mais :

- il faut **deux** messages (offre puis réponse), donc deux codes ou un aller-retour ;
- il faut quand même **STUN** pour connaître l'adresse publique derrière le NAT ;
- sans TURN, les NAT symétriques et certains réseaux mobiles échouent ;
- un seul code ne peut pas, à lui seul, établir une connexion bidirectionnelle fiable.

Donc un code remplace le **transport du signaling**, pas le rendez-vous NAT. C'est une solution de
dépannage pour utilisateurs techniques, pas une UX grand public.

### Bluetooth seul ?

Mesures iOS réelles : L2CAP/GATT plafonnent autour de **20–60 Ko/s** sur iPhone (quelques centaines
de kbps au mieux, selon les appareils et la configuration). Un JPEG de 5 Mo prend 1,5 à 4 minutes ;
une vidéo est hors sujet.

Le BLE est bon pour :

- découvrir et appairer ;
- échanger les métadonnées d'un autre transport ;
- envoyer du texte ou une petite photo, en dernier recours.

Pas pour le transfert de fichiers sérieux.

## 10. Synthèse : un portefeuille de transports

Aucune technologie unique ne couvre Windows + Linux + macOS + iPhone, à distance, sans serveur.

| Besoin | Transport |
|---|---|
| Même réseau | mDNS + TCP direct (déjà là, tous OS) |
| iPhone ↔ iPhone sans réseau | Wi-Fi Aware (iOS 26, entitlement) |
| iPhone ↔ macOS sans réseau | MultipeerConnectivity/AWDL |
| iPhone ↔ Linux sans réseau | Wi-Fi Aware si matériel NAN, sinon LAN |
| Windows ↔ quoi que ce soit, à distance | WebRTC (signaling + STUN/TURN) ou relais |
| Partout, secours | relais applicatif |
| Appairage / petits fichiers | BLE |

La « révolution » réaliste n'est pas une radio unique : c'est un **sélecteur de transport** qui
prend le meilleur disponible pour la paire d'appareils, sans que l'utilisateur ait à choisir.

## Sources

- Apple, *Supercharge device connectivity with Wi-Fi Aware* (WWDC25) — https://developer.apple.com/videos/play/wwdc2025/228/
- Apple, *Wi-Fi Aware* — https://developer.apple.com/documentation/wifiaware
- MacRumors, *iOS 26 Adding Two New Wi-Fi Features* — https://www.macrumors.com/2025/06/21/ios-26-adding-two-new-wi-fi-features/
- Test SDK macOS 26 (API indisponible) — https://labee.jp/posts/macos-wifi-aware-api-boundary
- Microsoft Q&A, *Does Windows have plans to support Wi-Fi Aware* — https://learn.microsoft.com/en-us/answers/questions/2284386/does-windows-have-plans-to-support-wifi-aware
- Linux NAN data path — https://lwn.net/Articles/1059533/
- Débit BLE iOS — https://interrupt.memfault.com/blog/ble-throughput-primer et retours CoreBluetooth
