# Envoyer un fichier à distance

Deux appareils, deux réseaux, aucun compte. L'un crée un code, l'autre le saisit, le fichier part
par un relais qui ne voit que du chiffrement.

## Parcours

1. **Hôte** : `pairs → [ connexion ]`, onglet `code`, puis `[ créer un code ]`. Le code s'affiche (`ABCD-2345`) avec un compte à
   rebours de 5 minutes. `[ copier ]` ou, sur iPhone, `[ partager ]`.
2. **Invité** : `pairs → [ connexion ]`, onglet `code`, saisit le code puis `[ rejoindre ]`. Les deux apparaissent
   dans la liste `pairs` avec l'étiquette `relay`.
3. **Transfert** : sélectionner l'appareil, déposer le fichier. Le destinataire voit une demande
   avec un **SAS à 6 chiffres** ; les deux personnes le comparent avant d'accepter.
4. **Fin** : `[ terminer ]` côté hôte, `[ quitter ]` côté invité. Sinon le code expire, et la
   session meurt une heure après sa création.

## Ce qui tourne, ce qui tourne pas

| Situation | Chemin |
|---|---|
| Même Wi-Fi | direct (mDNS), aucun relais |
| Partage de connexion iPhone, Wi-Fi invité | onglet `adresse` (`ip:port`) |
| Réseaux différents, 4G | relais, code de session |
| Relais coupé en cours de transfert | échec propre, session reprise au retour |

## Règles

- Le code **tourne** : `[ nouveau code ]` invalide l'ancien immédiatement ; les appareils déjà
  connectés restent connectés.
- Le code n'est **pas** une clé : un relais malveillant peut couper, pas lire. Le SAS comparé à
  l'écran est la seule preuve d'identité.
- L'app doit rester **au premier plan** sur iPhone : en arrière-plan, iOS suspend le WebSocket.
  Le propriétaire peut reprendre dans les 2 minutes ; le transfert en cours, lui, ne survit pas à
  la coupure.
- Un code partagé par messagerie laisse une trace chez le tiers (WhatsApp, iMessage…). Il expire
  en 5 minutes ; c'est le compromis assumé.

## Dépannage

| Symptôme | Cause | Solution |
|---|---|---|
| `relais injoignable` | URL du relais absente ou service arrêté | Réglages → URL du relay ; `curl https://…/healthz` |
| `code inconnu ou expiré` | code mal saisi, expiré, ou renouvelé | redemander le code à l'hôte |
| `trop de tentatives` | 5 essais/minute par réseau | attendre une minute |
| Le pair n'apparaît pas | session non confirmée | vérifier que l'hôte a bien `créer un code` puis laisser 2 s |
| Transfert très lent | relais distant | préférer le direct quand les deux sont sur le même réseau |
