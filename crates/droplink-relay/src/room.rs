use droplink_core::protocol::PeerId;
use droplink_core::relay::{
    decode_binary, CircuitId, PeerSummary, ServerMsg, MAX_CIRCUITS_PER_PEER, MAX_GLOBAL_SOCKETS,
    MAX_PEERS_PER_ROOM, MAX_SOCKETS_PER_IP,
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
    #[error("duplicate device id")]
    DuplicateId,
    #[error("unknown peer")]
    UnknownPeer,
    #[error("unknown circuit")]
    UnknownCircuit,
    #[error("peers are not in the same room")]
    WrongRoom,
}

struct Peer {
    summary: PeerSummary,
    tx: mpsc::Sender<Outbound>,
}

struct Circuit {
    a: PeerId,
    b: PeerId,
}

/// Relay state, deliberately free of axum: fully testable without a socket.
#[derive(Default)]
pub struct Rooms {
    rooms: HashMap<String, BTreeMap<PeerId, Peer>>,
    peer_room: HashMap<PeerId, String>,
    device_ids: HashMap<String, PeerId>,
    circuits: HashMap<CircuitId, Circuit>,
    per_peer_circuits: HashMap<PeerId, HashSet<CircuitId>>,
    sockets: usize,
    per_ip: HashMap<IpAddr, usize>,
}

