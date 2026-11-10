//! Relay wire protocol shared by the server (`droplink-relay`) and the client (`droplink-net`).
//!
//! Control messages travel as WebSocket **text** frames (JSON). Circuit payloads travel as
//! WebSocket **binary** frames: `[0..16] circuit uuid raw bytes || payload`.
//!
//! The relay never sees plaintext: payloads are already AEAD-sealed end to end.
//!
//! ## Sessions
//!
//! A session is a rendezvous: the server generates a short code (8 characters, ~40 bits) and a
//! 256-bit bearer token. The code is shared out of band; the token never leaves the device that
//! created the session. The server indexes codes and tokens by BLAKE3 hash. The hash is an index,
//! **not** a secret: 40 bits fall to an offline search. The real protection is the attempt rate
//! limit, the 5-minute TTL and the absolute session lifetime.

use crate::protocol::{PeerId, PROTOCOL_VERSION};
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

/// Outbound queue depth per socket. Bounded memory: `RELAY_QUEUE_FRAMES × MAX_CHUNK × sockets`.
pub const RELAY_QUEUE_FRAMES: usize = 4;

pub const SESSION_CODE_LEN: usize = 8;
/// 30 symbols, no `0/1/I/L/O/U`: unambiguous when read aloud or copied by hand.
pub const SESSION_CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTVWXYZ";
pub const SESSION_TTL_SECS: u64 = 300;
/// Absolute lifetime, refreshes included.
pub const SESSION_MAX_LIFETIME_SECS: u64 = 3600;
/// The owner may reconnect and resume within this window after a disconnect.
pub const SESSION_OWNER_GRACE_SECS: u64 = 120;
pub const SESSION_TOKEN_BYTES: usize = 32;
pub const SESSION_JOIN_ATTEMPTS_PER_MIN: u32 = 5;
pub const SESSION_JOIN_BURST: u32 = 5;
pub const SESSION_GLOBAL_ATTEMPTS_PER_SEC: u32 = 20;
pub const SESSION_MAX_ACTIVE: usize = MAX_GLOBAL_SOCKETS;

pub type CircuitId = String;
pub type SessionId = String;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    CreateSession {
        proto: u8,
        device_id: String,
        alias: String,
    },
    JoinSession {
        proto: u8,
        device_id: String,
        alias: String,
        code: String,
    },
    ResumeSession {
        token: String,
    },
    RefreshSession {
        token: String,
    },
    LeaveSession,
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
    SessionCreated {
        code: String,
        token: String,
        expires_at: u64,
        ttl_secs: u64,
    },
    SessionJoined {
        expires_at: u64,
        ttl_secs: u64,
        peers: Vec<PeerSummary>,
        /// Guest reconnect token. Lets an invited peer come back after a network blip.
        token: String,
    },
    SessionResumed {
        code: String,
        expires_at: u64,
        ttl_secs: u64,
        peers: Vec<PeerSummary>,
    },
    SessionRefreshed {
        code: String,
        expires_at: u64,
        ttl_secs: u64,
    },
    SessionEnded {
        reason: String,
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

/// Normalise a typed code: uppercase, drop spaces and dashes, refuse anything else.
pub fn normalize_code(input: &str) -> Option<String> {
    let cleaned: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if cleaned.chars().count() != SESSION_CODE_LEN {
        return None;
    }
    if cleaned.bytes().all(|b| SESSION_CODE_ALPHABET.contains(&b)) {
        Some(cleaned)
    } else {
        None
    }
}

/// Human-readable form: `ABCD-EFGH`.
pub fn display_code(code: &str) -> String {
    if code.len() == SESSION_CODE_LEN {
        format!("{}-{}", &code[..4], &code[4..])
    } else {
        code.to_string()
    }
}

pub fn code_hash(code: &str) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"droplink/v2/session");
    hasher.update(code.as_bytes());
    *hasher.finalize().as_bytes()
}

pub fn token_hash(token: &str) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"droplink/v2/token");
    hasher.update(token.as_bytes());
    *hasher.finalize().as_bytes()
}

/// Generate a code from an injected RNG (testable, no global state).
pub fn generate_code(rng: &mut impl rand_core::RngCore) -> String {
    let len = SESSION_CODE_ALPHABET.len() as u32;
    let zone = u32::MAX - (u32::MAX % len);
    let mut out = String::with_capacity(SESSION_CODE_LEN);
    for _ in 0..SESSION_CODE_LEN {
        loop {
            let value = rng.next_u32();
            if value < zone {
                out.push(SESSION_CODE_ALPHABET[(value % len) as usize] as char);
                break;
            }
        }
    }
    out
}

