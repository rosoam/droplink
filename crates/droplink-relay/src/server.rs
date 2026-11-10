use crate::room::{Outbound, RelayError, Rooms};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use droplink_core::relay::{
    generate_code, generate_token, normalize_code, now_secs, proto_matches, ClientMsg, ServerMsg,
    HEARTBEAT_SECS, IDLE_TIMEOUT_SECS, MAX_CONTROL_PER_SECOND, RELAY_QUEUE_FRAMES,
    SESSION_TTL_SECS,
};
use futures_util::{SinkExt, StreamExt};
use rand_core::OsRng;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub type AppState = Arc<Mutex<Rooms>>;

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/ws", get(ws_handler))
        .with_state(state)
}

async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    let rooms = lock(&state);
    Json(serde_json::json!({
        "status": "ok",
        "sessions": rooms.session_count(),
        "peers": rooms.peer_count(),
        "circuits": rooms.total_circuits(),
    }))
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let ip = client_ip(&headers, addr, trusted_cidrs());
    let allowed = {
        let mut rooms = lock(&state);
        rooms.try_open_socket(ip)
    };
    if !allowed {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    ws.on_upgrade(move |socket| handle_socket(socket, ip, state))
        .into_response()
}

fn lock(state: &AppState) -> std::sync::MutexGuard<'_, Rooms> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Trusted reverse proxies, from `TRUSTED_PROXY_CIDRS` (comma-separated CIDRs).
///
/// Empty by default: headers are then ignored and the socket address is authoritative.
fn trusted_cidrs() -> &'static [(IpAddr, u8)] {
    static TRUSTED: OnceLock<Vec<(IpAddr, u8)>> = OnceLock::new();
    TRUSTED.get_or_init(|| {
        std::env::var("TRUSTED_PROXY_CIDRS")
            .ok()
            .map(|value| value.split(',').filter_map(parse_cidr).collect::<Vec<_>>())
            .unwrap_or_default()
    })
}

fn parse_cidr(value: &str) -> Option<(IpAddr, u8)> {
    let value = value.trim();
    let (addr, prefix) = match value.split_once('/') {
        Some((addr, prefix)) => (addr, Some(prefix.parse::<u8>().ok()?)),
        None => (value, None),
    };
    let ip: IpAddr = addr.parse().ok()?;
    let prefix = prefix.unwrap_or(match ip {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    });
    Some((ip, prefix))
}

fn in_cidr(ip: IpAddr, net: IpAddr, prefix: u8) -> bool {
    match (ip, net) {
        (IpAddr::V4(ip), IpAddr::V4(net)) => {
            let prefix = prefix.min(32) as u32;
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            (u32::from(ip) & mask) == (u32::from(net) & mask)
        }
        (IpAddr::V6(ip), IpAddr::V6(net)) => {
            let prefix = prefix.min(128) as u32;
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            (u128::from(ip) & mask) == (u128::from(net) & mask)
        }
        _ => false,
    }
}

/// Resolve the client address.
///
/// Headers are only read when the socket peer is a configured trusted proxy, and the
/// `X-Forwarded-For` list is walked **right to left**: the leftmost element is fully controlled by
/// the client. The proxy must also overwrite the header (`header_up X-Forwarded-For {remote_host}`).
fn client_ip(headers: &HeaderMap, addr: SocketAddr, trusted: &[(IpAddr, u8)]) -> IpAddr {
    let peer = addr.ip();
    if !trusted
        .iter()
        .any(|(net, prefix)| in_cidr(peer, *net, *prefix))
    {
        return peer;
    }
    let Some(value) = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
    else {
        return peer;
    };
    for part in value.split(',').rev() {
        if let Ok(candidate) = part.trim().parse::<IpAddr>() {
            if !trusted
                .iter()
                .any(|(net, prefix)| in_cidr(candidate, *net, *prefix))
            {
                return candidate;
            }
        }
    }
    peer
}

struct TokenBucket {
    tokens: f64,
    capacity: f64,
    rate_per_sec: f64,
    last: Instant,
}

