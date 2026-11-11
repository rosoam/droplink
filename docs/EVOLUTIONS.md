# Évolutions possibles — du transfert de fichiers au messager et à la voix

> Ce document décrit ce que le socle DropLink peut devenir, ce qu'il faut construire pour chaque
> étape, ce que ça coûte, et ce qui reste hors de portée à court terme. Il ne remplace pas le plan
> WebRTC (`WEBRTC-PLAN.md`) : il montre où ce plan mène.

## 1. Principe

DropLink est un **transport pair-à-pair chiffré**, pas un compte, pas un cloud. Le jour où il
traverse les réseaux, il peut porter autre chose que des fichiers : des messages, de la voix, de
la vidéo. Le tuyau est réutilisable ; ce qui change à chaque étape, c'est l'identité, le
stockage hors ligne et l'interface.

Trois principes à conserver :

1. **Le contenu reste chiffré de bout en bout.** Le serveur ne voit que du ciphertext.
2. **Le LAN reste sans serveur.** Un serveur n'apparaît que pour le hors-ligne ou les groupes.
3. **L'identité est vérifiable.** Un code court comparé à l'écran vaut mieux qu'un compte.

## 2. Ce que le socle WebRTC fournit déjà

| Brique | Usage fichier | Réutilisable pour |
|---|---|---|
| Signaling (code, offer/answer/ICE) | se rencontrer | messages, appels, présence |
| ICE / STUN / TURN | traverser les NAT | appels, messages hors LAN |
| DTLS + AEAD + SAS | confidentialité, identité | messages, appels (SRTP) |
| Data channel | octets fiables | messages, contrôle d'appel |
| Transfert de fichiers | pièces jointes | photos, vocaux, documents |

## 3. Carte des évolutions

| Évolution | Complexité | Serveur | Coût | Dépend de |
|---|---|---|---|---|
| Fichiers à distance | moyenne | signaling + TURN optionnel | faible | plan WebRTC |
| Contacts + identité longue durée | moyenne | aucun | 0 | fichiers |
| Messagerie quand les deux sont en ligne | moyenne | signaling | faible | identité |
| Messagerie hors ligne | élevée | boîte aux lettres + push | moyen | identité + messages |
| Appel audio/vidéo 1:1 | élevée | signaling + TURN | moyen | appels WebRTC |
| Groupes (messages et voix) | très élevée | MLS + SFU | élevé | 1:1 |
| Multi-appareils | très élevée | synchronisation | moyen | identité |

« Faible » = quelques euros par mois. « Moyen » = dizaines d'euros. « Élevé » = centaines, selon
l'usage.

## 4. La brique fondatrice : l'identité

Aujourd'hui, chaque transfert a un SAS éphémère. Un messager ne peut pas demander de comparer six
chiffres à chaque message. Il faut une identité durable.

### À construire

- **Clé d'identité** par appareil : Ed25519 pour signer, X25519 pour l'échange.
- **Carnet de contacts** : identifiant, clé publique, nom local.
- **Vérification** : empreinte courte ou QR, à la première rencontre (TOFU), puis alerte si la clé
  change.
- **Numéro de sécurité** : suite de chiffres dérivée des deux clés, comparable de vive voix.
- **Rotation** : clé d'appareil révocable, appareil perdu.

### Ce qu'on ne fait pas tout de suite

- Annuaire mondial (key transparency) : utile à grande échelle, lourd.
- Comptes email/téléphone : DropLink peut rester sans compte, avec des identifiants de contact.

### Livrable

`docs/IDENTITY.md` + module `droplink-core::identity` + écran « contacts ». Aucun serveur.

## 5. Messagerie en ligne

Quand les deux appareils sont connectés (même LAN ou WebRTC), un message est un petit paquet sur
le data channel.

### À construire

- Enveloppe : `{ conversation_id, message_id, timestamp, kind, body_cipher }`.
- Protocole de session : **Double Ratchet** (Signal) ou MLS pour plusieurs appareils.
- Accusés de réception, ordre, doublons, reprise.
- Pièces jointes : réutiliser le transfert de fichiers existant.
- UI : conversations, non-lus, recherche locale, suppression.

### Ce que ça ne demande pas

Pas de serveur supplémentaire : le signaling sert déjà à se trouver, le data channel à échanger.

### Limite

Si l'autre est hors ligne, il n'y a pas de data channel. D'où l'étape suivante.

## 6. Messagerie hors ligne

C'est le premier vrai backend. Il ne stocke que du chiffré et un minimum de métadonnées.

### À construire

- **Boîte aux lettres** : `POST /messages` (ciphertext, destinataire, TTL), `GET /messages`.
- **Identifiants** : clé publique de l'appareil, pas d'email obligatoire.
- **File d'attente** : un message par destinataire, TTL 7–30 jours.
- **Push** : APNs pour iOS, notifications Windows/Linux ; le payload ne contient pas le texte.
- **Suppression** : à la lecture, à l'expiration, ou sur demande.
- **Anti-abus** : quotas, rate limit, signalement.

### Vie privée

Le serveur voit : qui écrit à quelle clé, quand, la taille. Il ne voit ni le texte ni les noms.
C'est le modèle de Signal, pas celui d'un cloud de documents.

### Coût

