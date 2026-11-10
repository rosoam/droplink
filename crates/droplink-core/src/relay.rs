//! Relay wire protocol shared by the server (`droplink-relay`) and the client (`droplink-net`).
//!
//! Control messages travel as WebSocket **text** frames (JSON). Circuit payloads travel as
//! WebSocket **binary** frames: `[0..16] circuit uuid raw bytes || payload`.
//!
//! The relay never sees plaintext: payloads are already AEAD-sealed end to end.

use crate::protocol::PeerId;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum payload of a relayed binary frame (16-byte header excluded).
pub const MAX_RELAY_FRAME: usize = 1024 * 1024 + 16;
pub const CIRCUIT_ID_LEN: usize = 16;
pub const MAX_PEERS_PER_ROOM: usize = 32;
pub const MAX_CIRCUITS_PER_PEER: usize = 4;
pub const MAX_CONTROL_PER_SECOND: u32 = 10;
pub const MAX_GLOBAL_SOCKETS: usize = 512;
pub const MAX_SOCKETS_PER_IP: usize = 8;
pub const HEARTBEAT_SECS: u64 = 15;
pub const IDLE_TIMEOUT_SECS: u64 = 60;
pub const HANDSHAKE_TIMEOUT_SECS: u64 = 5;

pub type CircuitId = String;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    Register {
        device_id: String,
        alias: String,
        #[serde(default)]
        room: Option<String>,
    },
    Connect {
        peer_id: PeerId,
    },
    Close {
        circuit_id: CircuitId,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg {
    Registered {
        peer_id: PeerId,
    },
    RoomUpdate {
        peers: Vec<PeerSummary>,
    },
    CircuitOpened {
        circuit_id: CircuitId,
        peer: PeerSummary,
        initiator: bool,
    },
    CircuitClosed {
        circuit_id: CircuitId,
        reason: String,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerSummary {
    pub peer_id: PeerId,
    pub alias: String,
}

pub fn encode_binary(circuit: &str, payload: &[u8]) -> Result<Vec<u8>> {
    if payload.len() > MAX_RELAY_FRAME {
        return Err(Error::FrameTooLarge(payload.len()));
    }
    let uuid = Uuid::parse_str(circuit)
        .map_err(|_| Error::Protocol(format!("invalid circuit id: {circuit}")))?;
    let mut out = Vec::with_capacity(CIRCUIT_ID_LEN + payload.len());
    out.extend_from_slice(uuid.as_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

pub fn decode_binary(buf: &[u8]) -> Result<(CircuitId, &[u8])> {
    if buf.len() < CIRCUIT_ID_LEN {
        return Err(Error::Truncated);
    }
    let payload = &buf[CIRCUIT_ID_LEN..];
    if payload.len() > MAX_RELAY_FRAME {
        return Err(Error::FrameTooLarge(payload.len()));
    }
    let uuid = Uuid::from_slice(&buf[..CIRCUIT_ID_LEN])
        .map_err(|_| Error::Protocol("invalid circuit id bytes".into()))?;
    Ok((uuid.to_string(), payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CIRCUIT: &str = "33333333-3333-4333-8333-333333333333";

    #[test]
    fn client_msgs_roundtrip() {
        let msgs = [
            ClientMsg::Register {
                device_id: "11111111-1111-4111-8111-111111111111".into(),
                alias: "Mac".into(),
                room: None,
            },
            ClientMsg::Connect {
                peer_id: "peer".into(),
            },
            ClientMsg::Close {
                circuit_id: CIRCUIT.into(),
            },
        ];
        for msg in msgs {
            let json = serde_json::to_string(&msg).unwrap();
            let back: ClientMsg = serde_json::from_str(&json).unwrap();
            assert_eq!(msg, back);
        }
    }

    #[test]
    fn server_msgs_roundtrip() {
        let msgs = [
            ServerMsg::Registered {
                peer_id: "p".into(),
            },
            ServerMsg::RoomUpdate {
                peers: vec![PeerSummary {
                    peer_id: "p".into(),
                    alias: "A".into(),
                }],
            },
            ServerMsg::CircuitOpened {
                circuit_id: CIRCUIT.into(),
                peer: PeerSummary {
                    peer_id: "p".into(),
                    alias: "A".into(),
                },
                initiator: true,
            },
            ServerMsg::CircuitClosed {
                circuit_id: CIRCUIT.into(),
                reason: "gone".into(),
            },
            ServerMsg::Error {
                code: "limit".into(),
                message: "too many".into(),
            },
        ];
        for msg in msgs {
            let json = serde_json::to_string(&msg).unwrap();
            let back: ServerMsg = serde_json::from_str(&json).unwrap();
            assert_eq!(msg, back);
        }
    }

    #[test]
    fn binary_roundtrip() {
        let payload = vec![9u8; 1024];
        let frame = encode_binary(CIRCUIT, &payload).unwrap();
        let (id, body) = decode_binary(&frame).unwrap();
        assert_eq!(id, CIRCUIT);
        assert_eq!(body, payload.as_slice());
    }

    #[test]
    fn binary_rejects_short_id() {
        assert!(decode_binary(&[0u8; 4]).is_err());
        let mut frame = vec![0u8; CIRCUIT_ID_LEN + 1];
        frame[CIRCUIT_ID_LEN] = 0xAA;
        let (_, body) = decode_binary(&frame).unwrap();
        assert_eq!(body, [0xAA]);
    }

    #[test]
    fn binary_rejects_oversize() {
        let payload = vec![0u8; MAX_RELAY_FRAME + 1];
        assert!(matches!(
            encode_binary(CIRCUIT, &payload),
            Err(Error::FrameTooLarge(_))
        ));
    }

    #[test]
    fn binary_rejects_non_uuid_circuit() {
        assert!(encode_binary("nope", b"x").is_err());
    }
}