impl TokenBucket {
    fn new(rate_per_sec: u32) -> Self {
        let capacity = rate_per_sec as f64;
        Self {
            tokens: capacity,
            capacity,
            rate_per_sec: capacity,
            last: Instant::now(),
        }
    }

    fn allow(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.rate_per_sec).min(self.capacity);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

async fn handle_socket(socket: WebSocket, ip: IpAddr, state: AppState) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<Outbound>(RELAY_QUEUE_FRAMES);

    let writer = tokio::spawn(async move {
        while let Some(outbound) = rx.recv().await {
            let message = match outbound {
                Outbound::Text(server) => match serde_json::to_string(&server) {
                    Ok(json) => Message::Text(json.into()),
                    Err(_) => continue,
                },
                Outbound::Binary(bytes) => Message::Binary(bytes.into()),
                Outbound::Ping => Message::Ping(Default::default()),
            };
            if sink.send(message).await.is_err() {
                break;
            }
        }
    });

    let mut peer_id: Option<String> = None;
    let mut bucket = TokenBucket::new(MAX_CONTROL_PER_SECOND);
    let mut last_seen = Instant::now();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECS));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if last_seen.elapsed() > Duration::from_secs(IDLE_TIMEOUT_SECS) {
                    break;
                }
                if tx.send(Outbound::Ping).await.is_err() {
                    break;
                }
            }
            message = stream.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        last_seen = Instant::now();
                        if !bucket.allow() {
                            let _ = tx.try_send(Outbound::Text(ServerMsg::Error {
                                code: "rate_limited".into(),
                                message: "too many control messages".into(),
                            }));
                            continue;
                        }
                        handle_text(&state, &mut peer_id, ip, &tx, text.as_str());
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        last_seen = Instant::now();
                        if let Some(id) = &peer_id {
                            // Prepare under the lock, then await the send **outside** it:
                            // a slow reader must slow the sender, not lose the frame.
                            let prepared = {
                                let mut rooms = lock(&state);
                                rooms.prepare_route(id, &bytes)
                            };
                            match prepared {
                                Ok(route) => {
                                    if route
                                        .tx
                                        .send(Outbound::Binary(bytes.to_vec()))
                                        .await
                                        .is_err()
                                    {
                                        lock(&state).close_circuit(
                                            &route.circuit_id,
                                            "backpressure",
                                            None,
                                        );
                                    }
                                }
                                Err(err) => {
                                    let _ = tx.try_send(Outbound::Text(ServerMsg::Error {
                                        code: relay_error_code(&err).into(),
                                        message: err.to_string(),
                                    }));
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {
                        last_seen = Instant::now();
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                }
            }
        }
    }

    if let Some(id) = peer_id {
        lock(&state).unregister(&id);
    }
    lock(&state).close_socket(ip);
    writer.abort();
}

