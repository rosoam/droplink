# Déployer le relais DropLink

Le relais ne voit que des trames opaques : il relaie un circuit déjà scellé de bout en bout. Il
voit les adresses IP, les alias, les tailles et les durées — rien du contenu ni des noms de
fichiers.

## Prérequis

- Un hôte Linux avec un nom DNS `relay.droplink.app` (ou le tien, voir plus bas).
- Caddy pour le TLS et le reverse proxy WebSocket.
- Le binaire `droplink-relay` construit en release.

```bash
cargo build --release -p droplink-relay
install -m 0755 target/release/droplink-relay /usr/local/bin/droplink-relay
```

## Installation

```bash
install -m 0644 deploy/relay.service /etc/systemd/system/droplink-relay.service
install -m 0644 deploy/Caddyfile /etc/caddy/Caddyfile
systemctl daemon-reload
systemctl enable --now droplink-relay
systemctl reload caddy
```

## Vérifier

```bash
curl -fsS https://relay.droplink.app/healthz
# {"status":"ok","sessions":0,"peers":0,"circuits":0}
journalctl -u droplink-relay -f
```

Puis, depuis deux réseaux distincts, créer un code sur l'un et le saisir sur l'autre ; le
transfert doit passer en `relay` et le hash du fichier reçu doit être identique.

## Pourquoi `TRUSTED_PROXY_CIDRS` est limité à la loopback

`X-Forwarded-For` n'est lu que si la connexion vient d'un proxy de confiance. Caddy doit
**réécrire** l'en-tête (`header_up X-Forwarded-For {remote_host}`) : sinon un client peut envoyer
`X-Forwarded-For: 1.2.3.4` et contourner les plafonds par IP. Le relais lit la liste de droite à
gauche et ignore les sauts de confiance.

## Mise à jour

```bash
cargo build --release -p droplink-relay
install -m 0755 target/release/droplink-relay /usr/local/bin/droplink-relay
systemctl restart droplink-relay
```

## Rollback

```bash
systemctl disable --now droplink-relay
```

Si des clients pointent déjà sur le domaine, remets `DEFAULT_RELAY_URL` à `None` dans
`droplink-core/src/config.rs` et livre une version : l'app dira « relais injoignable » au lieu
d'attendre un DNS mort.

## Dimensionnement

- `MemoryMax=1G` ; files bornées à 4 trames de 256 Kio par socket, 512 sockets au plus.
- Un récepteur lent ralentit l'émetteur (contre-pression TCP), il ne fait pas tomber le relais.
- Les sessions expirent après 5 min de code, 1 h de vie absolue.
