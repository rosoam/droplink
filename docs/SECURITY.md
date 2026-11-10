# Sécurité — DropLink

## Ce qui est protégé

Chaque transfert utilise un canal chiffré et authentifié de bout en bout :

```
A = X25519 éphémère (expéditeur)        B = X25519 éphémère (récepteur)
shared     = X25519(A_secret, B_public)          — rejeté si non contributif (clé de petit ordre)
transcript = BLAKE3("droplink/v2" || hello_canon || B_public)
key_s2r    = HKDF-SHA256(shared, salt=transcript, info="droplink/s2r")
key_r2s    = HKDF-SHA256(shared, salt=transcript, info="droplink/r2s")
SAS        = 6 chiffres dérivés de BLAKE3("droplink/v2" || transcript || "/sas")
```

- **Confidentialité** : ChaCha20-Poly1305, une clé **par direction**, compteur de nonce distinct par
  direction. Aucune réutilisation de nonce n'est possible : chaque direction possède sa clé et son
  compteur monotone.
- **Intégrité** : tout le trafic après `KeyOffer` est scellé, y compris `Accept`, `Refuse`, `End`, `Done`
  et `Error`. Un attaquant ne peut pas forger un « transfert terminé ».
- **Intégrité du fichier** : BLAKE3 calculé en streaming pendant l'envoi, transporté dans `End` scellé,
  vérifié à la réception avant de renommer `.part` vers le nom final. Un échec supprime le `.part`.
- **Anti-MITM** : le `transcript` couvre la version, le `transfer_id`, la taille, la taille de chunk, le
  nom de fichier, l'alias et la clé publique éphémère. Un attaquant qui modifie les métadonnées produit un
  transcript différent, donc une clé différente et un SAS différent.
- **SAS** : les deux pairs affichent le même code à 6 chiffres. S'ils diffèrent, il y a un homme du
  milieu. Le SAS est affiché **avant** la décision d'accepter : le `Accept` est la première frame scellée,
  donc accepter valide le canal authentifié.
- **Identité locale** : `device_id` est un UUID persistant généré localement ; ni le nom ni l'IP ne
  servent de clé.

## Ce qui n'est pas protégé

- **Un utilisateur qui ne compare pas le SAS** accepte un canal potentiellement intercepté. Le SAS est
  informatif : la décision reste humaine.
- **La disponibilité** : un attaquant sur le réseau peut couper un transfert (DoS). Il ne peut ni lire
  ni modifier les données.
- **Les métadonnées réseau** : mDNS diffuse le nom et l'IP sur le LAN. Le relay voit l'IP publique et
  l'alias, jamais le contenu, le nom de fichier ni le `device_id`.
- **Les fichiers déjà écrits** : un fichier reçu et vérifié est un fichier normal dans
  `~/Downloads/DropLink`. DropLink ne l'analyse pas (pas d'antivirus intégré).

## Le relay ne peut pas déchiffrer

Le relay ne reçoit que :

- des messages de contrôle (identifiants de circuit, alias),
- des frames binaires `[circuit_id 16 octets || ciphertext AEAD]`.

Les clés sont dérivées localement par les deux pairs via X25519. Le relay n'a aucune clé, aucun nonce,
aucun `transcript`. Le test `relay_no_plaintext_leak` vérifie qu'un marqueur ASCII présent dans le fichier
n'apparaît jamais dans les frames relayées.

## Limites connues

- Room du relay : par défaut dérivée de l'IP publique (hashée). Derrière un CGNAT, plusieurs foyers
  partagent une room. Les pairs n'y voient qu'un `peer_id` et un alias, et chaque connexion exige une
  action explicite de l'utilisateur. Un `room_code` explicite permet d'isoler un groupe.
- Le SAS n'est pas un secret partagé : c'est un contrôle d'intégrité de l'échange, à comparer de visu.
- Aucune persistance des clés : chaque transfert utilise un échange éphémère.

## Relais de session (2026-11)

- Le relais ne termine jamais le chiffrement : trames AEAD opaques, aucune clé, aucun nom.
- Code de session : 8 caractères, ~40 bits, émis par le serveur, TTL 5 min, plafond 1 h.
- Index serveur par hash BLAKE3 : ce n'est **pas** un secret (40 bits se cassent hors ligne).
  La protection est la limite de 5 essais/min par /64 IPv6 et /32 IPv4, plus un budget global.
- Token de session 256 bits, propriétaire comme invité : reprise après coupure sans dépendre du
  `device_id` (public via mDNS).
- `X-Forwarded-For` n'est lu que derrière un proxy de confiance, de droite à gauche ; Caddy
  réécrit l'en-tête.
- SAS à 6 chiffres affiché des deux côtés avant acceptation : un relais MITM le fait diverger.