fn handle_text(
    state: &AppState,
    peer_id: &mut Option<String>,
    ip: IpAddr,
    tx: &mpsc::Sender<Outbound>,
    text: &str,
) {
    let Ok(message) = serde_json::from_str::<ClientMsg>(text) else {
        let _ = tx.try_send(Outbound::Text(ServerMsg::Error {
            code: "bad_message".into(),
            message: "invalid control message".into(),
        }));
        return;
    };

    match message {
        ClientMsg::CreateSession {
            proto,
            device_id,
            alias,
        } => {
            if !proto_matches(proto) {
                send_error(
                    tx,
                    "unsupported_version",
                    "client protocol is not supported",
                );
                return;
            }
            if !valid_alias(&alias) {
                send_error(tx, "bad_alias", "alias must be 1..64 chars");
                return;
            }
            if peer_id.is_some() {
                send_error(tx, "already_registered", "one session per connection");
                return;
            }
            if !lock(state).allow_session_attempt(ip) {
                send_error(tx, "rate_limited", "too many session attempts");
                return;
            }
            let code = generate_code(&mut OsRng);
            let token = generate_token(&mut OsRng);
            let now = now_secs();
            let mut rooms = lock(state);
            match rooms.create_session(
                device_id,
                alias,
                tx.clone(),
                now,
                code.clone(),
                token.clone(),
            ) {
                Ok((peer, _, expires_at)) => {
                    *peer_id = Some(peer);
                    let _ = tx.try_send(Outbound::Text(ServerMsg::SessionCreated {
                        code,
                        token,
                        expires_at,
                        ttl_secs: SESSION_TTL_SECS,
                    }));
                }
                Err(err) => send_error(tx, relay_error_code(&err), &err.to_string()),
            }
        }
        ClientMsg::JoinSession {
            proto,
            device_id,
            alias,
            code,
        } => {
            if !proto_matches(proto) {
                send_error(
                    tx,
                    "unsupported_version",
                    "client protocol is not supported",
                );
                return;
            }
            if !valid_alias(&alias) {
                send_error(tx, "bad_alias", "alias must be 1..64 chars");
                return;
            }
            if peer_id.is_some() {
                send_error(tx, "already_registered", "one session per connection");
                return;
            }
            if !lock(state).allow_session_attempt(ip) {
                send_error(tx, "rate_limited", "too many session attempts");
                return;
            }
            let Some(code) = normalize_code(&code) else {
                send_error(tx, "no_session", "unknown or expired session");
                return;
            };
            let now = now_secs();
            let guest_token = generate_token(&mut OsRng);
            let mut rooms = lock(state);
            match rooms.join_session(
                &code,
                device_id,
                alias,
                tx.clone(),
                now,
                guest_token.clone(),
            ) {
                Ok((peer, _, expires_at)) => {
                    let peers = rooms.peers_of(&peer);
                    *peer_id = Some(peer);
                    let _ = tx.try_send(Outbound::Text(ServerMsg::SessionJoined {
                        expires_at,
                        ttl_secs: SESSION_TTL_SECS,
                        peers,
                        token: guest_token,
                    }));
                }
                Err(err) => send_error(tx, relay_error_code(&err), &err.to_string()),
            }
        }
        ClientMsg::ResumeSession { token } => {
            if peer_id.is_some() {
                send_error(tx, "already_registered", "one session per connection");
                return;
            }
            let code = generate_code(&mut OsRng);
            let now = now_secs();
            let mut rooms = lock(state);
            match rooms.resume_session(&token, tx.clone(), now, code) {
                Ok((peer, _, code, expires_at)) => {
                    let peers = rooms.peers_of(&peer);
                    *peer_id = Some(peer);
                    let _ = tx.try_send(Outbound::Text(ServerMsg::SessionResumed {
                        code,
                        expires_at,
                        ttl_secs: SESSION_TTL_SECS,
                        peers,
                    }));
                }
                Err(RelayError::UnknownSession) => {
                    let guest_token = generate_token(&mut OsRng);
                    match rooms.resume_guest(&token, tx.clone(), now, guest_token.clone()) {
                        Ok((peer, _, expires_at)) => {
                            let peers = rooms.peers_of(&peer);
                            *peer_id = Some(peer);
                            let _ = tx.try_send(Outbound::Text(ServerMsg::SessionJoined {
                                expires_at,
                                ttl_secs: SESSION_TTL_SECS,
                                peers,
                                token: guest_token,
                            }));
                        }
                        Err(err) => send_error(tx, relay_error_code(&err), &err.to_string()),
                    }
                }
                Err(err) => send_error(tx, relay_error_code(&err), &err.to_string()),
            }
        }
        ClientMsg::RefreshSession { token } => {
            let code = generate_code(&mut OsRng);
            let mut rooms = lock(state);
            match rooms.refresh_session(&token, now_secs(), code) {
                Ok((code, expires_at)) => {
                    let _ = tx.try_send(Outbound::Text(ServerMsg::SessionRefreshed {
                        code,
                        expires_at,
                        ttl_secs: SESSION_TTL_SECS,
                    }));
                }
                Err(err) => send_error(tx, relay_error_code(&err), &err.to_string()),
            }
        }
        ClientMsg::LeaveSession => {
            let Some(me) = peer_id.clone() else {
                send_error(tx, "not_registered", "join a session first");
                return;
            };
            lock(state).leave_session(&me);
            *peer_id = None;
        }
        ClientMsg::Connect { peer_id: target } => {
            let Some(me) = peer_id.clone() else {
                send_error(tx, "not_registered", "join a session first");
                return;
            };
            let mut rooms = lock(state);
            if let Err(err) = rooms.connect(&me, &target) {
                send_error(tx, relay_error_code(&err), &err.to_string());
            }
        }
        ClientMsg::Close { circuit_id } => {
            if peer_id.is_some() {
                lock(state).close_circuit(&circuit_id, "closed by peer", None);
            }
        }
    }
}

