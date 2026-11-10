# Modèle de menaces — DropLink

Méthode STRIDE, limitée aux flux réellement exposés : handshake, données, relay, fichiers reçus, configuration
locale. Chaque ligne indique la mitigation implémentée et le test qui la couvre.

## Actifs

1. Le contenu des fichiers transférés.
2. Le nom et la taille des fichiers.
3. L'identité des appareils (`device_id`, alias).
4. Les clés éphémères et les pairs du canal chiffré.
5. Le dossier de réception (intégrité du système de fichiers).

## Surface d'attaque

| Surface | Qui peut l'atteindre |
|---|---|
| mDNS (multicast LAN) | tout appareil du même L2/L3 |
| TCP 7777–7800 (ou port éphémère) | tout appareil du LAN |
| Relay WebSocket | tout client Internet connaissant l'URL |
| Fichier de config / historique | processus local de l'utilisateur |

## STRIDE par flux

### Handshake

| Menace | Scénario | Mitigation | Test |
|---|---|---|---|
| Spoofing | Un pair se fait passer pour un autre | `device_id` + transcript authentifié + SAS comparé | `crypto::transcript_and_sas_match` |
| Tampering | Réécriture du nom ou de la taille | `hello_canon` couvre version, id, taille, chunk, nom, alias, clé A | `crypto::transcript_changes_with_metadata` |
| Repudiation | Nier avoir envoyé | hors périmètre (pas de journal signé) | — |
| Information disclosure | Lecture des clés | X25519 éphémère, clés jamais sur le fil | `crypto::direction_keys_differ` |
| Denial of service | Connexions idle | `handshake_timeout` 5 s, cap 2/IP, sémaphore | `limits::max_parallel_rejects_fifth` |
| Elevation | Injection de petit ordre | `was_contributory()` obligatoire | `crypto::non_contributory_public_key_rejected` |

### Données

| Menace | Scénario | Mitigation | Test |
|---|---|---|---|
| Spoofing | Forger un `Done` | Toutes les frames post-handshake sont scellées | `direct_transfer::no_completed_without_done_ok` |
| Tampering | Modifier un chunk | AEAD ChaCha20-Poly1305, tag 16 octets | `direct_transfer::tampered_ciphertext_is_rejected` |
| Replay | Rejouer une frame | Compteur de nonce strictement croissant | `crypto::replayed_frame_fails` |
| Info disclosure | Lire le fichier | Ciphertext uniquement | `relay_transfer::relay_no_plaintext_leak` |
| DoS | Annoncer 1 Mio et envoyer 10 Gio | `bytes + len > file_size` ⇒ coupure | `limits::lying_file_size` |
| DoS | Écrire hors du dossier | `sanitize_file_name` + `is_within` | `path::sanitized_name_is_safe` (10 000 cas) |

### Relay

| Menace | Scénario | Mitigation | Test |
|---|---|---|---|
| Spoofing | `device_id` d'autrui | `duplicate_id` refusé ; `device_id` jamais rebroadcasté | `ws_relay::relay_rejects_duplicate_device_id` |
| Tampering | Modifier le payload | Le relay ne peut pas déchiffrer | `relay_no_plaintext_leak` |
| Info disclosure | Énumérer les pairs | Room par IP hashée ou code explicite ; alias seul | `room::peers_in_different_rooms_cannot_connect` |
| DoS | Flood de contrôle | Token bucket 10/s | `server::token_bucket_limits_bursts` |
| DoS | Slow reader | File pleine ⇒ fermeture du circuit seul | `room::slow_reader_closes_circuit_only` |
| DoS | Socket flood | 512 global, 8/IP | `room::socket_caps` |

### Fichiers reçus

| Menace | Scénario | Mitigation | Test |
|---|---|---|---|
| Tampering | Fichier corrompu publié | Hash vérifié avant `hard_link` | `direct_transfer::hash_mismatch_is_rejected` |
| Tampering | Écrasement d'un fichier existant | `create_new` + `hard_link` (atomique) | `fs_util::publish_never_overwrites` |
| DoS | Disque rempli | `MAX_FILE_SIZE`, `.part` supprimé à l'échec | `direct_transfer::truncated_stream_is_rejected` |
| Info disclosure | Fichier partiel visible | Nom final attribué seulement après vérification | `fs_util::publish_leaves_no_part_file` |

### Configuration locale

| Menace | Scénario | Mitigation | Test |
|---|---|---|---|
| Info disclosure | Lecture par un autre utilisateur | Permissions `0600` sur `config.json` | revue S02/S06 |
| Tampering | Modification manuelle invalide | `Config::validate` au chargement, régénération du `device_id` | `config::json_roundtrip` |

## Risques acceptés

- **SAS non comparé** : sans comparaison humaine, un MITM actif reste possible. C'est le compromis d'un protocole
  sans PKI.
- **mDNS en clair** : nom et IP visibles sur le LAN. Le contenu reste chiffré.
- **Room par IP publique** : derrière un CGNAT, plusieurs foyers partagent une room. Un code de room isole.
- **Métadonnées dans les logs** : identifiants de transfert et tailles, jamais de contenu ni de clé.

## Hors périmètre

- Compromission de la machine locale (keylogger, root).
- Analyse antivirus des fichiers reçus.
- Reprise après interruption et chiffrement au repos.
- Distribution signée / notarisation.

## Sessions distantes (relais)

| Menace | Contre-mesure | Test |
|---|---|---|
| Énumération de codes | 40 bits + 5/min par /64 + budget global + échec uniforme | `ws_relay.rs`, `limit.rs` |
| Force brute distribuée IPv6 | agrégation /64 | `ipv6_is_grouped_by_64` |
| Usurpation d'IP via XFF | en-têtes ignorés hors proxy de confiance, lecture de droite à gauche | `forwarded_header_*` |
| Kill d'un transfert via `device_id` public | session possédée par token, `already_active` refuse la nouvelle | `duplicate_device_id_never_kills_*` |
| DoS mémoire par récepteur lent | contre-pression `send().await`, file de 4 trames, `MemoryMax=1G` | `slow_reader_route_exposes_backpressure` |
| MITM du relais | AEAD + SAS comparé des deux côtés | `crypto.rs` |
| Code partagé par messagerie | fuite tierce assumée, TTL 5 min | documentation |
