# iPhone ↔ Windows / Linux / macOS — état et mode d'emploi

## Ce qui marche aujourd'hui

| Paire | Même réseau | Réseaux différents |
|---|---|---|
| iPhone ↔ macOS | oui (testé, mDNS + adresse) | non (WebRTC à faire) |
| iPhone ↔ Windows | attendu, pas encore testé sur machine réelle | non |
| iPhone ↔ Linux | attendu, pas encore testé sur machine réelle | non |
| Windows ↔ Linux | attendu (même LAN) | non |
| macOS ↔ macOS | oui | non |

« Attendu » signifie : le code compile et passe les tests sur Windows/Linux, la découverte et le
transfert sont les mêmes que sur macOS, mais aucun essai réel n'a été fait sur ces deux OS.

## Construire les binaires

La CI manuelle produit un binaire par OS :

```
GitHub → Actions → CI → Run workflow
```

Artefacts :

- `droplink-macos-aarch64`
- `droplink-windows-x86_64.exe`
- `droplink-linux-x86_64`

Aucun installateur, aucun compte, aucun serveur.

## Windows

1. Décompresser l'exécutable.
2. Au premier lancement, Windows demande l'accès réseau : autoriser sur **réseau privé**
   (pas public).
3. Si le réseau est classé « Public », le pare-feu bloque les connexions entrantes :
   Paramètres → Réseau → propriétés → Profil privé.
4. Dans DropLink : `connexion → adresse` affiche `cette machine : <ip>:<port>`.
5. Sur l'iPhone, même Wi-Fi, `connexion → adresse`, saisir cette adresse.

## Linux

Dépendances d'exécution (Ubuntu/Debian) :

```bash
sudo apt install libgtk-3-0 libxkbcommon0 libwayland-client0 libgl1 libasound2
chmod +x droplink-linux-x86_64
./droplink-linux-x86_64
```

Pare-feu : le port est aléatoire par défaut. Le fixer dans les réglages (ex. `7777`), puis :

```bash
sudo ufw allow 7777/tcp
```

Même procédure : `adresse` sur le Linux, saisie sur l'iPhone.

## macOS

Même parcours. Gatekeeper peut bloquer un binaire non signé :

```bash
xattr -d com.apple.quarantine droplink-macos-aarch64
```

## Pourquoi l'adresse manuelle est le chemin sûr

Le mDNS peut être filtré (partage de connexion, Wi-Fi invité, pare-feu Windows, avahi absent).
L'adresse `ip:port` ne dépend d'aucun multicast : elle traverse tout réseau local qui autorise le
TCP. C'est le mode à utiliser pour les premiers tests iPhone ↔ Windows/Linux.

## Ce qui ne marche pas encore

- **Réseaux différents** : pas de WebRTC branché, pas de signaling. Le LAN seulement.
- **Android** : hors périmètre.
- **Découverte automatique** sur Windows/Linux : à vérifier sur machine réelle ; l'adresse
  manuelle reste le fallback.

## Prochaine étape

WebRTC (`str0m` pour iOS + desktop) pour les réseaux différents. Tant que ce n'est pas fait, la
réponse à « iPhone ↔ Windows à distance » est **non**.
