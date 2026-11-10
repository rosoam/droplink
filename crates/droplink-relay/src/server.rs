use crate::room::{Outbound, RelayError, Rooms};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use droplink_core::relay::{
    ClientMsg, ServerMsg, HEARTBEAT_SECS, IDLE_TIMEOUT_SECS, MAX_CONTROL_PER_SECOND,
};
use futures_util::{SinkExt, StreamExt};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub type AppState = Arc<Mutex<Rooms>>;

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/ws", get(ws_handler))
        .with_state(state)
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let ip = client_ip(&headers, addr);
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

fn client_ip(headers: &HeaderMap, addr: SocketAddr) -> IpAddr {
    headers
        .get("fly-client-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or_else(|| addr.ip())
}

fn room_id(ip: IpAddr, code: Option<&str>) -> String {
    let mut hasher = blake3::Hasher::new();
    let prefix = if code.is_some() { "code" } else { "ip" };
    match code {
        Some(code) => {
            hasher.update(b"code:");
            hasher.update(code.as_bytes());
        }
        None => {
            hasher.update(b"ip:");
            hasher.update(ip.to_string().as_bytes());
        }
    }
    let hex = hasher.finalize().to_hex().to_string();
    format!("{prefix}:{}", &hex[..16])
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
    let (tx, mut rx) = mpsc::channel::<Outbound>(64);

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
                            let mut rooms = lock(&state);
                            let _ = rooms.route(id, &bytes);
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
        ClientMsg::Register {
            device_id,
            alias,
            room,
        } => {
            if peer_id.is_some() {
                let _ = tx.try_send(Outbound::Text(ServerMsg::Error {
                    code: "already_registered".into(),
                    message: "register once per connection".into(),
                }));
                return;
            }
            if alias.trim().is_empty() || alias.chars().count() > 64 {
                let _ = tx.try_send(Outbound::Text(ServerMsg::Error {
                    code: "bad_alias".into(),
                    message: "alias must be 1..64 chars".into(),
                }));
                return;
            }
            let room = room_id(ip, room.as_deref());
            let mut rooms = lock(state);
            match rooms.register(room, device_id, alias, tx.clone()) {
                Ok(id) => {
                    *peer_id = Some(id.clone());
                    let _ = tx.try_send(Outbound::Text(ServerMsg::Registered { peer_id: id }));
                }
                Err(err) => {
                    let code = match err {
                        RelayError::DuplicateId => "duplicate_id",
                        RelayError::Limit(_) => "limit",
                        _ => "refused",
                    };
                    let _ = tx.try_send(Outbound::Text(ServerMsg::Error {
                        code: code.into(),
                        message: err.to_string(),
                    }));
                }
            }
        }
        ClientMsg::Connect { peer_id: target } => {
            let Some(me) = peer_id.clone() else {
                let _ = tx.try_send(Outbound::Text(ServerMsg::Error {
                    code: "not_registered".into(),
                    message: "register first".into(),
                }));
                return;
            };
            let mut rooms = lock(state);
            if let Err(err) = rooms.connect(&me, &target) {
                let code = match err {
                    RelayError::Limit(_) => "limit",
                    RelayError::UnknownPeer => "unknown_peer",
                    RelayError::WrongRoom => "wrong_room",
                    _ => "refused",
                };
                let _ = tx.try_send(Outbound::Text(ServerMsg::Error {
                    code: code.into(),
                    message: err.to_string(),
                }));
            }
        }
        ClientMsg::Close { circuit_id } => {
            if peer_id.is_some() {
                lock(state).close_circuit(&circuit_id, "closed by peer", None);
            }
        }
    }
}

/// Serve the relay until `shutdown` resolves.
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: AppState,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
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

    #[test]
    fn room_id_is_stable_and_hides_ip() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));
        let a = room_id(ip, None);
        let b = room_id(ip, None);
        assert_eq!(a, b);
        assert!(!a.contains("192"));
        assert_ne!(a, room_id(ip, Some("abc")));
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
    fn client_ip_prefers_fly_header() {
        let mut headers = HeaderMap::new();
        headers.insert("fly-client-ip", "203.0.113.9".parse().unwrap());
        let addr: SocketAddr = "10.0.0.1:1234".parse().unwrap();
        assert_eq!(
            client_ip(&headers, addr),
            "203.0.113.9".parse::<IpAddr>().unwrap()
        );
    }
}
