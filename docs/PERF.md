# Performances — DropLink 1.0

Mesures effectuées sur la machine cible (Apple Silicon, macOS, build `release` sauf mention contraire).

## Hachage (32 Mio, meilleur de 3 passes)

| Algorithme | Débit |
|---|---|
| BLAKE3 | 2 222 MiB/s |
| SHA-256 | 541 MiB/s |
| Ratio | **4,11×** |

Commande : `cargo bench -p droplink-core --bench hash -- --sample-size 10` (l'assertion `ratio >= 3.0`
échoue la CI si la supériorité disparaît).

## Transfert loopback (256 Mio)

| Métrique | Valeur |
|---|---|
| Débit mesuré | **425 MB/s** (436 MB/s hors charge de fuzzing) |
| Budget | ≥ 250 MB/s |

Commande : `cargo bench -p droplink-net --bench loopback -- --sample-size 10`.
Le transfert inclut l'échange de clés, le chiffrement par chunk, la vérification BLAKE3 et l'écriture disque.

## Démarrage

| Métrique | Valeur | Budget |
|---|---|---|
| `startup_ms` (config + moteur + mDNS, sans fenêtre) | 1 ms | < 250 ms |

Commande : `cargo run -p droplink-app -- --smoke`.
Le test `ui_smoke` échoue si `startup_ms` dépasse 250 ms.

## Binaire release

| Métrique | Valeur | Budget |
|---|---|---|
| Taille de `target/release/droplink` (strippé, LTO fat) | **6,36 Mo** (6 360 512 octets) | < 20 Mo |

Mesure : `cargo xtask perf` (affiche la taille réelle et échoue au-delà de 22 Mo).

## Budgets vérifiés automatiquement

| Budget | Test |
|---|---|
| ≤ 20 événements de progression / s | `direct_transfer::progress_rate_capped` |
| Un consommateur figé ne bloque pas le transfert | `direct_transfer::stalled_consumer_does_not_stall_transfer` |
| Une seule ouverture du fichier source | `direct_transfer::single_read_proof` |
| Contre-pression mémoire bornée | `slow_reader` (S10) |
| Anti-fuite en clair via relay | `relay_transfer::relay_no_plaintext_leak` |

## Couverture de tests

| Périmètre | Lignes couvertes |
|---|---|
| Tout le workspace (y compris points d'entrée et xtask) | 84,70 % |
| Bibliothèques + UI, hors `main.rs` et `xtask` (gate CI) | **87,45 %** |

Commande du gate : `cargo llvm-cov --workspace --fail-under-lines 85 --ignore-filename-regex 'main\.rs$|xtask/'`.
Les points d'entrée binaires (bind du port, `ctrl-c`, boucle eframe) ne portent pas de logique testable ; ils sont
exercés par `--smoke` et par le lancement réel.

## Fuzzing

| Cible | Exécutions | Durée | Crashs |
|---|---|---|---|
| `fuzz_frame` (décodage de frames arbitraires) | 84 737 513 | 600 s | 0 |
| `fuzz_control` (JSON de contrôle + validation) | 64 602 492 | 600 s | 0 |
| `fuzz_txt` (TXT mDNS) | 32 175 640 | 600 s | 0 |
