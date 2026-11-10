# DropLink sur iOS

## État

L'app iOS native est construite à partir du même code Rust que le desktop :

```
crates/droplink-app     UI egui + moteur (partagé)
crates/droplink-ios     shim FFI (staticlib, point d'entrée C)
crates/droplink-ios-bridge  pont Rust ↔ Swift (Bonjour, picker)
ios/                    projet Xcode (xcodegen)
```

- Rendu : `eframe` + `wgpu` (Metal).
- Découverte : mDNS (`mdns-sd`) **et** Bonjour (`NSNetService` pour publier, `NWBrowser` pour parcourir).
- Sélection de fichier : `UIDocumentPickerViewController` via Swift, copie dans le sandbox puis envoi.
- Réception : `Documents/DropLink` (visible dans Fichiers grâce à `UIFileSharingEnabled`).
- **Destination** : sélecteur natif de dossier (`UIDocumentPicker` mode folder), mémorisé par bookmark.
  `cfg → Destination → [ choisir ]`, et `[ fichiers ]` ouvre l'app Fichiers.
- **Export automatique** : images et vidéos → **Photos** (`NSPhotoLibraryAddUsageDescription`) ; le reste →
  copié dans le dossier choisi (ou `Documents/DropLink` si aucun). La copie de travail reste dans Documents
  pour que l'historique puisse toujours ouvrir le fichier.

## Prérequis

- Xcode avec un SDK iOS.
- Cibles Rust : `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`.
- `brew install xcodegen`.
- Un iPhone connecté et **déverrouillé** (l'image développeur ne se monte pas sur un appareil verrouillé).

## Signature

Le projet est configuré pour la signature automatique avec l'équipe personnelle :

```yaml
DEVELOPMENT_TEAM: 8R2MC75QS5
PRODUCT_BUNDLE_IDENTIFIER: com.droplink.desktop
```

Pour une autre équipe, modifie `DEVELOPMENT_TEAM` dans `ios/project.yml`, puis :

```bash
cd ios && xcodegen generate
```

Au premier lancement, l'iPhone demande de faire confiance au certificat :
**Réglages → Général → VPN et gestion de l'appareil → Apple Development → Faire confiance**.
Avec une équipe personnelle, l'app expire après 7 jours.

## Build et installation

```bash
# Simulateur
cd ios
xcodegen generate
xcodebuild -project DropLink.xcodeproj -scheme DropLink \
  -sdk iphonesimulator -configuration Debug \
  -derivedDataPath build build

# Appareil
xcodebuild -project DropLink.xcodeproj -scheme DropLink \
  -sdk iphoneos -destination 'generic/platform=iOS' \
  -configuration Debug -allowProvisioningUpdates \
  -derivedDataPath build-device build

xcrun devicectl device install app --device <UDID> \
  build-device/Build/Products/Debug-iphoneos/DropLink.app
xcrun devicectl device process launch --device <UDID> com.droplink.desktop
```

Le script de build Xcode compile automatiquement `libdroplink_ios.a` pour la bonne cible
(`aarch64-apple-ios` ou `-sim`).

## Réseau : le point critique

Deux appareils ne peuvent se découvrir que s'ils sont **sur le même lien réseau** :

- iOS interdit le multicast brut sans l'entitlement Apple `com.apple.developer.networking.multicast`.
  La publication passe donc par Bonjour (`NSNetService`), qui est autorisé sans entitlement.
- Bonjour ne traverse pas deux réseaux différents : un Mac sur un autre Wi-Fi, en VPN ou sur un
  réseau invité ne verra pas l'iPhone.
- Le **partage de connexion** iPhone (`172.20.10.0/28`) filtre le multicast de façon
  intermittente : le pair apparaît puis disparaît. Les deux appareils restent joignables en TCP.

Deux chemins, dans cet ordre :

1. **Découverte auto** (mDNS/Bonjour) — même routeur Wi-Fi, ou partage de connexion quand le
   multicast passe.
2. **Connexion directe** — les deux apps affichent `cette machine : <ip>:<port>` sous la liste des
   pairs. Sur l'autre appareil, saisir cette adresse dans le champ `direct` puis `connecter`.
   C'est le chemin fiable sur partage de connexion, Wi-Fi invité ou réseau qui filtre mDNS.

Vérifier le sous-réseau côté Mac :

```bash
ipconfig getifaddr en0     # 172.20.10.x = client du partage de connexion iPhone
```

## Logs

Les logs de l'app iOS sont dans son conteneur :

```bash
xcrun devicectl device copy from --device <UDID> \
  --domain-type appDataContainer \
  --domain-identifier com.droplink.desktop \
  --source "Library/Application Support/DropLink/droplink.log" \
  --destination ios.log
```

## Dépannage iOS

| Symptôme | Cause | Solution |
|---|---|---|
| `Operation not permitted` à la réception | chemin de sandbox d'une ancienne installation | corrigé : le dossier est recalculé à chaque démarrage |
| Le fichier n'est pas dans Fichiers | `Documents` vide ou app non visible | `UIFileSharingEnabled` est activé ; chercher `Sur mon iPhone > DropLink` |
| Photos ne reçoit rien | permission « Ajouter des photos » refusée | Réglages → DropLink → Photos → Ajouter |
| Aucun pair | iPhone et Mac sur deux réseaux différents, ou multicast filtré | même routeur Wi-Fi, ou connexion directe `ip:port` |
| Le pair apparaît puis disparaît | partage de connexion qui filtre le multicast | connexion directe `ip:port` |
| L'app iPhone ne publie rien | build installé obsolète ou permission « Réseau local » refusée | réinstaller l'app, Réglages → DropLink → Réseau local |
| Profil non approuvé | première installation | Réglages → Général → VPN et gestion de l'appareil → Faire confiance |

## Limites connues

- L'app iOS ne peut pas encore envoyer vers un pair trouvé uniquement en Bonjour tant que
  l'iPhone est sur un autre réseau que le Mac.
- Pas d'entitlement multicast : la publication mDNS brute est remplacée par Bonjour.
- Pas d'icône d'app ni de thème sombre natif spécifique iOS.
