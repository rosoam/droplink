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

#[derive(Debug, Clone)]
pub enum WsOut {
    Text(ClientMsg),
    Binary(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayEvent {
    Peers(Vec<PeerSummary>),
    Incoming { circuit_id: String },
    Disconnected,
}

pub(crate) struct RelayInner {
    out: mpsc::Sender<WsOut>,
    circuits: Mutex<HashMap<String, mpsc::Sender<Vec<u8>>>>,
    receivers: Mutex<HashMap<String, mpsc::Receiver<Vec<u8>>>>,
    pending: Mutex<HashMap<PeerId, oneshot::Sender<String>>>,
    events: mpsc::Sender<RelayEvent>,
    peer_id: Mutex<Option<PeerId>>,
}

pub struct RelayClient {
    inner: Arc<RelayInner>,
    writer: tokio::task::JoinHandle<()>,
    reader: tokio::task::JoinHandle<()>,
}

impl RelayClient {
    pub async fn connect(
        url: &str,
        device_id: &str,
        alias: &str,
        room: Option<&str>,
        events: mpsc::Sender<RelayEvent>,
    ) -> Result<Self> {
        let url = Url::parse(url).map_err(|e| Error::Protocol(e.to_string()))?;
        let (socket, _) = connect_async(url.as_str())
            .await
            .map_err(|e| Error::Protocol(format!("relay unreachable: {e}")))?;
        let (mut sink, mut stream) = socket.split();
        let (out_tx, mut out_rx) = mpsc::channel::<WsOut>(256);

        let inner = Arc::new(RelayInner {
            out: out_tx.clone(),
            circuits: Mutex::new(HashMap::new()),
            receivers: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            events,
            peer_id: Mutex::new(None),
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

        let (reg_tx, reg_rx) = oneshot::channel();
        let reader_inner = inner.clone();
        let reader = tokio::spawn(async move {
            let mut reg_tx = Some(reg_tx);
            while let Some(message) = stream.next().await {
                match message {
                    Ok(Message::Text(text)) => {
                        let Ok(server) = serde_json::from_str::<ServerMsg>(text.as_str()) else {
                            continue;
                        };
                        match server {
                            ServerMsg::Registered { peer_id } => {
                                if let Ok(mut slot) = reader_inner.peer_id.lock() {
                                    *slot = Some(peer_id.clone());
                                }
                                if let Some(tx) = reg_tx.take() {
                                    let _ = tx.send(peer_id);
                                }
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
                                    let (tx, rx) = mpsc::channel(64);
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
                                let _ = sender.try_send(payload.to_vec());
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

        out_tx
            .send(WsOut::Text(ClientMsg::Register {
                device_id: device_id.to_string(),
                alias: alias.to_string(),
                room: room.map(str::to_string),
            }))
            .await
            .map_err(|_| Error::Protocol("relay writer closed".into()))?;

        let registration = tokio::time::timeout(Duration::from_secs(5), reg_rx).await;
        match registration {
            Ok(Ok(peer_id)) => {
                if let Ok(mut slot) = inner.peer_id.lock() {
                    *slot = Some(peer_id);
                }
                Ok(Self {
                    inner,
                    writer,
                    reader,
                })
            }
            _ => {
                writer.abort();
                reader.abort();
                Err(Error::Timeout)
            }
        }
    }

    pub fn peer_id(&self) -> Option<PeerId> {
        self.inner.peer_id.lock().ok().and_then(|slot| slot.clone())
    }

    /// Open an outgoing circuit to `peer_id` and wait for the relay to pair both sides.
    pub async fn open_circuit(&self, peer_id: &str) -> Result<RelayStream> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut pending) = self.inner.pending.lock() {
            pending.insert(peer_id.to_string(), tx);
        }
        self.inner
            .out
            .send(WsOut::Text(ClientMsg::Connect {
                peer_id: peer_id.to_string(),
            }))
            .await
            .map_err(|_| Error::Protocol("relay writer closed".into()))?;

        let circuit_id = match tokio::time::timeout(Duration::from_secs(10), rx).await {
            Ok(Ok(circuit_id)) => circuit_id,
            _ => {
                if let Ok(mut pending) = self.inner.pending.lock() {
                    pending.remove(peer_id);
                }
                return Err(Error::Timeout);
            }
        };

        let (data_tx, data_rx) = mpsc::channel(64);
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
        let result = RelayClient::connect(
            "ws://127.0.0.1:1/ws",
            "11111111-1111-4111-8111-111111111111",
            "test",
            None,
            tx,
        )
        .await;
        assert!(result.is_err());
    }
}