/// 32 random bytes encoded as lowercase hex (64 characters).
pub fn generate_token(rng: &mut impl rand_core::RngCore) -> String {
    let mut bytes = [0u8; SESSION_TOKEN_BYTES];
    rng.fill_bytes(&mut bytes);
    let mut out = String::with_capacity(SESSION_TOKEN_BYTES * 2);
    for byte in bytes {
        out.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}

/// Parse a Unix timestamp in seconds, as used on the wire.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Reject a client that speaks another protocol version.
pub fn proto_matches(proto: u8) -> bool {
    proto == PROTOCOL_VERSION
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
    use rand_core::OsRng;

    const CIRCUIT: &str = "33333333-3333-4333-8333-333333333333";

    #[test]
    fn client_msgs_roundtrip() {
        let msgs = [
            ClientMsg::CreateSession {
                proto: PROTOCOL_VERSION,
                device_id: "11111111-1111-4111-8111-111111111111".into(),
                alias: "Mac".into(),
            },
            ClientMsg::JoinSession {
                proto: PROTOCOL_VERSION,
                device_id: "11111111-1111-4111-8111-111111111111".into(),
                alias: "Mac".into(),
                code: "ABCD2345".into(),
            },
            ClientMsg::ResumeSession {
                token: "tok".into(),
            },
            ClientMsg::RefreshSession {
                token: "tok".into(),
            },
            ClientMsg::LeaveSession,
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
            ServerMsg::SessionCreated {
                code: "ABCD2345".into(),
                token: "tok".into(),
                expires_at: 10,
                ttl_secs: SESSION_TTL_SECS,
            },
            ServerMsg::SessionJoined {
                expires_at: 10,
                ttl_secs: SESSION_TTL_SECS,
                peers: vec![],
                token: "guest".into(),
            },
            ServerMsg::SessionResumed {
                code: "ABCD2345".into(),
                expires_at: 10,
                ttl_secs: SESSION_TTL_SECS,
                peers: vec![],
            },
            ServerMsg::SessionRefreshed {
                code: "ABCD2345".into(),
                expires_at: 10,
                ttl_secs: SESSION_TTL_SECS,
            },
            ServerMsg::SessionEnded {
                reason: "expired".into(),
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
    fn legacy_register_is_gone() {
        let json = r#"{"t":"register","device_id":"x","alias":"A"}"#;
        assert!(serde_json::from_str::<ClientMsg>(json).is_err());
    }

    #[test]
    fn code_normalisation() {
        assert_eq!(normalize_code("abcd-2345").as_deref(), Some("ABCD2345"));
        assert_eq!(normalize_code(" ABCD 2345 ").as_deref(), Some("ABCD2345"));
        assert_eq!(normalize_code("ABCD234"), None);
        assert_eq!(normalize_code("ABCD23450"), None);
        assert_eq!(normalize_code("ABCD234I"), None);
        assert_eq!(normalize_code("ABCD234L"), None);
        assert_eq!(normalize_code("ABCD234O"), None);
        assert_eq!(normalize_code("ABCD234U"), None);
        assert_eq!(normalize_code("ABCD2340"), None);
        assert_eq!(normalize_code("ABCD2341"), None);
    }

    #[test]
    fn display_is_grouped() {
        assert_eq!(display_code("ABCD2345"), "ABCD-2345");
        assert_eq!(display_code("short"), "short");
    }

    #[test]
    fn generated_codes_are_in_alphabet_and_distinct() {
        let a = generate_code(&mut OsRng);
        let b = generate_code(&mut OsRng);
        assert_eq!(a.len(), SESSION_CODE_LEN);
        assert_eq!(b.len(), SESSION_CODE_LEN);
        assert!(a.bytes().all(|byte| SESSION_CODE_ALPHABET.contains(&byte)));
        assert_ne!(a, b);
        assert!(normalize_code(&a).is_some());
    }

    #[test]
    fn tokens_are_32_bytes_of_hex() {
        let token = generate_token(&mut OsRng);
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(token, generate_token(&mut OsRng));
    }

    #[test]
    fn hashes_are_stable_and_domain_separated() {
        assert_eq!(code_hash("ABCD2345"), code_hash("ABCD2345"));
        assert_ne!(code_hash("ABCD2345"), code_hash("ABCD2346"));
        assert_ne!(code_hash("ABCD2345"), token_hash("ABCD2345"));
        assert_ne!(token_hash("a"), token_hash("b"));
    }

    #[test]
    fn protocol_version_gate() {
        assert!(proto_matches(PROTOCOL_VERSION));
        assert!(!proto_matches(PROTOCOL_VERSION.wrapping_add(1)));
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
