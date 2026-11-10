use crate::relay_client::{RelayClient, RelayEvent, RelayStream};
use crate::stream::FramedStream;
use crate::transport;
use crate::{receiver, sender};
use droplink_core::config::DEFAULT_RELAY_URL;
use droplink_core::peer::{Peer, PeerKey, Reach, Source};
use droplink_core::protocol::{Direction, TransferId, TransportMode};
use droplink_core::registry::DeviceRegistry;
use droplink_core::{Error, Result};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio_util::sync::CancellationToken;

pub const EVENT_CHANNEL: usize = 512;
pub const COMMAND_CHANNEL: usize = 128;

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub device_id: String,
    pub device_name: String,
    pub receive_dir: PathBuf,
    pub port_pref: Option<u16>,
    pub max_parallel: usize,
    pub accept_timeout: Duration,
    pub handshake_timeout: Duration,
    pub progress_interval: Duration,
    pub relay_enabled: bool,
    pub relay_url: Option<String>,
    pub room_code: Option<String>,
}

impl EngineConfig {
    pub fn new(device_id: String, device_name: String, receive_dir: PathBuf) -> Self {
        Self {
            device_id,
            device_name,
            receive_dir,
            port_pref: None,
            max_parallel: 4,
            accept_timeout: Duration::from_secs(60),
            handshake_timeout: Duration::from_secs(5),
            progress_interval: Duration::from_millis(100),
            relay_enabled: false,
            relay_url: Some(DEFAULT_RELAY_URL.to_string()),
            room_code: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bound {
    pub port: u16,
}

#[derive(Clone, Debug)]
pub enum Command {
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
    PeerSeen {
        peer: Peer,
        source: Source,
    },
    PeerGone {
        key: PeerKey,
        source: Source,
    },
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingTransfer {
    pub transfer_id: TransferId,
    pub sender_alias: String,
    pub file_name: String,
    pub file_size: u64,
    pub transport: TransportMode,
    pub sas: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    DevicesChanged(Vec<Peer>),
    Queued {
        transfer_id: TransferId,
        file_name: String,
        direction: Direction,
    },
    Incoming(IncomingTransfer),
    IncomingExpired {
        transfer_id: TransferId,
    },
    Sas {
        transfer_id: TransferId,
        code: String,
    },
    Progress {
        transfer_id: TransferId,
        file_name: String,
        bytes: u64,
        total: u64,
        direction: Direction,
        transport: TransportMode,
    },
    Completed {
        transfer_id: TransferId,
        direction: Direction,
        file_name: String,
        path: Option<PathBuf>,
        transport: TransportMode,
    },
    Failed {
        transfer_id: TransferId,
        direction: Direction,
        file_name: String,
        reason: String,
        transport: TransportMode,
    },
}

#[derive(Clone, Debug)]
pub struct ProgressSnapshot {
    pub transfer_id: TransferId,
    pub file_name: String,
    pub bytes: u64,
    pub total: u64,
    pub direction: Direction,
    pub transport: TransportMode,
}

#[derive(Clone, Default)]
pub(crate) struct ProgressRegistry {
    inner: Arc<Mutex<HashMap<TransferId, ProgressSnapshot>>>,
}

impl ProgressRegistry {
    fn set(&self, snapshot: ProgressSnapshot) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(snapshot.transfer_id.clone(), snapshot);
        }
    }

    fn remove(&self, transfer_id: &str) {
        if let Ok(mut map) = self.inner.lock() {
            map.remove(transfer_id);
        }
    }

    fn snapshot(&self) -> Vec<ProgressSnapshot> {
        self.inner
            .lock()
            .map(|map| map.values().cloned().collect())
            .unwrap_or_default()
    }
}

pub(crate) type PendingMap = Arc<tokio::sync::Mutex<HashMap<TransferId, oneshot::Sender<bool>>>>;

pub(crate) struct EngineState {
    pub cfg: EngineConfig,
    pub events: mpsc::Sender<Event>,
    pub progress: ProgressRegistry,
    pub pending: PendingMap,
    registry: Mutex<DeviceRegistry>,
    cancels: Mutex<HashMap<TransferId, CancellationToken>>,
    per_peer: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    per_ip: Mutex<HashMap<IpAddr, usize>>,
    semaphore: Arc<Semaphore>,
    shutdown: CancellationToken,
    relay: Mutex<Option<Arc<RelayClient>>>,
    relay_peers: Mutex<HashSet<String>>,
}

impl EngineState {
    pub(crate) fn emit(&self, event: Event) {
        if let Err(err) = self.events.try_send(event) {
            tracing::debug!(error = %err, "event channel full, event dropped");
        }
    }

    pub(crate) fn progress_set(&self, snapshot: ProgressSnapshot) {
        self.progress.set(snapshot);
    }

    pub(crate) fn progress_remove(&self, transfer_id: &str) {
        self.progress.remove(transfer_id);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.shutdown.is_cancelled()
    }

    fn emit_devices(&self) {
        let peers = self
            .registry
            .lock()
            .map(|r| r.snapshot())
            .unwrap_or_default();
        self.emit(Event::DevicesChanged(peers));
    }

    fn peer_lock(&self, peer: &Peer) -> Arc<tokio::sync::Mutex<()>> {
        let key = peer
            .device_id
            .clone()
            .or_else(|| {
                peer.reach
                    .direct_addr()
                    .map(|(ip, port)| format!("{ip}:{port}"))
            })
            .unwrap_or_else(|| peer.alias.clone());
        let mut map = match self.per_peer.lock() {
            Ok(map) => map,
            Err(poisoned) => poisoned.into_inner(),
        };
        map.entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    fn ip_acquire(&self, ip: IpAddr) -> bool {
        let mut map = match self.per_ip.lock() {
            Ok(map) => map,
            Err(poisoned) => poisoned.into_inner(),
        };
        let count = map.entry(ip).or_insert(0);
        if *count >= 2 {
            return false;
        }
        *count += 1;
        true
    }

    pub(crate) fn ip_release(&self, ip: IpAddr) {
        let mut map = match self.per_ip.lock() {
            Ok(map) => map,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(count) = map.get_mut(&ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(&ip);
            }
        }
    }

    pub(crate) fn register_cancel(&self, transfer_id: TransferId, token: CancellationToken) {
        if let Ok(mut map) = self.cancels.lock() {
            map.insert(transfer_id, token);
        }
    }

    pub(crate) fn finish_cancel(&self, transfer_id: &str) {
        if let Ok(mut map) = self.cancels.lock() {
            map.remove(transfer_id);
        }
    }

    fn cancel(&self, transfer_id: &str) {
        let token = self
            .cancels
            .lock()
            .ok()
            .and_then(|map| map.get(transfer_id).cloned());
        if let Some(token) = token {
            token.cancel();
        }
    }

    pub(crate) fn peer_seen(&self, peer: Peer, source: Source) {
        let changed = self
            .registry
            .lock()
            .map(|mut registry| registry.upsert(peer, source).is_some())
            .unwrap_or(false);
        if changed {
            self.emit_devices();
        }
    }

    pub(crate) fn peer_gone(&self, key: PeerKey, source: Source) {
        let changed = self
            .registry
            .lock()
            .map(|mut registry| registry.remove_source(&key, source).is_some())
            .unwrap_or(false);
        if changed {
            self.emit_devices();
        }
    }

    fn set_relay(&self, client: Option<Arc<RelayClient>>) {
        if let Ok(mut slot) = self.relay.lock() {
            *slot = client;
        }
    }

    fn relay(&self) -> Option<Arc<RelayClient>> {
        self.relay.lock().ok().and_then(|slot| slot.clone())
    }

    fn replace_relay_peers(&self, ids: HashSet<String>) -> HashSet<String> {
        match self.relay_peers.lock() {
            Ok(mut current) => std::mem::replace(&mut *current, ids),
            Err(_) => HashSet::new(),
        }
    }

    fn clear_relay_peers(&self) {
        if let Ok(mut ids) = self.relay_peers.lock() {
            ids.clear();
        }
    }

    fn relay_peer_ids(&self) -> HashSet<String> {
        self.relay_peers
            .lock()
            .map(|ids| ids.clone())
            .unwrap_or_default()
    }
}

#[derive(Clone)]
pub struct EngineHandle {
    commands: mpsc::Sender<Command>,
    port: u16,
}

impl EngineHandle {
    pub fn command(&self, command: Command) {
        if let Err(err) = self.commands.try_send(command) {
            tracing::warn!(error = %err, "command channel full or closed");
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Clone of the command channel, for platform bridges (iOS Bonjour callbacks).
    pub fn command_sender(&self) -> mpsc::Sender<Command> {
        self.commands.clone()
    }
}

/// Start the transfer engine. Must be called from within a Tokio runtime.
pub fn spawn(cfg: EngineConfig) -> Result<(EngineHandle, Bound, mpsc::Receiver<Event>)> {
    if cfg.max_parallel == 0 || cfg.max_parallel > 32 {
        return Err(Error::Protocol("max_parallel out of range".into()));
    }
    let std_listener = std::net::TcpListener::bind(("0.0.0.0", cfg.port_pref.unwrap_or(0)))?;
    std_listener.set_nonblocking(true)?;
    let listener = TcpListener::from_std(std_listener)?;
    let port = listener.local_addr()?.port();

    let (event_tx, event_rx) = mpsc::channel(EVENT_CHANNEL);
    let (command_tx, command_rx) = mpsc::channel(COMMAND_CHANNEL);

    let state = Arc::new(EngineState {
        semaphore: Arc::new(Semaphore::new(cfg.max_parallel)),
        cfg,
        events: event_tx,
        progress: ProgressRegistry::default(),
        pending: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        registry: Mutex::new(DeviceRegistry::new()),
        cancels: Mutex::new(HashMap::new()),
        per_peer: Mutex::new(HashMap::new()),
        per_ip: Mutex::new(HashMap::new()),
        shutdown: CancellationToken::new(),
        relay: Mutex::new(None),
        relay_peers: Mutex::new(HashSet::new()),
    });

    if state.cfg.relay_enabled {
        start_relay(state.clone());
    }

    let _task = tokio::spawn(accept_loop(state.clone(), listener));
    let _task = tokio::spawn(command_loop(state.clone(), command_rx));
    let _task = tokio::spawn(progress_loop(state));

    Ok((
        EngineHandle {
            commands: command_tx,
            port,
        },
        Bound { port },
        event_rx,
    ))
}

async fn progress_loop(state: Arc<EngineState>) {
    let interval = state.cfg.progress_interval;
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = state.shutdown.cancelled() => break,
            _ = ticker.tick() => {
                for snapshot in state.progress.snapshot() {
                    state.emit(Event::Progress {
                        transfer_id: snapshot.transfer_id,
                        file_name: snapshot.file_name,
                        bytes: snapshot.bytes,
                        total: snapshot.total,
                        direction: snapshot.direction,
                        transport: snapshot.transport,
                    });
                }
            }
        }
    }
}

async fn accept_loop(state: Arc<EngineState>, listener: TcpListener) {
    loop {
        let accepted = tokio::select! {
            _ = state.shutdown.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        let (stream, addr) = match accepted {
            Ok(pair) => pair,
            Err(err) => {
                tracing::warn!(error = %err, "accept failed");
                continue;
            }
        };
        if !state.ip_acquire(addr.ip()) {
            tracing::debug!(%addr, "per-ip connection cap reached");
            let _ = crate::receiver::send_busy(stream).await;
            continue;
        }
        let permit = match state.semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                state.ip_release(addr.ip());
                let _ = crate::receiver::send_busy(stream).await;
                continue;
            }
        };
        let task_state = state.clone();
        let _task = tokio::spawn(async move {
            let _permit = permit;
            if let Err(err) = receiver::handle(task_state.clone(), stream, addr).await {
                tracing::debug!(error = %err, "incoming transfer ended");
            }
            task_state.ip_release(addr.ip());
        });
    }
}

async fn command_loop(state: Arc<EngineState>, mut commands: mpsc::Receiver<Command>) {
    while let Some(command) = commands.recv().await {
        match command {
            Command::Send { peer, path } => {
                let task_state = state.clone();
                let _task = tokio::spawn(async move {
                    send_task(task_state, peer, path).await;
                });
            }
            Command::Respond {
                transfer_id,
                accept,
            } => {
                let sender = {
                    let mut pending = state.pending.lock().await;
                    pending.remove(&transfer_id)
                };
                if let Some(sender) = sender {
                    let _ = sender.send(accept);
                }
            }
            Command::Cancel { transfer_id } => state.cancel(&transfer_id),
            Command::PeerSeen { peer, source } => state.peer_seen(peer, source),
            Command::PeerGone { key, source } => state.peer_gone(key, source),
            Command::Shutdown => {
                state.shutdown.cancel();
                break;
            }
        }
    }
    state.shutdown.cancel();
}

async fn send_task(state: Arc<EngineState>, peer: Peer, path: PathBuf) {
    let transfer_id = uuid::Uuid::new_v4().to_string();
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "fichier".to_string());

    state.emit(Event::Queued {
        transfer_id: transfer_id.clone(),
        file_name: file_name.clone(),
        direction: Direction::Send,
    });

    let token = CancellationToken::new();
    state.register_cancel(transfer_id.clone(), token.clone());

    let peer_lock = state.peer_lock(&peer);
    let _guard = peer_lock.lock().await;

    let selection = pick_transport(&state, &peer).await;
    let (result, mode) = match selection {
        Ok(TransportChoice::Direct(stream)) => (
            sender::send_file(
                state.clone(),
                stream,
                &path,
                &transfer_id,
                TransportMode::Direct,
                token,
            )
            .await,
            TransportMode::Direct,
        ),
        Ok(TransportChoice::Relay(stream)) => (
            sender::send_file(
                state.clone(),
                stream,
                &path,
                &transfer_id,
                TransportMode::Relay,
                token,
            )
            .await,
            TransportMode::Relay,
        ),
        Err(err) => (Err(err), TransportMode::Direct),
    };
    state.progress_remove(&transfer_id);
    state.finish_cancel(&transfer_id);

    match result {
        Ok(()) => state.emit(Event::Completed {
            transfer_id,
            direction: Direction::Send,
            file_name,
            path: None,
            transport: mode,
        }),
        Err(err) => state.emit(Event::Failed {
            transfer_id,
            direction: Direction::Send,
            file_name,
            reason: err.to_string(),
            transport: mode,
        }),
    }
}

enum TransportChoice {
    Direct(tokio::net::TcpStream),
    Relay(RelayStream),
}

async fn pick_transport(state: &EngineState, peer: &Peer) -> Result<TransportChoice> {
    if let Some((ip, port)) = peer.reach.direct_addr() {
        if let Ok(stream) = transport::direct::connect(ip, port, Duration::from_secs(2)).await {
            return Ok(TransportChoice::Direct(stream));
        }
    }
    if let Some(peer_id) = peer.reach.relay_id() {
        if let Some(client) = state.relay() {
            return Ok(TransportChoice::Relay(client.open_circuit(peer_id).await?));
        }
    }
    Err(Error::Protocol("peer unreachable".into()))
}

fn start_relay(state: Arc<EngineState>) {
    let Some(url) = state.cfg.relay_url.clone() else {
        return;
    };
    let device_id = state.cfg.device_id.clone();
    let alias = state.cfg.device_name.clone();
    let room = state.cfg.room_code.clone();

    let _task = tokio::spawn(async move {
        let mut backoff = Duration::from_secs(1);
        loop {
            if state.is_cancelled() {
                break;
            }
            let (events_tx, mut events_rx) = mpsc::channel(64);
            match RelayClient::connect(&url, &device_id, &alias, room.as_deref(), events_tx).await {
                Ok(client) => {
                    backoff = Duration::from_secs(1);
                    let client = Arc::new(client);
                    state.set_relay(Some(client.clone()));
                    while let Some(event) = events_rx.recv().await {
                        match event {
                            RelayEvent::Peers(peers) => {
                                let new_ids: HashSet<String> =
                                    peers.iter().map(|peer| peer.peer_id.clone()).collect();
                                let old_ids = state.replace_relay_peers(new_ids.clone());
                                for peer in peers {
                                    state.peer_seen(
                                        Peer {
                                            device_id: None,
                                            alias: peer.alias,
                                            reach: Reach::Relay {
                                                peer_id: peer.peer_id,
                                            },
                                        },
                                        Source::Relay,
                                    );
                                }
                                for id in old_ids.difference(&new_ids) {
                                    state.peer_gone(PeerKey::Relay(id.clone()), Source::Relay);
                                }
                            }
                            RelayEvent::Incoming { circuit_id } => {
                                if let Some(stream) = client.take_circuit(&circuit_id) {
                                    let handle = state.clone();
                                    let _t = tokio::spawn(async move {
                                        if let Err(err) = receiver::handle_stream(
                                            handle,
                                            stream,
                                            TransportMode::Relay,
                                        )
                                        .await
                                        {
                                            tracing::debug!(error = %err, "relay incoming ended");
                                        }
                                    });
                                }
                            }
                            RelayEvent::Disconnected => {
                                state.set_relay(None);
                                let ids = state.relay_peer_ids();
                                state.clear_relay_peers();
                                for id in ids {
                                    state.peer_gone(PeerKey::Relay(id), Source::Relay);
                                }
                                break;
                            }
                        }
                    }
                    state.set_relay(None);
                }
                Err(err) => tracing::warn!(error = %err, "relay unavailable, will retry"),
            }
            if state.is_cancelled() {
                break;
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    });
}

/// Send a plaintext error and close the connection.
pub(crate) async fn send_plain_error(stream: tokio::net::TcpStream, reason: &str) -> Result<()> {
    let mut framed = FramedStream::new(stream);
    framed
        .write_control(&droplink_core::protocol::Control::Error {
            transfer_id: None,
            reason: reason.to_string(),
        })
        .await
}
