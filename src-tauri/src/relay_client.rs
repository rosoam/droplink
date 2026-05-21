#![allow(dead_code)] // consumed in Task 4 (relay integration into main.rs)

use crate::protocol::DeviceInfo;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

pub const RELAY_URL: &str = "wss://droplink-relay.fly.dev/ws";

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg<'a> {
    Register {
        name: &'a str,
        ip: &'a str,
        port: u16,
    },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    RoomUpdate { devices: Vec<DeviceInfo> },
}

pub enum RelayEvent {
    DevicesUpdated(Vec<DeviceInfo>),
    Disconnected,
}

pub async fn connect_and_listen(
    device_name: String,
    local_ip: String,
    port: u16,
    tx: mpsc::Sender<RelayEvent>,
) {
    let url = Url::parse(RELAY_URL).expect("invalid relay URL");
    let Ok((ws_stream, _)) = connect_async(url).await else {
        log::warn!("Relay server unreachable — mode local uniquement");
        return;
    };

    let (mut write, mut read) = ws_stream.split();

    let register = serde_json::to_string(&ClientMsg::Register {
        name: &device_name,
        ip: &local_ip,
        port,
    })
    .unwrap();

    if write.send(Message::Text(register)).await.is_err() {
        return;
    }

    while let Some(Ok(msg)) = read.next().await {
        if let Message::Text(text) = msg {
            if let Ok(ServerMsg::RoomUpdate { devices }) = serde_json::from_str(&text) {
                if tx.send(RelayEvent::DevicesUpdated(devices)).await.is_err() {
                    break;
                }
            }
        }
    }

    let _ = tx.send(RelayEvent::Disconnected).await;
}
