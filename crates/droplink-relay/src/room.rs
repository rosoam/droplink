use crate::limit::SessionLimiter;
use droplink_core::protocol::PeerId;
use droplink_core::relay::{
    code_hash, decode_binary, now_secs, token_hash, CircuitId, PeerSummary, ServerMsg, SessionId,
    MAX_CIRCUITS_PER_PEER, MAX_GLOBAL_SOCKETS, MAX_PEERS_PER_ROOM, MAX_SOCKETS_PER_IP,
    SESSION_MAX_ACTIVE, SESSION_MAX_LIFETIME_SECS, SESSION_OWNER_GRACE_SECS, SESSION_TTL_SECS,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::IpAddr;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub enum Outbound {
    Text(ServerMsg),
    Binary(Vec<u8>),
    Ping,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum RelayError {
    #[error("limit reached: {0}")]
    Limit(&'static str),
    #[error("device already has an active session")]
    AlreadyActive,
    #[error("duplicate device id")]
    DuplicateId,
    #[error("unknown peer")]
    UnknownPeer,
    #[error("unknown circuit")]
    UnknownCircuit,
    #[error("unknown or expired session")]
    UnknownSession,
    #[error("session expired")]
    Expired,
    #[error("peers are not in the same room")]
    WrongRoom,
}

/// A prepared route: the receiver's queue sender, to be awaited **outside** the state lock.
#[derive(Debug)]
pub struct Route {
    pub tx: mpsc::Sender<Outbound>,
    pub circuit_id: CircuitId,
}

struct Peer {
    summary: PeerSummary,
    tx: mpsc::Sender<Outbound>,
}

struct Circuit {
    a: PeerId,
    b: PeerId,
}

#[derive(Clone)]
struct GuestToken {
    session_id: SessionId,
    device_id: String,
    alias: String,
}

struct Session {
    id: SessionId,
    code_hash: [u8; 32],
    token_hash: [u8; 32],
    created_at: u64,
    expires_at: u64,
    owner: Option<PeerId>,
    owner_offline_at: Option<u64>,
    room: String,
    device_id: String,
    alias: String,
}

/// Relay state, deliberately free of axum: fully testable without a socket.
#[derive(Default)]
pub struct Rooms {
    rooms: HashMap<String, BTreeMap<PeerId, Peer>>,
    peer_room: HashMap<PeerId, String>,
    device_ids: HashMap<String, PeerId>,
    circuits: HashMap<CircuitId, Circuit>,
    per_peer_circuits: HashMap<PeerId, HashSet<CircuitId>>,
    sessions: HashMap<SessionId, Session>,
    by_code: HashMap<[u8; 32], SessionId>,
    by_token: HashMap<[u8; 32], SessionId>,
    session_of_peer: HashMap<PeerId, SessionId>,
    guest_tokens: HashMap<[u8; 32], GuestToken>,
    sockets: usize,
    per_ip: HashMap<IpAddr, usize>,
    limiter: SessionLimiter,
}

impl Rooms {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserve one session attempt for this address (see `limit.rs`).
    pub fn allow_session_attempt(&mut self, ip: IpAddr) -> bool {
        self.limiter.allow(ip)
    }

    /// Peers of the same room, excluding `peer_id`.
    pub fn peers_of(&self, peer_id: &str) -> Vec<PeerSummary> {
        let Some(room) = self.peer_room.get(peer_id) else {
            return Vec::new();
        };
        self.rooms
            .get(room)
            .map(|peers| {
                peers
                    .values()
                    .filter(|peer| peer.summary.peer_id != peer_id)
                    .map(|peer| peer.summary.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn try_open_socket(&mut self, ip: IpAddr) -> bool {
        if self.sockets >= MAX_GLOBAL_SOCKETS {
            return false;
        }
        let count = self.per_ip.entry(ip).or_insert(0);
        if *count >= MAX_SOCKETS_PER_IP {
            return false;
        }
        *count += 1;
        self.sockets += 1;
        true
    }

    pub fn close_socket(&mut self, ip: IpAddr) {
        self.sockets = self.sockets.saturating_sub(1);
        if let Some(count) = self.per_ip.get_mut(&ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.per_ip.remove(&ip);
            }
        }
    }

    /// Register a peer inside an existing room. Internal: sessions own the room.
    fn register(
        &mut self,
        room: String,
        device_id: String,
        alias: String,
        tx: mpsc::Sender<Outbound>,
    ) -> Result<PeerId, RelayError> {
        if self.device_ids.contains_key(&device_id) {
            return Err(RelayError::DuplicateId);
        }
        let room_peers = self.rooms.get(&room).map(|r| r.len()).unwrap_or(0);
        if room_peers >= MAX_PEERS_PER_ROOM {
            return Err(RelayError::Limit("room peers"));
        }
        let peer_id = uuid::Uuid::new_v4().to_string();
        self.rooms.entry(room.clone()).or_default().insert(
            peer_id.clone(),
            Peer {
                summary: PeerSummary {
                    peer_id: peer_id.clone(),
                    alias,
                },
                tx,
            },
        );
        self.peer_room.insert(peer_id.clone(), room.clone());
        self.device_ids.insert(device_id, peer_id.clone());
        self.broadcast_room(&room);
        Ok(peer_id)
    }

    /// Create an ephemeral session. The caller supplies the already-generated code and token.
    pub fn create_session(
        &mut self,
        device_id: String,
        alias: String,
        tx: mpsc::Sender<Outbound>,
        now: u64,
        code: String,
        token: String,
    ) -> Result<(PeerId, SessionId, u64), RelayError> {
        if self.device_ids.contains_key(&device_id) {
            return Err(RelayError::AlreadyActive);
        }
        if self.sessions.len() >= SESSION_MAX_ACTIVE {
            return Err(RelayError::Limit("sessions"));
        }
        let session_id = uuid::Uuid::new_v4().to_string();
        // The room id is a random UUID, never derived from the code or the IP.
        let room = format!("session:{session_id}");
        let peer = self.register(room.clone(), device_id.clone(), alias.clone(), tx)?;
        let expires_at = now.saturating_add(SESSION_TTL_SECS);
        let session = Session {
            id: session_id.clone(),
            code_hash: code_hash(&code),
            token_hash: token_hash(&token),
            created_at: now,
            expires_at,
            owner: Some(peer.clone()),
            owner_offline_at: None,
            room,
            device_id,
            alias,
        };
        self.by_code.insert(session.code_hash, session_id.clone());
        self.by_token.insert(session.token_hash, session_id.clone());
        self.session_of_peer
            .insert(peer.clone(), session_id.clone());
        self.sessions.insert(session_id.clone(), session);
        Ok((peer, session_id, expires_at))
    }

    /// Join a session with a code. Unknown, expired and revoked codes are indistinguishable.
    pub fn join_session(
        &mut self,
        code: &str,
        device_id: String,
        alias: String,
        tx: mpsc::Sender<Outbound>,
        now: u64,
        guest_token: String,
    ) -> Result<(PeerId, SessionId, u64), RelayError> {
        let hash = code_hash(code);
        let session_id = self
            .by_code
            .get(&hash)
            .cloned()
            .ok_or(RelayError::UnknownSession)?;
        let (room, expires_at) = {
            let session = self
                .sessions
                .get(&session_id)
                .ok_or(RelayError::UnknownSession)?;
            if session.expires_at <= now {
                return Err(RelayError::UnknownSession);
            }
            (session.room.clone(), session.expires_at)
        };
        let guest = GuestToken {
            session_id: session_id.clone(),
            device_id: device_id.clone(),
            alias: alias.clone(),
        };
        let peer = self.register(room, device_id, alias, tx)?;
        self.session_of_peer
            .insert(peer.clone(), session_id.clone());
        self.guest_tokens.insert(token_hash(&guest_token), guest);
        Ok((peer, session_id, expires_at))
    }

    /// Reattach an invited peer after a disconnect. The session must still exist.
    pub fn resume_guest(
        &mut self,
        token: &str,
        tx: mpsc::Sender<Outbound>,
        now: u64,
        new_token: String,
    ) -> Result<(PeerId, SessionId, u64), RelayError> {
        let hash = token_hash(token);
        let guest = self
            .guest_tokens
            .get(&hash)
            .cloned()
            .ok_or(RelayError::UnknownSession)?;
        let (room, expires_at, created_at) = {
            let session = self
                .sessions
                .get(&guest.session_id)
                .ok_or(RelayError::UnknownSession)?;
            (session.room.clone(), session.expires_at, session.created_at)
        };
        if now >= created_at.saturating_add(SESSION_MAX_LIFETIME_SECS) {
            return Err(RelayError::Expired);
        }
        let peer = self.register(room, guest.device_id.clone(), guest.alias.clone(), tx)?;
        self.session_of_peer
            .insert(peer.clone(), guest.session_id.clone());
        self.guest_tokens.remove(&hash);
        self.guest_tokens.insert(
            token_hash(&new_token),
            GuestToken {
                session_id: guest.session_id.clone(),
                device_id: guest.device_id,
                alias: guest.alias,
            },
        );
        Ok((peer, guest.session_id, expires_at))
    }

    /// Resume ownership after a disconnect, within the grace window. Issues a new code.
    pub fn resume_session(
        &mut self,
        token: &str,
        tx: mpsc::Sender<Outbound>,
        now: u64,
        new_code: String,
    ) -> Result<(PeerId, SessionId, String, u64), RelayError> {
        let token_hash = token_hash(token);
        let session_id = self
            .by_token
            .get(&token_hash)
            .cloned()
            .ok_or(RelayError::UnknownSession)?;
        let (room, device_id, alias, created_at) = {
            let session = self
                .sessions
                .get(&session_id)
                .ok_or(RelayError::UnknownSession)?;
            if session.owner.is_some() {
                return Err(RelayError::UnknownSession);
            }
            let offline = session.owner_offline_at.ok_or(RelayError::UnknownSession)?;
            if now > offline.saturating_add(SESSION_OWNER_GRACE_SECS) {
                return Err(RelayError::Expired);
            }
            if now >= session.created_at.saturating_add(SESSION_MAX_LIFETIME_SECS) {
                return Err(RelayError::Expired);
            }
            (
                session.room.clone(),
                session.device_id.clone(),
                session.alias.clone(),
                session.created_at,
            )
        };
        let peer = self.register(room, device_id, alias, tx)?;
        let expires_at = now
            .saturating_add(SESSION_TTL_SECS)
            .min(created_at.saturating_add(SESSION_MAX_LIFETIME_SECS));
        let hash = code_hash(&new_code);
        let old_hash = {
            let session = self
                .sessions
                .get_mut(&session_id)
                .ok_or(RelayError::UnknownSession)?;
            let old_hash = session.code_hash;
            session.code_hash = hash;
            session.expires_at = expires_at;
            session.owner = Some(peer.clone());
            session.owner_offline_at = None;
            old_hash
        };
        self.by_code.remove(&old_hash);
        self.by_code.insert(hash, session_id.clone());
        self.session_of_peer
            .insert(peer.clone(), session_id.clone());
        Ok((peer, session_id, new_code, expires_at))
    }

    /// Rotate the code of the session owned by `token`. Existing peers and circuits are untouched.
    pub fn refresh_session(
        &mut self,
        token: &str,
        now: u64,
        new_code: String,
    ) -> Result<(String, u64), RelayError> {
        let token_hash = token_hash(token);
        let session_id = self
            .by_token
            .get(&token_hash)
            .cloned()
            .ok_or(RelayError::UnknownSession)?;
        let created_at = {
            let session = self
                .sessions
                .get(&session_id)
                .ok_or(RelayError::UnknownSession)?;
            if session.owner.is_none() {
                return Err(RelayError::UnknownSession);
            }
            session.created_at
        };
        let cap = created_at.saturating_add(SESSION_MAX_LIFETIME_SECS);
        if now >= cap {
            return Err(RelayError::Expired);
        }
        let expires_at = now.saturating_add(SESSION_TTL_SECS).min(cap);
        let hash = code_hash(&new_code);
        let old = {
            let session = self
                .sessions
                .get_mut(&session_id)
                .ok_or(RelayError::UnknownSession)?;
            let old = session.code_hash;
            session.code_hash = hash;
            session.expires_at = expires_at;
            old
        };
        self.by_code.remove(&old);
        self.by_code.insert(hash, session_id.clone());
        Ok((new_code, expires_at))
    }

    /// Leave the session. The owner closes it and gets the guest list to notify.
    pub fn leave_session(&mut self, peer_id: &str) -> Vec<PeerId> {
        let Some(session_id) = self.session_of_peer.get(peer_id).cloned() else {
            return Vec::new();
        };
        let is_owner = self
            .sessions
            .get(&session_id)
            .map(|s| s.owner.as_deref() == Some(peer_id))
            .unwrap_or(false);
        if !is_owner {
            self.unregister(peer_id);
            return Vec::new();
        }
        self.close_session(&session_id, "closed")
            .into_iter()
            .filter(|guest| guest != peer_id)
            .collect()
    }

    /// Expire codes and sessions. Returns the peers that must receive `SessionEnded`.
    pub fn sweep(&mut self, now: u64) -> Vec<(PeerId, &'static str)> {
        let expired_codes: Vec<[u8; 32]> = self
            .sessions
            .values()
            .filter(|session| session.expires_at <= now)
            .map(|session| session.code_hash)
            .collect();
        for hash in expired_codes {
            if let Some(session_id) = self.by_code.get(&hash).cloned() {
                let expired = self
                    .sessions
                    .get(&session_id)
                    .map(|session| session.expires_at <= now)
                    .unwrap_or(true);
                if expired {
                    self.by_code.remove(&hash);
                }
            }
        }

        let dead: Vec<SessionId> = self
            .sessions
            .values()
            .filter(|session| {
                let offline_expired = session
                    .owner_offline_at
                    .map(|at| now > at.saturating_add(SESSION_OWNER_GRACE_SECS))
                    .unwrap_or(false);
                let lifetime_expired =
                    now >= session.created_at.saturating_add(SESSION_MAX_LIFETIME_SECS);
                offline_expired || lifetime_expired
            })
            .map(|session| session.id.clone())
            .collect();

        let mut notify = Vec::new();
        for session_id in dead {
            let reason = if self
                .sessions
                .get(&session_id)
                .map(|session| {
                    session.owner_offline_at.is_some()
                        && now
                            > session
                                .owner_offline_at
                                .unwrap_or(0)
                                .saturating_add(SESSION_OWNER_GRACE_SECS)
                })
                .unwrap_or(false)
            {
                "owner-left"
            } else {
                "expired"
            };
            for peer in self.close_session(&session_id, reason) {
                notify.push((peer, reason));
            }
        }
        notify
    }

    /// Close a session: invalidate its code/token, notify and unregister every peer in its room.
    fn close_session(&mut self, session_id: &str, reason: &str) -> Vec<PeerId> {
        let Some(session) = self.sessions.remove(session_id) else {
            return Vec::new();
        };
        self.by_code.remove(&session.code_hash);
        self.by_token.remove(&session.token_hash);
        self.guest_tokens
            .retain(|_, guest| guest.session_id.as_str() != session_id);
        let peers: Vec<PeerId> = self
            .rooms
            .get(&session.room)
            .map(|room| room.keys().cloned().collect())
            .unwrap_or_default();
        for peer in &peers {
            self.session_of_peer.remove(peer);
        }
        for peer in &peers {
            self.send_to(
                peer,
                Outbound::Text(ServerMsg::SessionEnded {
                    reason: reason.to_string(),
                }),
            );
        }
        for peer in &peers {
            self.unregister(peer);
        }
        peers
    }

    pub fn unregister(&mut self, peer_id: &str) {
        self.unregister_at(peer_id, now_secs());
    }

    /// Testable variant: the caller supplies the clock.
    pub fn unregister_at(&mut self, peer_id: &str, now: u64) {
        let Some(room) = self.peer_room.remove(peer_id) else {
            return;
        };
        if let Some(session_id) = self.session_of_peer.remove(peer_id) {
            if let Some(session) = self.sessions.get_mut(&session_id) {
                if session.owner.as_deref() == Some(peer_id) {
                    session.owner = None;
                    session.owner_offline_at = Some(now);
                    // The code dies with the owner's connection: no new join while away.
                    self.by_code.remove(&session.code_hash);
                }
            }
        }
        if let Some(peers) = self.rooms.get_mut(&room) {
            peers.remove(peer_id);
            if peers.is_empty() {
                self.rooms.remove(&room);
            }
        }
        self.per_peer_circuits.remove(peer_id);
        self.device_ids.retain(|_, id| id != peer_id);

        let circuits: Vec<CircuitId> = self
            .circuits
            .iter()
            .filter(|(_, c)| c.a == peer_id || c.b == peer_id)
            .map(|(id, _)| id.clone())
            .collect();
        for circuit in circuits {
            self.close_circuit(&circuit, "peer disconnected", Some(peer_id));
        }
        self.broadcast_room(&room);
    }

    pub fn connect(&mut self, from: &str, target: &str) -> Result<CircuitId, RelayError> {
        if from == target {
            return Err(RelayError::UnknownPeer);
        }
        let from_room = self
            .peer_room
            .get(from)
            .ok_or(RelayError::UnknownPeer)?
            .clone();
        let target_room = self
            .peer_room
            .get(target)
            .ok_or(RelayError::UnknownPeer)?
            .clone();
        if from_room != target_room {
            return Err(RelayError::WrongRoom);
        }
        if self.circuit_count(from) >= MAX_CIRCUITS_PER_PEER
            || self.circuit_count(target) >= MAX_CIRCUITS_PER_PEER
        {
            return Err(RelayError::Limit("peer circuits"));
        }

        let circuit_id = uuid::Uuid::new_v4().to_string();
        self.circuits.insert(
            circuit_id.clone(),
            Circuit {
                a: from.to_string(),
                b: target.to_string(),
            },
        );
        self.per_peer_circuits
            .entry(from.to_string())
            .or_default()
            .insert(circuit_id.clone());
        self.per_peer_circuits
            .entry(target.to_string())
            .or_default()
            .insert(circuit_id.clone());

        let from_summary = self.summary(from).ok_or(RelayError::UnknownPeer)?;
        let target_summary = self.summary(target).ok_or(RelayError::UnknownPeer)?;

        self.send_to(
            target,
            Outbound::Text(ServerMsg::CircuitOpened {
                circuit_id: circuit_id.clone(),
                peer: from_summary,
                initiator: false,
            }),
        );
        self.send_to(
            from,
            Outbound::Text(ServerMsg::CircuitOpened {
                circuit_id: circuit_id.clone(),
                peer: target_summary,
                initiator: true,
            }),
        );
        Ok(circuit_id)
    }

    /// Resolve the destination of an opaque payload and clone its queue sender.
    ///
    /// The caller must `send().await` **outside** the lock: a slow reader then slows the
    /// sender's TCP stream instead of losing the transfer.
    pub fn prepare_route(&mut self, from: &str, frame: &[u8]) -> Result<Route, RelayError> {
        let (circuit_id, _payload) =
            decode_binary(frame).map_err(|_| RelayError::UnknownCircuit)?;
        let circuit = self
            .circuits
            .get(&circuit_id)
            .ok_or(RelayError::UnknownCircuit)?;
        let dest = if circuit.a == from {
            circuit.b.clone()
        } else if circuit.b == from {
            circuit.a.clone()
        } else {
            return Err(RelayError::UnknownCircuit);
        };
        let tx = self
            .find_peer(&dest)
            .map(|peer| peer.tx.clone())
            .ok_or(RelayError::UnknownPeer)?;
        Ok(Route { tx, circuit_id })
    }

    pub fn close_circuit(&mut self, circuit_id: &str, reason: &str, except: Option<&str>) {
        let Some(circuit) = self.circuits.remove(circuit_id) else {
            return;
        };
        for peer in [circuit.a, circuit.b] {
            if let Some(set) = self.per_peer_circuits.get_mut(&peer) {
                set.remove(circuit_id);
            }
            if except == Some(peer.as_str()) {
                continue;
            }
            self.send_to(
                &peer,
                Outbound::Text(ServerMsg::CircuitClosed {
                    circuit_id: circuit_id.to_string(),
                    reason: reason.to_string(),
                }),
            );
        }
    }

    pub fn peer_count(&self) -> usize {
        self.peer_room.len()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn circuit_count(&self, peer: &str) -> usize {
        self.per_peer_circuits
            .get(peer)
            .map(|set| set.len())
            .unwrap_or(0)
    }

    pub fn total_circuits(&self) -> usize {
        self.circuits.len()
    }

    pub fn socket_count(&self) -> usize {
        self.sockets
    }

    fn summary(&self, peer: &str) -> Option<PeerSummary> {
        self.find_peer(peer).map(|p| p.summary.clone())
    }

    fn find_peer(&self, peer: &str) -> Option<&Peer> {
        let room = self.peer_room.get(peer)?;
        self.rooms.get(room)?.get(peer)
    }

    fn send_to(&self, peer: &str, message: Outbound) {
        if let Some(target) = self.find_peer(peer) {
            let _ = target.tx.try_send(message);
        }
    }

    fn broadcast_room(&self, room: &str) {
        let Some(peers) = self.rooms.get(room) else {
            return;
        };
        let summaries: Vec<PeerSummary> = peers.values().map(|p| p.summary.clone()).collect();
        for (peer_id, peer) in peers {
            let others: Vec<PeerSummary> = summaries
                .iter()
                .filter(|s| &s.peer_id != peer_id)
                .cloned()
                .collect();
            let _ = peer
                .tx
                .try_send(Outbound::Text(ServerMsg::RoomUpdate { peers: others }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use droplink_core::relay::{
        encode_binary, generate_code, generate_token, MAX_RELAY_FRAME, SESSION_TTL_SECS,
    };
    use rand_core::OsRng;
    use std::net::{IpAddr, Ipv4Addr};

    fn chan(cap: usize) -> (mpsc::Sender<Outbound>, mpsc::Receiver<Outbound>) {
        mpsc::channel(cap)
    }

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, last))
    }

    struct Node {
        peer: PeerId,
        session: SessionId,
        rx: mpsc::Receiver<Outbound>,
    }

    struct Created {
        node: Node,
        code: String,
        token: String,
        expires: u64,
    }

    fn create(rooms: &mut Rooms, alias: &str, now: u64) -> Created {
        let (tx, rx) = chan(64);
        let code = generate_code(&mut OsRng);
        let token = generate_token(&mut OsRng);
        let (peer, session, expires) = rooms
            .create_session(
                uuid::Uuid::new_v4().to_string(),
                alias.into(),
                tx,
                now,
                code.clone(),
                token.clone(),
            )
            .unwrap();
        Created {
            node: Node { peer, session, rx },
            code,
            token,
            expires,
        }
    }

    fn join(rooms: &mut Rooms, code: &str, alias: &str, now: u64) -> Node {
        let (tx, rx) = chan(64);
        let (peer, session, _) = rooms
            .join_session(
                code,
                uuid::Uuid::new_v4().to_string(),
                alias.into(),
                tx,
                now,
                generate_token(&mut OsRng),
            )
            .unwrap();
        Node { peer, session, rx }
    }

    fn drain(rx: &mut mpsc::Receiver<Outbound>) {
        while rx.try_recv().is_ok() {}
    }

    #[tokio::test]
    async fn session_create_join_and_broadcast() {
        let mut rooms = Rooms::new();
        let mut a = create(&mut rooms, "A", 1_000);
        let _own = a.node.rx.recv().await.unwrap();
        assert_eq!(a.expires, 1_000 + SESSION_TTL_SECS);
        let mut b = join(&mut rooms, &a.code, "B", 1_001);
        let update = a.node.rx.recv().await.unwrap();
        match update {
            Outbound::Text(ServerMsg::RoomUpdate { peers }) => {
                assert_eq!(peers.len(), 1);
                assert_eq!(peers[0].peer_id, b.peer);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            b.rx.recv().await.unwrap(),
            Outbound::Text(ServerMsg::RoomUpdate { .. })
        ));
        assert_eq!(rooms.peer_count(), 2);
        assert_eq!(rooms.session_count(), 1);
        assert_eq!(a.node.session, b.session);
    }

    #[tokio::test]
    async fn unknown_or_expired_code_is_the_same_error() {
        let mut rooms = Rooms::new();
        let a = create(&mut rooms, "A", 1_000);
        assert_eq!(
            rooms
                .join_session(
                    "ZZZZ2345",
                    "d".into(),
                    "B".into(),
                    chan(4).0,
                    1_001,
                    generate_token(&mut OsRng),
                )
                .unwrap_err(),
            RelayError::UnknownSession
        );
        assert_eq!(
            rooms
                .join_session(
                    &a.code,
                    "d".into(),
                    "B".into(),
                    chan(4).0,
                    1_000 + SESSION_TTL_SECS + 1,
                    generate_token(&mut OsRng),
                )
                .unwrap_err(),
            RelayError::UnknownSession
        );
    }

    #[tokio::test]
    async fn duplicate_device_id_never_kills_the_existing_session() {
        let mut rooms = Rooms::new();
        let (tx, _rx) = chan(8);
        rooms
            .create_session(
                "device-1".into(),
                "A".into(),
                tx,
                1_000,
                generate_code(&mut OsRng),
                generate_token(&mut OsRng),
            )
            .unwrap();
        let (tx2, _rx2) = chan(8);
        let err = rooms
            .create_session(
                "device-1".into(),
                "B".into(),
                tx2,
                1_001,
                generate_code(&mut OsRng),
                generate_token(&mut OsRng),
            )
            .unwrap_err();
        assert_eq!(err, RelayError::AlreadyActive);
        assert_eq!(rooms.session_count(), 1);
    }

    #[tokio::test]
    async fn refresh_rotates_the_code_without_touching_peers() {
        let mut rooms = Rooms::new();
        let a = create(&mut rooms, "A", 1_000);
        let mut b = join(&mut rooms, &a.code, "B", 1_001);
        drain(&mut b.rx);
        let new_code = generate_code(&mut OsRng);
        let (returned, expires) = rooms
            .refresh_session(&a.token, 1_002, new_code.clone())
            .unwrap();
        assert_eq!(returned, new_code);
        assert!(expires > 1_000);
        assert_eq!(
            rooms
                .join_session(
                    &a.code,
                    "d2".into(),
                    "C".into(),
                    chan(4).0,
                    1_003,
                    generate_token(&mut OsRng),
                )
                .unwrap_err(),
            RelayError::UnknownSession
        );
        let _c = join(&mut rooms, &new_code, "C", 1_003);
        assert_eq!(rooms.peer_count(), 3);
    }

    #[tokio::test]
    async fn owner_disconnect_keeps_guests_and_kills_the_code() {
        let mut rooms = Rooms::new();
        let a = create(&mut rooms, "A", 1_000);
        let mut b = join(&mut rooms, &a.code, "B", 1_001);
        drain(&mut b.rx);
        rooms.unregister_at(&a.node.peer, 1_001);
        assert_eq!(rooms.peer_count(), 1);
        assert_eq!(
            rooms
                .join_session(
                    &a.code,
                    "d2".into(),
                    "C".into(),
                    chan(4).0,
                    1_002,
                    generate_token(&mut OsRng),
                )
                .unwrap_err(),
            RelayError::UnknownSession
        );
        let notified = rooms.sweep(1_001 + SESSION_OWNER_GRACE_SECS + 1);
        assert_eq!(notified.len(), 1);
        assert_eq!(notified[0].0, b.peer);
        assert_eq!(rooms.session_count(), 0);
        assert_eq!(rooms.peer_count(), 0);
    }

    #[tokio::test]
    async fn resume_inside_the_grace_issues_a_new_code() {
        let mut rooms = Rooms::new();
        let a = create(&mut rooms, "A", 1_000);
        let _b = join(&mut rooms, &a.code, "B", 1_001);
        rooms.unregister_at(&a.node.peer, 1_001);

        let new_code = generate_code(&mut OsRng);
        let (tx, _rx) = chan(8);
        let (peer, _, returned, expires) = rooms
            .resume_session(&a.token, tx, 1_050, new_code.clone())
            .unwrap();
        assert_eq!(returned, new_code);
        assert!(expires > 1_050);
        assert_eq!(rooms.peer_count(), 2);
        assert!(rooms.leave_session(&peer).len() == 1);
    }

    #[tokio::test]
    async fn resume_after_grace_is_refused() {
        let mut rooms = Rooms::new();
        let a = create(&mut rooms, "A", 1_000);
        rooms.unregister_at(&a.node.peer, 1_001);
        let (tx, _rx) = chan(8);
        let err = rooms
            .resume_session(
                &a.token,
                tx,
                1_001 + SESSION_OWNER_GRACE_SECS + 1,
                generate_code(&mut OsRng),
            )
            .unwrap_err();
        assert_eq!(err, RelayError::Expired);
    }

    #[tokio::test]
    async fn lifetime_cap_stops_refresh() {
        let mut rooms = Rooms::new();
        let a = create(&mut rooms, "A", 1_000);
        let err = rooms
            .refresh_session(
                &a.token,
                1_000 + SESSION_MAX_LIFETIME_SECS,
                generate_code(&mut OsRng),
            )
            .unwrap_err();
        assert_eq!(err, RelayError::Expired);
    }

    #[tokio::test]
    async fn guest_can_resume_after_a_disconnect() {
        let mut rooms = Rooms::new();
        let a = create(&mut rooms, "A", 1_000);
        let guest_token = generate_token(&mut OsRng);
        let (tx, _rx) = chan(8);
        let (guest_peer, guest_session, _) = rooms
            .join_session(
                &a.code,
                "guest".into(),
                "B".into(),
                tx,
                1_001,
                guest_token.clone(),
            )
            .unwrap();
        rooms.unregister_at(&guest_peer, 1_002);

        let new_token = generate_token(&mut OsRng);
        let (tx2, _rx2) = chan(8);
        let (peer, session, _) = rooms
            .resume_guest(&guest_token, tx2, 1_003, new_token.clone())
            .unwrap();
        assert_eq!(session, guest_session);
        assert_eq!(rooms.peer_count(), 2);
        assert!(rooms.peer_room.contains_key(&peer));
        // The old guest token is consumed.
        assert_eq!(
            rooms
                .resume_guest(&guest_token, chan(4).0, 1_004, generate_token(&mut OsRng))
                .unwrap_err(),
            RelayError::UnknownSession
        );
    }

    #[tokio::test]
    async fn connect_and_route_both_ways() {
        let mut rooms = Rooms::new();
        let mut a = create(&mut rooms, "A", 1_000);
        let mut b = join(&mut rooms, &a.code, "B", 1_001);
        drain(&mut a.node.rx);
        drain(&mut b.rx);

        let circuit = rooms.connect(&a.node.peer, &b.peer).unwrap();
        let opened_a = a.node.rx.recv().await.unwrap();
        assert!(matches!(
            opened_a,
            Outbound::Text(ServerMsg::CircuitOpened {
                initiator: true,
                ..
            })
        ));
        let _opened_b = b.rx.recv().await.unwrap();

        let frame = encode_binary(&circuit, b"payload").unwrap();
        let route = rooms.prepare_route(&a.node.peer, &frame).unwrap();
        route
            .tx
            .send(Outbound::Binary(frame.clone()))
            .await
            .unwrap();
        assert_eq!(b.rx.recv().await.unwrap(), Outbound::Binary(frame.clone()));

        let route = rooms.prepare_route(&b.peer, &frame).unwrap();
        route
            .tx
            .send(Outbound::Binary(frame.clone()))
            .await
            .unwrap();
        assert_eq!(a.node.rx.recv().await.unwrap(), Outbound::Binary(frame));
    }

    #[tokio::test]
    async fn circuit_cap_enforced() {
        let mut rooms = Rooms::new();
        let a = create(&mut rooms, "A", 1_000);
        let b = join(&mut rooms, &a.code, "B", 1_001);
        for _ in 0..MAX_CIRCUITS_PER_PEER {
            rooms.connect(&a.node.peer, &b.peer).unwrap();
        }
        assert_eq!(
            rooms.connect(&a.node.peer, &b.peer).unwrap_err(),
            RelayError::Limit("peer circuits")
        );
    }

    #[tokio::test]
    async fn peers_in_different_sessions_cannot_connect() {
        let mut rooms = Rooms::new();
        let a = create(&mut rooms, "A", 1_000);
        let b = create(&mut rooms, "B", 1_000);
        assert_eq!(
            rooms.connect(&a.node.peer, &b.node.peer).unwrap_err(),
            RelayError::WrongRoom
        );
    }

    #[tokio::test]
    async fn unregister_closes_circuits_and_broadcasts() {
        let mut rooms = Rooms::new();
        let mut a = create(&mut rooms, "A", 1_000);
        let mut b = join(&mut rooms, &a.code, "B", 1_001);
        drain(&mut a.node.rx);
        drain(&mut b.rx);
        rooms.connect(&a.node.peer, &b.peer).unwrap();
        let _ = a.node.rx.recv().await;
        let _ = b.rx.recv().await;

        rooms.unregister_at(&b.peer, 1_001);
        let closed = a.node.rx.recv().await.unwrap();
        assert!(matches!(
            closed,
            Outbound::Text(ServerMsg::CircuitClosed { .. })
        ));
        let update = a.node.rx.recv().await.unwrap();
        assert!(matches!(
            update,
            Outbound::Text(ServerMsg::RoomUpdate { peers }) if peers.is_empty()
        ));
        assert_eq!(rooms.peer_count(), 1);
    }

    #[tokio::test]
    async fn slow_reader_route_exposes_backpressure() {
        let mut rooms = Rooms::new();
        let mut a = create(&mut rooms, "A", 1_000);
        let (tx, rx_b) = chan(1);
        let b_peer = rooms
            .join_session(
                &a.code,
                "b".into(),
                "B".into(),
                tx,
                1_001,
                generate_token(&mut OsRng),
            )
            .unwrap()
            .0;
        drain(&mut a.node.rx);
        // The join broadcast filled the capacity-1 queue: drain it before exercising backpressure.
        let mut rx_b = rx_b;
        drain(&mut rx_b);
        let circuit = rooms.connect(&a.node.peer, &b_peer).unwrap();
        let _ = a.node.rx.recv().await;
        // Both sides receive `CircuitOpened`: drain B's queue before testing backpressure.
        drain(&mut rx_b);

        let frame = encode_binary(&circuit, b"x").unwrap();
        let route = rooms.prepare_route(&a.node.peer, &frame).unwrap();
        route.tx.try_send(Outbound::Binary(frame.clone())).unwrap();
        let route = rooms.prepare_route(&a.node.peer, &frame).unwrap();
        assert!(route.tx.try_send(Outbound::Binary(frame)).is_err());
        drop(rx_b);
    }

    #[tokio::test]
    async fn oversize_relay_frame_rejected() {
        let mut rooms = Rooms::new();
        let a = create(&mut rooms, "A", 1_000);
        let b = join(&mut rooms, &a.code, "B", 1_001);
        let circuit = rooms.connect(&a.node.peer, &b.peer).unwrap();
        let mut frame = encode_binary(&circuit, b"x").unwrap();
        frame.resize(MAX_RELAY_FRAME + 64, 0);
        assert_eq!(
            rooms.prepare_route(&a.node.peer, &frame).unwrap_err(),
            RelayError::UnknownCircuit
        );
    }

    #[tokio::test]
    async fn socket_caps() {
        let mut rooms = Rooms::new();
        for _ in 0..MAX_SOCKETS_PER_IP {
            assert!(rooms.try_open_socket(ip(1)));
        }
        assert!(!rooms.try_open_socket(ip(1)));
        assert!(rooms.try_open_socket(ip(2)));
        rooms.close_socket(ip(1));
        assert!(rooms.try_open_socket(ip(1)));
    }
}
