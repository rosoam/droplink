//! Phase 0 spike: two WebRTC peers in one process, one data channel, 10 MiB with BLAKE3.
//! No signaling server, no STUN: host candidates only. Proves the library works and gives a
//! throughput baseline before wiring it into DropLink.

use anyhow::{anyhow, Result};
use bytes::BytesMut;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use webrtc::data_channel::{DataChannel, DataChannelEvent};
use webrtc::peer_connection::{
    register_default_interceptors, MediaEngine, RTCConfigurationBuilder, RTCIceGatheringState,
    RTCPeerConnectionState, Registry,
};
use webrtc::peer_connection::{PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler};
use webrtc::runtime::{default_runtime, Sender};

const PAYLOAD: usize = 10 * 1024 * 1024;
const CHUNK: usize = 16 * 1024;

struct GatherHandler {
    tx: Sender<()>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for GatherHandler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let _ = self.tx.try_send(());
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        println!("peer state: {state}");
    }
}

struct AnswerHandler {
    tx: Sender<()>,
    data: mpsc::UnboundedSender<Arc<dyn DataChannel>>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for AnswerHandler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let _ = self.tx.try_send(());
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        println!("peer state: {state}");
    }

    async fn on_data_channel(&self, dc: Arc<dyn DataChannel>) {
        println!("incoming data channel: {}", dc.label().await.unwrap_or_default());
        let _ = self.data.send(dc);
    }
}

async fn new_peer(
    handler: Arc<dyn PeerConnectionEventHandler>,
    runtime: Arc<dyn webrtc::runtime::Runtime>,
) -> Result<impl PeerConnection> {
    let mut media = MediaEngine::default();
    media.register_default_codecs()?;
    let registry = register_default_interceptors(Registry::new(), &mut media)?;
    PeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::new().build())
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_handler(handler)
        .with_runtime(runtime)
        .with_udp_addrs(vec!["0.0.0.0:0".to_string()])
        .build()
        .await
        .map_err(|e| anyhow!("peer build: {e}"))
}

#[tokio::main]
async fn main() -> Result<()> {
    let runtime = default_runtime().ok_or_else(|| anyhow!("no runtime"))?;

    let (gather_a_tx, mut gather_a_rx) = webrtc::runtime::channel::<()>(1);
    let (gather_b_tx, mut gather_b_rx) = webrtc::runtime::channel::<()>(1);
    let (dc_tx, mut dc_rx) = mpsc::unbounded_channel::<Arc<dyn DataChannel>>();

    let pc_a = new_peer(Arc::new(GatherHandler { tx: gather_a_tx }), runtime.clone()).await?;
    let pc_b = new_peer(
        Arc::new(AnswerHandler {
            tx: gather_b_tx,
            data: dc_tx,
        }),
        runtime.clone(),
    )
    .await?;

    let dc_a: Arc<dyn DataChannel> = pc_a.create_data_channel("drop", None).await?;

    let offer = pc_a.create_offer(None).await?;
    pc_a.set_local_description(offer).await?;
    let _ = tokio::time::timeout(Duration::from_secs(10), gather_a_rx.recv()).await?;
    let offer_sdp = pc_a.local_description().await.ok_or_else(|| anyhow!("no offer"))?;

    pc_b.set_remote_description(offer_sdp).await?;
    let answer = pc_b.create_answer(None).await?;
    pc_b.set_local_description(answer).await?;
    let _ = tokio::time::timeout(Duration::from_secs(10), gather_b_rx.recv()).await?;
    let answer_sdp = pc_b.local_description().await.ok_or_else(|| anyhow!("no answer"))?;

    pc_a.set_remote_description(answer_sdp).await?;

    let dc_b = tokio::time::timeout(Duration::from_secs(10), dc_rx.recv())
        .await?
        .ok_or_else(|| anyhow!("no incoming data channel"))?;

    // Receiver: accumulate, then answer the hash on the "END" marker.
    let (hash_tx, mut hash_rx) = mpsc::channel::<String>(1);
    tokio::spawn(async move {
        let mut buffer = Vec::with_capacity(PAYLOAD);
        loop {
            match dc_b.poll().await {
                Some(DataChannelEvent::OnMessage(message)) => {
                    if message.data.as_ref() == b"END" {
                        let hash = blake3::hash(&buffer).to_hex().to_string();
                        let _ = dc_b.send(BytesMut::from(hash.as_bytes())).await;
                        let _ = hash_tx.send(hash).await;
                        break;
                    }
                    buffer.extend_from_slice(&message.data);
                }
                Some(DataChannelEvent::OnClose) | None => break,
                _ => {}
            }
        }
    });

    // Sender: wait for open, stream, then wait for the receiver's hash.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match dc_a.poll().await {
                Some(DataChannelEvent::OnOpen) => break,
                Some(DataChannelEvent::OnClose) | None => panic!("data channel closed before open"),
                _ => {}
            }
        }
    })
    .await?;

    let payload = vec![0xABu8; PAYLOAD];
    let expected = blake3::hash(&payload).to_hex().to_string();
    let started = Instant::now();
    for chunk in payload.chunks(CHUNK) {
        dc_a.send(BytesMut::from(chunk)).await?;
    }
    dc_a.send(BytesMut::from(b"END".as_slice())).await?;

    let got = tokio::time::timeout(Duration::from_secs(60), hash_rx.recv())
        .await?
        .ok_or_else(|| anyhow!("receiver gave no hash"))?;
    let elapsed = started.elapsed();

    if got != expected {
        return Err(anyhow!("hash mismatch: {got} != {expected}"));
    }
    let mib = PAYLOAD as f64 / (1024.0 * 1024.0);
    println!(
        "OK {:.1} MiB in {:.2?} ({:.1} MiB/s), blake3 {}",
        mib,
        elapsed,
        mib / elapsed.as_secs_f64(),
        got
    );

    pc_a.close().await?;
    pc_b.close().await?;
    Ok(())
}
