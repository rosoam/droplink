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

Deux appareils ne peuvent se voir que s'ils sont **sur le même réseau L2** :

- iOS interdit le multicast brut sans l'entitlement Apple `com.apple.developer.networking.multicast`.
  La publication passe donc par Bonjour (`NSNetService`), qui est autorisé sans entitlement.
- Bonjour ne traverse pas deux réseaux différents. En particulier, un Mac connecté au
  **partage de connexion** de l'iPhone (`172.20.10.x`) n'est pas sur le même réseau que le Wi-Fi
  de l'iPhone (`10.x` ou `192.168.x`).

Configuration correcte : les deux appareils sur le **même routeur Wi-Fi**. Vérifier :

```bash
ipconfig getifaddr en0     # Mac
```

et l'IP affichée par l'app iPhone (ou dans les logs). Les deux doivent être dans le même sous-réseau.

## Logs

Les logs de l'app iOS sont dans son conteneur :

```bash
xcrun devicectl device copy from --device <UDID> \
  --domain-type appDataContainer \
  --domain-identifier com.droplink.desktop \
  --source "Library/Application Support/DropLink/droplink.log" \
  --destination ios.log
```

## Limites connues

- L'app iOS ne peut pas encore envoyer vers un pair trouvé uniquement en Bonjour tant que
  l'iPhone est sur un autre réseau que le Mac.
- Pas d'entitlement multicast : la publication mDNS brute est remplacée par Bonjour.
- Pas d'icône d'app ni de thème sombre natif spécifique iOS.
