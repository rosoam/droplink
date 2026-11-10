use droplink_core::protocol::PeerId;
use droplink_core::relay::{decode_binary, encode_binary, ClientMsg, PeerSummary, ServerMsg};
use droplink_core::{Error, Result};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::sync::PollSender;
use url::Url;

/// Client-to-relay queue depth. `poll_write` applies backpressure when full.
const OUT_QUEUE: usize = 8;
/// Incoming circuit chunks buffered before the reader is slowed down.
const CIRCUIT_QUEUE: usize = 16;

#[derive(Debug, Clone)]
pub enum WsOut {
    Text(ClientMsg),
    Binary(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayEvent {
    Created {
        code: String,
        token: String,
        expires_at: u64,
        ttl_secs: u64,
    },
    Joined {
        expires_at: u64,
        ttl_secs: u64,
        peers: Vec<PeerSummary>,
        token: String,
    },
    Resumed {
        code: String,
        expires_at: u64,
        ttl_secs: u64,
        peers: Vec<PeerSummary>,
    },
    Refreshed {
        code: String,
        expires_at: u64,
        ttl_secs: u64,
    },
    SessionEnded {
        reason: String,
    },
    Peers(Vec<PeerSummary>),
    Incoming {
        circuit_id: String,
    },
    Error {
        code: String,
        message: String,
    },
    Disconnected,
}

pub(crate) struct RelayInner {
    out: mpsc::Sender<WsOut>,
    circuits: Mutex<HashMap<String, mpsc::Sender<Vec<u8>>>>,
    receivers: Mutex<HashMap<String, mpsc::Receiver<Vec<u8>>>>,
    pending: Mutex<HashMap<PeerId, oneshot::Sender<String>>>,
    events: mpsc::Sender<RelayEvent>,
}

pub struct RelayClient {
    inner: Arc<RelayInner>,
    writer: tokio::task::JoinHandle<()>,
    reader: tokio::task::JoinHandle<()>,
}

impl RelayClient {
    /// Open the socket. Nothing is registered until a session message is sent.
    pub async fn connect(url: &str, events: mpsc::Sender<RelayEvent>) -> Result<Self> {
        let url = Url::parse(url).map_err(|e| Error::Protocol(e.to_string()))?;
        let (socket, _) = connect_async(url.as_str())
            .await
            .map_err(|e| Error::Protocol(format!("relay unreachable: {e}")))?;
        let (mut sink, mut stream) = socket.split();
        let (out_tx, mut out_rx) = mpsc::channel::<WsOut>(OUT_QUEUE);

        let inner = Arc::new(RelayInner {
            out: out_tx.clone(),
            circuits: Mutex::new(HashMap::new()),
            receivers: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            events,
        });

        let writer = tokio::spawn(async move {
            while let Some(out) = out_rx.recv().await {
                let message = match out {
                    WsOut::Text(control) => match serde_json::to_string(&control) {
                        Ok(json) => Message::Text(json.into()),
                        Err(_) => continue,
                    },
                    WsOut::Binary(bytes) => Message::Binary(bytes.into()),
                };
                if sink.send(message).await.is_err() {
                    break;
                }
            }
        });

        let reader_inner = inner.clone();
        let reader = tokio::spawn(async move {
            while let Some(message) = stream.next().await {
                match message {
                    Ok(Message::Text(text)) => {
                        let Ok(server) = serde_json::from_str::<ServerMsg>(text.as_str()) else {
                            // An unknown message means a protocol drift: say it instead of
                            // waiting forever for a reply that will never come.
                            let _ = reader_inner.events.try_send(RelayEvent::Error {
                                code: "bad_message".into(),
                                message: "unrecognised server message".into(),
                            });
                            continue;
                        };
                        match server {
                            ServerMsg::SessionCreated {
                                code,
                                token,
                                expires_at,
                                ttl_secs,
                            } => {
                                let _ = reader_inner.events.try_send(RelayEvent::Created {
                                    code,
                                    token,
                                    expires_at,
                                    ttl_secs,
                                });
                            }
                            ServerMsg::SessionJoined {
                                expires_at,
                                ttl_secs,
                                peers,
                                token,
                            } => {
                                let _ = reader_inner.events.try_send(RelayEvent::Joined {
                                    expires_at,
                                    ttl_secs,
                                    peers,
                                    token,
                                });
                            }
                            ServerMsg::SessionResumed {
                                code,
                                expires_at,
                                ttl_secs,
                                peers,
                            } => {
                                let _ = reader_inner.events.try_send(RelayEvent::Resumed {
                                    code,
                                    expires_at,
                                    ttl_secs,
                                    peers,
                                });
                            }
                            ServerMsg::SessionRefreshed {
                                code,
                                expires_at,
                                ttl_secs,
                            } => {
                                let _ = reader_inner.events.try_send(RelayEvent::Refreshed {
                                    code,
                                    expires_at,
                                    ttl_secs,
                                });
                            }
                            ServerMsg::SessionEnded { reason } => {
                                let _ = reader_inner
                                    .events
                                    .try_send(RelayEvent::SessionEnded { reason });
                            }
                            ServerMsg::RoomUpdate { peers } => {
                                let _ = reader_inner.events.try_send(RelayEvent::Peers(peers));
                            }
                            ServerMsg::CircuitOpened {
                                circuit_id,
                                peer,
                                initiator,
                            } => {
                                if initiator {
                                    let waiter = reader_inner
                                        .pending
                                        .lock()
                                        .ok()
                                        .and_then(|mut map| map.remove(&peer.peer_id));
                                    if let Some(tx) = waiter {
                                        let _ = tx.send(circuit_id);
                                    }
                                } else {
                                    let (tx, rx) = mpsc::channel(CIRCUIT_QUEUE);
                                    if let Ok(mut map) = reader_inner.circuits.lock() {
                                        map.insert(circuit_id.clone(), tx);
                                    }
                                    if let Ok(mut map) = reader_inner.receivers.lock() {
                                        map.insert(circuit_id.clone(), rx);
                                    }
                                    let _ = reader_inner
                                        .events
                                        .try_send(RelayEvent::Incoming { circuit_id });
                                }
                            }
                            ServerMsg::CircuitClosed { circuit_id, .. } => {
                                if let Ok(mut map) = reader_inner.circuits.lock() {
                                    map.remove(&circuit_id);
                                }
                            }
                            ServerMsg::Error { code, message } => {
                                tracing::warn!(%code, %message, "relay error");
                                let _ = reader_inner
                                    .events
                                    .try_send(RelayEvent::Error { code, message });
                            }
                        }
                    }
                    Ok(Message::Binary(bytes)) => {
                        if let Ok((circuit_id, payload)) = decode_binary(&bytes) {
                            let sender = reader_inner
                                .circuits
                                .lock()
                                .ok()
                                .and_then(|map| map.get(&circuit_id).cloned());
                            if let Some(sender) = sender {
                                // Awaiting here is the whole point: a full circuit queue must
                                // slow the WebSocket reader (and therefore the sender), never
                                // drop a frame. A dropped frame corrupts the AEAD stream.
                                match tokio::time::timeout(
                                    Duration::from_secs(30),
                                    sender.send(payload.to_vec()),
                                )
                                .await
                                {
                                    Ok(Ok(())) => {}
                                    _ => {
                                        let _ = reader_inner.events.try_send(RelayEvent::Error {
                                            code: "backpressure".into(),
                                            message: "receiver stalled for 30 s".into(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                    Ok(Message::Close(_)) | Err(_) => break,
                    _ => {}
                }
            }
            if let Ok(mut map) = reader_inner.circuits.lock() {
                map.clear();
            }
            if let Ok(mut map) = reader_inner.receivers.lock() {
                map.clear();
            }
            let _ = reader_inner.events.try_send(RelayEvent::Disconnected);
        });

        Ok(Self {
            inner,
            writer,
            reader,
        })
    }

    async fn send(&self, message: ClientMsg) -> Result<()> {
        self.inner
            .out
            .send(WsOut::Text(message))
            .await
            .map_err(|_| Error::Protocol("relay writer closed".into()))
    }

    pub async fn create_session(&self, device_id: &str, alias: &str) -> Result<()> {
        self.send(ClientMsg::CreateSession {
            proto: droplink_core::protocol::PROTOCOL_VERSION,
            device_id: device_id.to_string(),
            alias: alias.to_string(),
        })
        .await
    }

    pub async fn join_session(&self, device_id: &str, alias: &str, code: &str) -> Result<()> {
        self.send(ClientMsg::JoinSession {
            proto: droplink_core::protocol::PROTOCOL_VERSION,
            device_id: device_id.to_string(),
            alias: alias.to_string(),
            code: code.to_string(),
        })
        .await
    }

    pub async fn resume_session(&self, token: &str) -> Result<()> {
        self.send(ClientMsg::ResumeSession {
            token: token.to_string(),
        })
        .await
    }

    pub async fn refresh_session(&self, token: &str) -> Result<()> {
        self.send(ClientMsg::RefreshSession {
            token: token.to_string(),
        })
        .await
    }

    pub async fn leave_session(&self) -> Result<()> {
        self.send(ClientMsg::LeaveSession).await
    }

    /// Open an outgoing circuit to `peer_id` and wait for the relay to pair both sides.
    pub async fn open_circuit(&self, peer_id: &str) -> Result<RelayStream> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut pending) = self.inner.pending.lock() {
            pending.insert(peer_id.to_string(), tx);
        }
        self.send(ClientMsg::Connect {
            peer_id: peer_id.to_string(),
        })
        .await?;

        let circuit_id = match tokio::time::timeout(Duration::from_secs(10), rx).await {
            Ok(Ok(circuit_id)) => circuit_id,
            _ => {
                if let Ok(mut pending) = self.inner.pending.lock() {
                    pending.remove(peer_id);
                }
                return Err(Error::Timeout);
            }
        };

        let (data_tx, data_rx) = mpsc::channel(CIRCUIT_QUEUE);
        if let Ok(mut map) = self.inner.circuits.lock() {
            map.insert(circuit_id.clone(), data_tx);
        }
        let out = self.inner.out.clone();
        Ok(RelayStream {
            circuit_id,
            rx: data_rx,
            out: out.clone(),
            out_poll: PollSender::new(out),
            inner: self.inner.clone(),
            current: Vec::new(),
            offset: 0,
        })
    }

    /// Take the receive side of an incoming circuit.
    pub fn take_circuit(&self, circuit_id: &str) -> Option<RelayStream> {
        let rx = self
            .inner
            .receivers
            .lock()
            .ok()
            .and_then(|mut map| map.remove(circuit_id))?;
        let out = self.inner.out.clone();
        Some(RelayStream {
            circuit_id: circuit_id.to_string(),
            rx,
            out: out.clone(),
            out_poll: PollSender::new(out),
            inner: self.inner.clone(),
            current: Vec::new(),
            offset: 0,
        })
    }
}

impl Drop for RelayClient {
    fn drop(&mut self) {
        self.writer.abort();
        self.reader.abort();
    }
}

/// A bidirectional byte stream backed by a relay circuit.
pub struct RelayStream {
    circuit_id: String,
    rx: mpsc::Receiver<Vec<u8>>,
    out: mpsc::Sender<WsOut>,
    out_poll: PollSender<WsOut>,
    inner: Arc<RelayInner>,
    current: Vec<u8>,
    offset: usize,
}

impl AsyncRead for RelayStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            if self.offset >= self.current.len() {
                match self.rx.poll_recv(cx) {
                    Poll::Ready(Some(chunk)) => {
                        self.current = chunk;
                        self.offset = 0;
                        if self.current.is_empty() {
                            continue;
                        }
                    }
                    Poll::Ready(None) => return Poll::Ready(Ok(())),
                    Poll::Pending => return Poll::Pending,
                }
            }
            let available = &self.current[self.offset..];
            let n = available.len().min(buf.remaining());
            buf.put_slice(&available[..n]);
            self.offset += n;
            return Poll::Ready(Ok(()));
        }
    }
}

impl AsyncWrite for RelayStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let frame = encode_binary(&self.circuit_id, data).map_err(io::Error::other)?;
        match self.out_poll.poll_reserve(cx) {
            Poll::Ready(Ok(())) => {
                let _ = self.out_poll.send_item(WsOut::Binary(frame));
                Poll::Ready(Ok(data.len()))
            }
            Poll::Ready(Err(_)) => Poll::Ready(Err(io::ErrorKind::BrokenPipe.into())),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let _ = self.out.try_send(WsOut::Text(ClientMsg::Close {
            circuit_id: self.circuit_id.clone(),
        }));
        Poll::Ready(Ok(()))
    }
}

impl Drop for RelayStream {
    fn drop(&mut self) {
        if let Ok(mut map) = self.inner.circuits.lock() {
            map.remove(&self.circuit_id);
        }
        let _ = self.out.try_send(WsOut::Text(ClientMsg::Close {
            circuit_id: self.circuit_id.clone(),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unreachable_relay_returns_error() {
        let (tx, _rx) = mpsc::channel(1);
        let result = RelayClient::connect("ws://127.0.0.1:1/ws", tx).await;
        assert!(result.is_err());
    }
}