fn valid_alias(alias: &str) -> bool {
    !alias.trim().is_empty() && alias.chars().count() <= 64
}

fn send_error(tx: &mpsc::Sender<Outbound>, code: &str, message: &str) {
    let _ = tx.try_send(Outbound::Text(ServerMsg::Error {
        code: code.to_string(),
        message: message.to_string(),
    }));
}

fn relay_error_code(err: &RelayError) -> &'static str {
    match err {
        RelayError::Limit(_) => "limit",
        RelayError::AlreadyActive => "already_active",
        RelayError::DuplicateId => "duplicate_id",
        RelayError::UnknownPeer => "unknown_peer",
        RelayError::UnknownCircuit => "unknown_circuit",
        RelayError::UnknownSession => "no_session",
        RelayError::Expired => "expired",
        RelayError::WrongRoom => "wrong_room",
    }
}

/// Serve the relay until `shutdown` resolves.
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: AppState,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let sweep_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(5));
        loop {
            ticker.tick().await;
            let _ = lock(&sweep_state).sweep(now_secs());
        }
    });

    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn proxy_list() -> Vec<(IpAddr, u8)> {
        vec![(IpAddr::V4(Ipv4Addr::LOCALHOST), 32)]
    }

    #[test]
    fn token_bucket_limits_bursts() {
        let mut bucket = TokenBucket::new(10);
        for _ in 0..10 {
            assert!(bucket.allow());
        }
        assert!(!bucket.allow());
    }

    #[test]
    fn forwarded_header_is_ignored_without_a_trusted_proxy() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.9".parse().unwrap());
        let addr: SocketAddr = "198.51.100.7:1234".parse().unwrap();
        assert_eq!(
            client_ip(&headers, addr, &proxy_list()),
            "198.51.100.7".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn forwarded_header_is_read_right_to_left_behind_a_trusted_proxy() {
        let mut headers = HeaderMap::new();
        // The leftmost element is client-controlled; the rightmost is what the proxy appended.
        headers.insert("x-forwarded-for", "1.2.3.4, 203.0.113.9".parse().unwrap());
        let addr: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        assert_eq!(
            client_ip(&headers, addr, &proxy_list()),
            "203.0.113.9".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn cidr_matching() {
        let net: IpAddr = "10.0.0.0".parse().unwrap();
        assert!(in_cidr("10.0.0.7".parse().unwrap(), net, 8));
        assert!(!in_cidr("11.0.0.7".parse().unwrap(), net, 8));
        let v6: IpAddr = "2001:db8::".parse().unwrap();
        assert!(in_cidr("2001:db8::1".parse().unwrap(), v6, 32));
        assert!(!in_cidr("2001:db9::1".parse().unwrap(), v6, 32));
        assert!(!in_cidr(
            "10.0.0.7".parse().unwrap(),
            "2001:db8::".parse().unwrap(),
            32
        ));
    }

    #[test]
    fn parse_cidr_defaults() {
        assert_eq!(
            parse_cidr("127.0.0.1"),
            Some((IpAddr::V4(Ipv4Addr::LOCALHOST), 32))
        );
        assert_eq!(
            parse_cidr("127.0.0.0/8"),
            Some((IpAddr::V4(Ipv4Addr::new(127, 0, 0, 0)), 8))
        );
        assert_eq!(parse_cidr("nope"), None);
    }
}
