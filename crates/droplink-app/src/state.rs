use droplink_core::peer::{Peer, PeerKey};
use droplink_core::protocol::{Direction, TransferId, TransportMode};
use droplink_net::engine::{Event, IncomingTransfer};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq)]
pub struct TransferRow {
    pub transfer_id: TransferId,
    pub file_name: String,
    pub direction: Direction,
    pub bytes: u64,
    pub total: u64,
    pub transport: TransportMode,
    pub started: Instant,
    /// Smoothed progress used for the animated bar.
    pub displayed: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub file_name: String,
    pub direction: Direction,
    pub ok: bool,
    pub path: Option<String>,
    pub transport: TransportMode,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UiAction {
    Send {
        peer: Peer,
        path: PathBuf,
    },
    Respond {
        transfer_id: TransferId,
        accept: bool,
    },
    Cancel {
        transfer_id: TransferId,
    },
    Open {
        path: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub ok: bool,
    pub text: String,
    pub at: Instant,
}

impl Outcome {
    pub fn expired(&self) -> bool {
        self.at.elapsed().as_secs_f32() > 2.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub at: Instant,
}

impl Toast {
    pub fn success(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: ToastKind::Success,
            at: Instant::now(),
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: ToastKind::Error,
            at: Instant::now(),
        }
    }

    pub fn expired(&self) -> bool {
        self.at.elapsed().as_secs_f32() > 4.0
    }
}

#[derive(Default)]
pub struct UiState {
    pub devices: Vec<Peer>,
    pub selected: Option<PeerKey>,
    pub queue: Vec<TransferRow>,
    pub incoming: HashMap<TransferId, IncomingTransfer>,
    pub sas: HashMap<TransferId, String>,
    pub history: Vec<HistoryEntry>,
    pub discovery_available: bool,
    pub settings_open: bool,
    pub toast: Option<Toast>,
    pub outcome: Option<Outcome>,
    pub history_limit: usize,
}

impl UiState {
    pub fn new(history_limit: usize) -> Self {
        Self {
            discovery_available: true,
            history_limit: history_limit.max(1),
            ..Default::default()
        }
    }

    /// Click a peer: select it, or unselect it when it is already selected.
    pub fn toggle_selected(&mut self, key: PeerKey) {
        if self.selected.as_ref() == Some(&key) {
            self.selected = None;
        } else {
            self.selected = Some(key);
        }
    }

    pub fn selected_peer(&self) -> Option<&Peer> {
        let key = self.selected.as_ref()?;
        self.devices.iter().find(|peer| peer_matches(peer, key))
    }

    /// Translate a native drop into one `Send` action per file (never a batch: the engine
    /// serialises per device).
    pub fn on_files_dropped(&self, paths: Vec<PathBuf>) -> Vec<UiAction> {
        let Some(peer) = self.selected_peer().cloned() else {
            return Vec::new();
        };
        paths
            .into_iter()
            .filter(|path| path.is_file())
            .map(|path| UiAction::Send {
                peer: peer.clone(),
                path,
            })
            .collect()
    }

    pub fn apply(&mut self, event: Event) {
        match event {
            Event::DevicesChanged(peers) => {
                self.devices = peers;
                if let Some(key) = &self.selected {
                    if !self.devices.iter().any(|peer| peer_matches(peer, key)) {
                        self.selected = None;
                    }
                }
            }
            Event::Queued {
                transfer_id,
                file_name,
                direction,
            } => {
                if !self.queue.iter().any(|row| row.transfer_id == transfer_id) {
                    self.queue.push(TransferRow {
                        transfer_id,
                        file_name,
                        direction,
                        bytes: 0,
                        total: 0,
                        transport: TransportMode::Direct,
                        started: Instant::now(),
                        displayed: 0.0,
                    });
                }
            }
            Event::Incoming(incoming) => {
                if !self
                    .queue
                    .iter()
                    .any(|row| row.transfer_id == incoming.transfer_id)
                {
                    self.queue.push(TransferRow {
                        transfer_id: incoming.transfer_id.clone(),
                        file_name: incoming.file_name.clone(),
                        direction: Direction::Receive,
                        bytes: 0,
                        total: incoming.file_size,
                        transport: incoming.transport,
                        started: Instant::now(),
                        displayed: 0.0,
                    });
                }
                self.sas
                    .insert(incoming.transfer_id.clone(), incoming.sas.clone());
                self.incoming.insert(incoming.transfer_id.clone(), incoming);
            }
            Event::IncomingExpired { transfer_id } => {
                self.incoming.remove(&transfer_id);
                self.sas.remove(&transfer_id);
            }
            Event::Sas { transfer_id, code } => {
                self.sas.insert(transfer_id, code);
            }
            Event::Progress {
                transfer_id,
                file_name,
                bytes,
                total,
                direction,
                transport,
            } => {
                if let Some(row) = self
                    .queue
                    .iter_mut()
                    .find(|row| row.transfer_id == transfer_id)
                {
                    if bytes >= row.bytes {
                        row.bytes = bytes;
                    }
                    row.total = total;
                    row.transport = transport;
                    row.direction = direction;
                    row.file_name = file_name;
                }
            }
            Event::Completed {
                transfer_id,
                direction,
                file_name,
                path,
                transport,
            } => {
                let bytes = self
                    .queue
                    .iter()
                    .find(|row| row.transfer_id == transfer_id)
                    .map(|row| row.bytes)
                    .unwrap_or(0);
                self.queue.retain(|row| row.transfer_id != transfer_id);
                self.incoming.remove(&transfer_id);
                self.sas.remove(&transfer_id);
                self.toast = None;
                self.outcome = Some(Outcome {
                    ok: true,
                    text: format!(
                        "{} · {}",
                        file_name,
                        if direction == Direction::Send {
                            "envoyé"
                        } else {
                            "reçu"
                        }
                    ),
                    at: Instant::now(),
                });
                self.push_history(HistoryEntry {
                    file_name,
                    direction,
                    ok: true,
                    path: path.map(|p| p.to_string_lossy().to_string()),
                    transport,
                    bytes,
                });
            }
            Event::Failed {
                transfer_id,
                reason,
                ..
            } => {
                self.queue.retain(|row| row.transfer_id != transfer_id);
                self.incoming.remove(&transfer_id);
                self.sas.remove(&transfer_id);
                self.outcome = Some(Outcome {
                    ok: false,
                    text: "échec du transfert".to_string(),
                    at: Instant::now(),
                });
                self.toast = Some(Toast::error(reason));
            }
        }
    }

    pub fn tick(&mut self) {
        if self.toast.as_ref().map(Toast::expired).unwrap_or(false) {
            self.toast = None;
        }
        if self.outcome.as_ref().map(Outcome::expired).unwrap_or(false) {
            self.outcome = None;
        }
    }

    /// Smooth the progress bars towards their target so they advance continuously between events.
    pub fn animate(&mut self, dt: f32) {
        let factor = (dt * 14.0).clamp(0.0, 1.0);
        for row in &mut self.queue {
            let target = if row.total > 0 {
                (row.bytes as f32 / row.total as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            row.displayed += (target - row.displayed) * factor;
            if row.displayed > target {
                row.displayed = target;
            }
        }
    }

    fn push_history(&mut self, entry: HistoryEntry) {
        self.history.insert(0, entry);
        self.history.truncate(self.history_limit);
    }
}

pub fn peer_matches(peer: &Peer, key: &PeerKey) -> bool {
    match key {
        PeerKey::Direct(id) => peer.device_id.as_deref() == Some(id.as_str()),
        PeerKey::Relay(peer_id) => {
            peer.reach.relay_id().map(String::as_str) == Some(peer_id.as_str())
        }
    }
}

pub fn peer_key(peer: &Peer) -> PeerKey {
    if let Some(peer_id) = peer.reach.relay_id() {
        return PeerKey::Relay(peer_id.clone());
    }
    PeerKey::Direct(peer.device_id.clone().unwrap_or_else(|| peer.alias.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use droplink_core::peer::Reach;
    use std::net::{IpAddr, Ipv4Addr};

    fn peer(id: &str, alias: &str, port: u16) -> Peer {
        Peer {
            device_id: Some(id.into()),
            alias: alias.into(),
            reach: Reach::Direct {
                ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port,
            },
        }
    }

    fn relay_peer(peer_id: &str, alias: &str) -> Peer {
        Peer {
            device_id: None,
            alias: alias.into(),
            reach: Reach::Relay {
                peer_id: peer_id.into(),
            },
        }
    }

    fn queued(id: &str, name: &str) -> Event {
        Event::Queued {
            transfer_id: id.into(),
            file_name: name.into(),
            direction: Direction::Send,
        }
    }

    fn progress(id: &str, bytes: u64, total: u64) -> Event {
        Event::Progress {
            transfer_id: id.into(),
            file_name: "f.bin".into(),
            bytes,
            total,
            direction: Direction::Send,
            transport: TransportMode::Direct,
        }
    }

    #[test]
    fn drop_three_paths_enqueue_three_rows() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = UiState::new(10);
        state.apply(Event::DevicesChanged(vec![peer("dev-1", "Mac", 7777)]));
        state.selected = Some(PeerKey::Direct("dev-1".into()));

        let paths: Vec<PathBuf> = (0..3)
            .map(|i| {
                let path = dir.path().join(format!("f{i}.txt"));
                std::fs::write(&path, b"x").unwrap();
                path
            })
            .collect();
        let actions = state.on_files_dropped(paths);
        assert_eq!(actions.len(), 3);
        for (i, _) in (0..3).enumerate() {
            state.apply(queued(&format!("t{i}"), &format!("f{i}.txt")));
        }
        assert_eq!(state.queue.len(), 3);
    }

    #[test]
    fn queued_out_of_order_still_matches() {
        let mut state = UiState::new(10);
        state.apply(progress("t1", 10, 100));
        assert!(state.queue.is_empty());
        state.apply(queued("t1", "f.bin"));
        state.apply(progress("t1", 40, 100));
        assert_eq!(state.queue[0].bytes, 40);
    }

    #[test]
    fn progress_decreasing_ignored() {
        let mut state = UiState::new(10);
        state.apply(queued("t1", "f.bin"));
        state.apply(progress("t1", 50, 100));
        state.apply(progress("t1", 30, 100));
        assert_eq!(state.queue[0].bytes, 50);
    }

    #[test]
    fn completed_moves_to_history() {
        let mut state = UiState::new(10);
        state.apply(queued("t1", "f.bin"));
        state.apply(progress("t1", 100, 100));
        state.apply(Event::Completed {
            transfer_id: "t1".into(),
            direction: Direction::Send,
            file_name: "f.bin".into(),
            path: None,
            transport: TransportMode::Direct,
        });
        assert!(state.queue.is_empty());
        assert_eq!(state.history.len(), 1);
        assert!(state.history[0].ok);
        assert_eq!(state.history[0].bytes, 100);
        assert!(state.toast.is_none());
        assert!(matches!(state.outcome.as_ref(), Some(outcome) if outcome.ok));
    }

    #[test]
    fn failed_is_correlated() {
        let mut state = UiState::new(10);
        state.apply(queued("t1", "a.bin"));
        state.apply(queued("t2", "b.bin"));
        state.apply(Event::Failed {
            transfer_id: "t2".into(),
            direction: Direction::Send,
            file_name: "b.bin".into(),
            reason: "boom".into(),
            transport: TransportMode::Direct,
        });
        assert_eq!(state.queue.len(), 1);
        assert_eq!(state.queue[0].transfer_id, "t1");
        assert!(
            state.history.is_empty(),
            "failed transfers must not be listed"
        );
        assert!(matches!(
            state.toast.as_ref().map(|toast| toast.kind),
            Some(ToastKind::Error)
        ));
    }

    #[test]
    fn three_incoming_three_modals() {
        let mut state = UiState::new(10);
        for i in 0..3 {
            state.apply(Event::Incoming(IncomingTransfer {
                transfer_id: format!("in{i}"),
                sender_alias: "A".into(),
                file_name: format!("f{i}.bin"),
                file_size: 10,
                transport: TransportMode::Relay,
                sas: "123456".into(),
            }));
        }
        assert_eq!(state.incoming.len(), 3);
        assert_eq!(state.sas.len(), 3);
    }

    #[test]
    fn incoming_expired_removes_only_that() {
        let mut state = UiState::new(10);
        for i in 0..2 {
            state.apply(Event::Incoming(IncomingTransfer {
                transfer_id: format!("in{i}"),
                sender_alias: "A".into(),
                file_name: "f.bin".into(),
                file_size: 10,
                transport: TransportMode::Direct,
                sas: "000000".into(),
            }));
        }
        state.apply(Event::IncomingExpired {
            transfer_id: "in0".into(),
        });
        assert_eq!(state.incoming.len(), 1);
        assert!(state.incoming.contains_key("in1"));
    }

    #[test]
    fn same_name_two_devices() {
        let mut state = UiState::new(10);
        state.apply(Event::DevicesChanged(vec![
            peer("dev-1", "MacBook Pro", 7777),
            peer("dev-2", "MacBook Pro", 7778),
        ]));
        state.selected = Some(PeerKey::Direct("dev-2".into()));
        state.apply(Event::DevicesChanged(vec![peer(
            "dev-2",
            "MacBook Pro",
            7799,
        )]));
        assert!(state.selected.is_some());
        assert_eq!(state.devices.len(), 1);

        // A relay peer is keyed by its peer_id.
        state.apply(Event::DevicesChanged(vec![relay_peer(
            "peer-1",
            "MacBook Pro",
        )]));
        state.selected = Some(PeerKey::Relay("peer-1".into()));
        state.apply(Event::DevicesChanged(vec![relay_peer("peer-1", "Renamed")]));
        assert!(state.selected.is_some());
    }

    #[test]
    fn toast_expires() {
        let mut state = UiState::new(10);
        state.toast = Some(Toast {
            text: "x".into(),
            kind: ToastKind::Error,
            at: Instant::now() - std::time::Duration::from_secs(5),
        });
        state.tick();
        assert!(state.toast.is_none());
    }

    #[test]
    fn failed_event_is_not_listed_but_toasts() {
        let mut state = UiState::new(10);
        state.apply(queued("t1", "a.bin"));
        state.apply(Event::Failed {
            transfer_id: "t1".into(),
            direction: Direction::Send,
            file_name: "a.bin".into(),
            reason: "connexion refusée".into(),
            transport: TransportMode::Direct,
        });
        assert!(state.queue.is_empty());
        assert!(state.history.is_empty());
        assert_eq!(
            state.toast.as_ref().map(|toast| toast.text.as_str()),
            Some("connexion refusée")
        );
    }

    #[test]
    fn unknown_event_ignored() {
        let mut state = UiState::new(10);
        state.apply(progress("ghost", 10, 10));
        state.apply(Event::Completed {
            transfer_id: "ghost".into(),
            direction: Direction::Send,
            file_name: "ghost.bin".into(),
            path: None,
            transport: TransportMode::Direct,
        });
        assert!(state.queue.is_empty());
        assert_eq!(state.history.len(), 1);
    }

    #[test]
    fn selection_toggles_off_on_second_click() {
        let mut state = UiState::new(10);
        let key = PeerKey::Direct("dev-1".into());
        state.toggle_selected(key.clone());
        assert_eq!(state.selected, Some(key.clone()));
        state.toggle_selected(key);
        assert!(state.selected.is_none());
    }

    #[test]
    fn selection_cleared_when_device_disappears() {
        let mut state = UiState::new(10);
        state.apply(Event::DevicesChanged(vec![peer("dev-1", "Mac", 7777)]));
        state.selected = Some(PeerKey::Direct("dev-1".into()));
        state.apply(Event::DevicesChanged(Vec::new()));
        assert!(state.selected.is_none());
    }
}

#[cfg(test)]
mod proptest_tests {
    use super::*;
    use droplink_core::protocol::TransportMode;
    use proptest::prelude::*;

    fn event_strategy() -> impl Strategy<Value = Event> {
        let ids = proptest::sample::select(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        prop_oneof![
            ids.clone().prop_map(|id| Event::Queued {
                transfer_id: id.clone(),
                file_name: format!("{id}.bin"),
                direction: Direction::Send,
            }),
            (ids.clone(), 0u64..1_000_000u64).prop_map(|(id, bytes)| Event::Progress {
                transfer_id: id,
                file_name: "f".into(),
                bytes,
                total: 1_000_000,
                direction: Direction::Send,
                transport: TransportMode::Direct,
            }),
            ids.clone().prop_map(|id| Event::Completed {
                transfer_id: id,
                direction: Direction::Send,
                file_name: "f".into(),
                path: None,
                transport: TransportMode::Direct,
            }),
            ids.prop_map(|id| Event::Failed {
                transfer_id: id,
                direction: Direction::Send,
                file_name: "f".into(),
                reason: "boom".into(),
                transport: TransportMode::Direct,
            }),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 2_000, ..ProptestConfig::default() })]

        #[test]
        fn arbitrary_event_sequences_stay_consistent(events in proptest::collection::vec(event_strategy(), 0..200)) {
            let mut state = UiState::new(16);
            for event in events {
                state.apply(event);
            }
            let mut seen = std::collections::HashSet::new();
            for row in &state.queue {
                prop_assert!(seen.insert(row.transfer_id.clone()), "duplicate transfer id in queue");
            }
            prop_assert!(state.queue.len() <= 3);
            prop_assert!(state.history.len() <= 16);
        }
    }
}
