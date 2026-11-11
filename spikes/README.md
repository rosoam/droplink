# Spikes — phase 0 WebRTC

Deux cartons, pas de serveur, pas de dépendance payante.

## `webrtc-data` — data channel entre deux pairs locaux

```bash
cargo run --release --manifest-path spikes/webrtc-data/Cargo.toml
```

Résultat sur macOS (2026-11-11) :

```
OK 10.0 MiB in 1.13s (8.9 MiB/s), blake3 bbaf4bbe...
```

Deux `RTCPeerConnection` dans le même process, SDP échangé en mémoire, candidats host uniquement.
Cela prouve la bibliothèque et donne une base de débit. Ce n'est pas encore le réseau.

## Portabilité iOS

- `webrtc = 0.21` **ne compile pas** pour `aarch64-apple-ios` : `rtc-shared` tire `nix`, qui utilise
  `libc::SYSPROTO_CONTROL` (constante macOS absente d'iOS).
- `str0m = 0.25` **compile** pour `aarch64-apple-ios` :

```bash
cargo check --release --target aarch64-apple-ios --manifest-path spikes/str0m-check/Cargo.toml
# Finished
```

Conclusion : desktop = `webrtc-rs` possible ; iOS = `str0m` (sans-IO) ou libwebrtc. Le spike
`str0m` data channel est la prochaine étape.