impl Rooms {
    pub fn new() -> Self {
        Self::default()
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

    pub fn register(
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

    pub fn unregister(&mut self, peer_id: &str) {
        let Some(room) = self.peer_room.remove(peer_id) else {
            return;
        };
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

    /// Route an opaque payload from `from` to the other side of a circuit.
    pub fn route(&mut self, from: &str, frame: &[u8]) -> Result<(), RelayError> {
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

        let full = {
            let Some(peer) = self.find_peer(&dest) else {
                return Err(RelayError::UnknownPeer);
            };
            peer.tx.try_send(Outbound::Binary(frame.to_vec())).is_err()
        };
        if full {
            self.close_circuit(&circuit_id, "backpressure", Some(&dest));
            return Err(RelayError::Limit("circuit queue"));
        }
        Ok(())
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

    pub fn circuit_count(&self, peer: &str) -> usize {
        self.per_peer_circuits
            .get(peer)
            .map(|set| set.len())
            .unwrap_or(0)
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
    use droplink_core::relay::{encode_binary, MAX_RELAY_FRAME};
    use std::net::{IpAddr, Ipv4Addr};

    fn chan(cap: usize) -> (mpsc::Sender<Outbound>, mpsc::Receiver<Outbound>) {
        mpsc::channel(cap)
    }

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, last))
    }

    fn reg(
        rooms: &mut Rooms,
        room: &str,
        alias: &str,
        cap: usize,
    ) -> (String, mpsc::Receiver<Outbound>) {
        let (tx, rx) = chan(cap);
        let id = rooms
            .register(
                room.into(),
                uuid::Uuid::new_v4().to_string(),
                alias.into(),
                tx,
            )
            .unwrap();
        (id, rx)
    }

    fn drain(rx: &mut mpsc::Receiver<Outbound>) {
        while rx.try_recv().is_ok() {}
    }

    #[tokio::test]
    async fn register_and_broadcast() {
        let mut rooms = Rooms::new();
        let (a, mut rx_a) = reg(&mut rooms, "r", "A", 8);
        let _own_update = rx_a.recv().await.unwrap();
        let (b, mut rx_b) = reg(&mut rooms, "r", "B", 8);

        let update_a = rx_a.recv().await.unwrap();
        match update_a {
            Outbound::Text(ServerMsg::RoomUpdate { peers }) => {
                assert_eq!(peers.len(), 1);
                assert_eq!(peers[0].peer_id, b);
            }
            other => panic!("unexpected {other:?}"),
        }
        let update_b = rx_b.recv().await.unwrap();
        assert!(matches!(
            update_b,
            Outbound::Text(ServerMsg::RoomUpdate { .. })
        ));
        assert_eq!(rooms.peer_count(), 2);
        let _ = a;
    }

    #[tokio::test]
    async fn connect_and_route_both_ways() {
        let mut rooms = Rooms::new();
        let (a, mut rx_a) = reg(&mut rooms, "r", "A", 16);
        let (b, mut rx_b) = reg(&mut rooms, "r", "B", 16);
        drain(&mut rx_a);
        drain(&mut rx_b);

        let circuit = rooms.connect(&a, &b).unwrap();

        let opened_a = rx_a.recv().await.unwrap();
        match opened_a {
            Outbound::Text(ServerMsg::CircuitOpened {
                circuit_id,
                initiator,
                ..
            }) => {
                assert_eq!(circuit_id, circuit);
                assert!(initiator);
            }
            other => panic!("unexpected {other:?}"),
        }
        let opened_b = rx_b.recv().await.unwrap();
        assert!(matches!(
            opened_b,
            Outbound::Text(ServerMsg::CircuitOpened {
                initiator: false,
                ..
            })
        ));

        let frame = encode_binary(&circuit, b"payload").unwrap();
        rooms.route(&a, &frame).unwrap();
        let received = rx_b.recv().await.unwrap();
        assert_eq!(received, Outbound::Binary(frame.clone()));

        rooms.route(&b, &frame).unwrap();
        assert_eq!(rx_a.recv().await.unwrap(), Outbound::Binary(frame));
    }

    #[tokio::test]
    async fn duplicate_device_id_rejected() {
        let mut rooms = Rooms::new();
        let (tx, _rx) = chan(8);
        rooms
            .register("r".into(), "device-1".into(), "A".into(), tx)
            .unwrap();
        let (tx2, _rx2) = chan(8);
        let err = rooms
            .register("r".into(), "device-1".into(), "B".into(), tx2)
            .unwrap_err();
        assert_eq!(err, RelayError::DuplicateId);
    }

    #[tokio::test]
    async fn room_cap_enforced() {
        let mut rooms = Rooms::new();
        for i in 0..MAX_PEERS_PER_ROOM {
            let (tx, _rx) = chan(4);
            rooms
                .register("r".into(), format!("dev-{i}"), format!("P{i}"), tx)
                .unwrap();
        }
        let (tx, _rx) = chan(4);
        let err = rooms
            .register("r".into(), "dev-overflow".into(), "X".into(), tx)
            .unwrap_err();
        assert_eq!(err, RelayError::Limit("room peers"));
    }

    #[tokio::test]
    async fn circuit_cap_enforced() {
        let mut rooms = Rooms::new();
        let (a, _rx_a) = reg(&mut rooms, "r", "A", 64);
        let (b, _rx_b) = reg(&mut rooms, "r", "B", 64);
        for _ in 0..MAX_CIRCUITS_PER_PEER {
            rooms.connect(&a, &b).unwrap();
        }
        assert_eq!(
            rooms.connect(&a, &b).unwrap_err(),
            RelayError::Limit("peer circuits")
        );
    }

    #[tokio::test]
    async fn peers_in_different_rooms_cannot_connect() {
        let mut rooms = Rooms::new();
        let (a, _rx_a) = reg(&mut rooms, "r1", "A", 8);
        let (b, _rx_b) = reg(&mut rooms, "r2", "B", 8);
        assert_eq!(rooms.connect(&a, &b).unwrap_err(), RelayError::WrongRoom);
    }

    #[tokio::test]
    async fn unregister_closes_circuits_and_broadcasts() {
        let mut rooms = Rooms::new();
        let (a, mut rx_a) = reg(&mut rooms, "r", "A", 16);
        let (b, mut rx_b) = reg(&mut rooms, "r", "B", 16);
        drain(&mut rx_a);
        drain(&mut rx_b);
        rooms.connect(&a, &b).unwrap();
        let _ = rx_a.recv().await;
        let _ = rx_b.recv().await;

        rooms.unregister(&b);
        let closed = rx_a.recv().await.unwrap();
        assert!(matches!(
            closed,
            Outbound::Text(ServerMsg::CircuitClosed { .. })
        ));
        let update = rx_a.recv().await.unwrap();
        assert!(matches!(
            update,
            Outbound::Text(ServerMsg::RoomUpdate { peers }) if peers.is_empty()
        ));
        assert_eq!(rooms.peer_count(), 1);
    }

    #[tokio::test]
    async fn slow_reader_closes_circuit_only() {
        let mut rooms = Rooms::new();
        let (a, mut rx_a) = reg(&mut rooms, "r", "A", 16);
        let (b, rx_b) = reg(&mut rooms, "r", "B", 1);
        let _ = rx_a.recv().await;
        drop(rx_b); // B never reads: its queue is now closed
        let circuit = rooms.connect(&a, &b).unwrap();
        let _ = rx_a.recv().await;

        let frame = encode_binary(&circuit, b"x").unwrap();
        let err = rooms.route(&a, &frame).unwrap_err();
        assert_eq!(err, RelayError::Limit("circuit queue"));
    }

    #[tokio::test]
    async fn oversize_relay_frame_rejected() {
        let mut rooms = Rooms::new();
        let (a, _rx_a) = reg(&mut rooms, "r", "A", 8);
        let (b, _rx_b) = reg(&mut rooms, "r", "B", 8);
        let circuit = rooms.connect(&a, &b).unwrap();
        let mut frame = encode_binary(&circuit, b"x").unwrap();
        frame.resize(MAX_RELAY_FRAME + 64, 0);
        assert_eq!(
            rooms.route(&a, &frame).unwrap_err(),
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
