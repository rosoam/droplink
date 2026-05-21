use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::sync::Mutex;

#[derive(Clone, Serialize, Deserialize, Debug)]
struct PeerInfo {
    name: String,
    ip: String,
    port: u16,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Register { name: String, ip: String, port: u16 },
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    RoomUpdate { devices: Vec<PeerInfo> },
}

struct PeerSlot {
    info: PeerInfo,
    tx: tokio::sync::mpsc::Sender<String>,
}

type RoomMap = Arc<Mutex<HashMap<String, Vec<PeerSlot>>>>;

#[tokio::main]
async fn main() {
    env_logger::init();
    let rooms: RoomMap = Arc::new(Mutex::new(HashMap::new()));
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(rooms);
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8080);
    let addr = format!("0.0.0.0:{}", port);
    log::info!("Relay server on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    State(rooms): State<RoomMap>,
) -> impl IntoResponse {
    let public_ip = headers
        .get("fly-client-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .unwrap_or_else(|| addr.ip().to_string());
    ws.on_upgrade(move |socket| handle_socket(socket, public_ip, rooms))
}

async fn broadcast_room(room: &[PeerSlot]) {
    let all: Vec<PeerInfo> = room.iter().map(|s| s.info.clone()).collect();
    for slot in room.iter() {
        let others: Vec<PeerInfo> = all
            .iter()
            .filter(|p| p.name != slot.info.name)
            .cloned()
            .collect();
        let msg = serde_json::to_string(&ServerMsg::RoomUpdate { devices: others }).unwrap();
        let _ = slot.tx.send(msg).await;
    }
}

async fn handle_socket(socket: WebSocket, public_ip: String, rooms: RoomMap) {
    let (mut ws_write, mut ws_read) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);
    let mut my_name: Option<String> = None;

    // Writer task
    let write_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_write.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    // Reader loop
    while let Some(Ok(msg)) = ws_read.next().await {
        if let Message::Text(text) = msg {
            if let Ok(ClientMsg::Register { name, ip, port }) = serde_json::from_str(&text) {
                my_name = Some(name.clone());
                let mut map = rooms.lock().await;
                let room = map.entry(public_ip.clone()).or_default();
                room.retain(|s| s.info.name != name);
                room.push(PeerSlot {
                    info: PeerInfo { name: name.clone(), ip, port },
                    tx: tx.clone(),
                });
                broadcast_room(room).await;
            }
        }
    }

    // Cleanup on disconnect
    if let Some(name) = my_name {
        let mut map = rooms.lock().await;
        if let Some(room) = map.get_mut(&public_ip) {
            room.retain(|s| s.info.name != name);
            broadcast_room(room).await;
            if room.is_empty() {
                map.remove(&public_ip);
            }
        }
    }

    write_task.abort();
}