Un petit VPS + Postgres, 5–15 €/mois au début. Le push est gratuit (APNs/FCM).

## 7. Appels audio/vidéo 1:1

WebRTC est né pour ça. Le data channel devient un canal média (SRTP), le reste ne change pas.

### À construire

- Capture micro/caméra, permissions iOS/macOS/Windows/Linux.
- Codecs : Opus (audio), VP8/VP9/AV1 (vidéo) ; négociation via WebRTC.
- UI d'appel : sonnerie, décrocher, raccrocher, muet, haut-parleur.
- iOS : CallKit + PushKit pour recevoir un appel en arrière-plan ; AVAudioSession.
- Windows/Linux : périphériques audio, notifications.
- TURN sérieux : un appel passe mal sans relais quand le NAT est strict.
- Chiffrement : DTLS-SRTP natif ; le SAS reste possible pour vérifier l'identité.

### Ce que ça coûte

TURN : la bande passante d'un appel (64–200 kbps audio, 1–4 Mbps vidéo). Un TURN à 1 To/mois tient
longtemps pour de l'audio.

### Limite

Un appel de groupe n'est pas du 1:1 multiplié : il faut un SFU.

## 8. Groupes : messages et voix

- **Messages de groupe** : MLS (Messaging Layer Security) pour les clés, un serveur de boîte aux
  lettres par groupe.
- **Voix/vidéo de groupe** : SFU (LiveKit, mediasoup, Janus) qui reçoit un flux et le redistribue.
  C'est un serveur coûteux en CPU et en bande passante.
- **Modération** : invitations, rôles, bannissement, signalement.

C'est un produit à part entière. À ne pas mettre dans la première version.

## 9. Multi-appareils

Un utilisateur, plusieurs appareils (iPhone + Mac + PC). Chaque appareil a sa clé ; les messages
doivent être chiffrés pour chaque appareil destinataire.

- MLS gère nativement les groupes d'appareils.
- Sinon : chiffrer la clé de conversation pour chaque appareil.
- Synchronisation de l'historique : chiffrée, via la boîte aux lettres ou un export local.

C'est la partie la plus délicate d'un messager. À planifier après le 1:1.

## 10. Coûts et infrastructure

| Brique | Quand | Ordre de prix |
|---|---|---|
| Signaling | fichiers à distance | 5–10 €/mois |
| TURN | NAT stricts, appels | bande passante |
| Boîte aux lettres + push | messagerie hors ligne | 10–30 €/mois |
| SFU | appels de groupe | 50–300 €/mois selon usage |
| Domaine, TLS | toujours | inclus Caddy |

Tant qu'on reste en LAN ou en test local : **zéro**.

## 11. Conformité et vie privée

Un messager qui stocke des métadonnées change de catégorie :

- politique de rétention (7–30 jours) ;
- droit à l'effacement ;
- signalement d'abus ;
- transparence sur ce que le serveur voit ;
- chiffrement des sauvegardes ;
- RGPD / LPD suisse selon le marché.

Le chiffrement de bout en bout ne dispense pas de ces obligations.

## 12. Feuille de route proposée

| Phase | Livrable | Durée | Gate |
|---|---|---|---|
| P0 | Fichiers à distance (plan WebRTC) | 3–4 sem. | 100 Mio iPhone ↔ Windows/Linux |
| P1 | Identité + contacts + vérification | 2–4 sem. | deux appareils se reconnaissent sans SAS par transfert |
| P2 | Messagerie en ligne | 2–3 sem. | messages ordonnés, accusés, pièces jointes |
| P3 | Messagerie hors ligne + push | 4–6 sem. | message reçu app fermée |
| P4 | Appel 1:1 audio puis vidéo | 4–6 sem. | appel iPhone ↔ Windows |
| P5 | Groupes | 8+ sem. | à décider |

Chaque phase est indépendante : on peut s'arrêter après P0 ou P2 sans dette.

## 13. Décisions à prendre

1. **Destination** : rester un outil de transfert, ou devenir un messager ?
2. **Identité** : sans compte (clés/QR) ou avec un identifiant (email/téléphone) ?
3. **Hors ligne** : accepter un serveur de boîte aux lettres, ou rester en ligne seulement ?
4. **Appels** : 1:1 seulement, ou groupes plus tard ?
5. **Multi-appareils** : dès le départ ou après le 1:1 ?

## 14. Risques

| Risque | Conséquence | Parade |
|---|---|---|
| Identité bâclée | usurpation, contacts faux | clés signées, vérification, rotation |
| Crypto maison | failles | Double Ratchet/MLS, pas d'invention |
| Serveur hors ligne | métadonnées, coût | minimisation, TTL, chiffrement |
| Appels sans TURN | échecs NAT | TURN mesuré |
| Groupes trop tôt | complexité, coût | gate P4 |
| Dérive produit | retard sur P0 | une phase à la fois |

## 15. Résumé

- **Fichiers à distance** : le socle, déjà planifié.
- **Messagerie** : possible, mais l'identité et le hors-ligne sont le vrai travail.
- **Appels 1:1** : WebRTC le fait ; l'audio, les permissions et le TURN sont le travail.
- **Groupes** : possible, mais produit séparé avec SFU.
- **Impossible** : rien. Simplement des produits successifs, avec des serveurs et des coûts qui
  arrivent au fur et à mesure.
