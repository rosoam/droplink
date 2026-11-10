use crate::engine::{EngineState, Event, IncomingTransfer, ProgressSnapshot};
use crate::fs_util;
use crate::stream::{FramedStream, Incoming};
use droplink_core::crypto::{EphemeralSecret, Session};
use droplink_core::hash::{self, Hasher};
use droplink_core::path::sanitize_file_name;
use droplink_core::protocol::{
    Control, Direction, TransferId, TransportMode, MAX_FILE_SIZE, PROTOCOL_VERSION,
};
use droplink_core::{Error, Result};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;

const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

struct Meta {
    transfer_id: TransferId,
    file_name: String,
    file_size: u64,
    chunk_size: u32,
    sender_alias: String,
    mode: TransportMode,
}

pub(crate) async fn send_busy(stream: TcpStream) -> Result<()> {
    crate::engine::send_plain_error(stream, "busy").await
}

pub(crate) async fn handle(
    state: Arc<EngineState>,
    stream: TcpStream,
    _addr: SocketAddr,
) -> Result<()> {
    stream.set_nodelay(true)?;
    handle_stream(state, stream, TransportMode::Direct).await
}

/// Run the receiver side of a transfer over any connected transport.
pub(crate) async fn handle_stream<S>(
    state: Arc<EngineState>,
    stream: S,
    mode: TransportMode,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let mut framed = FramedStream::new(stream);

    let first = tokio::time::timeout(state.cfg.handshake_timeout, framed.read_frame())
        .await
        .map_err(|_| Error::Timeout)??;

    let hello = match first {
        Incoming::Control(control @ Control::Hello { .. }) => control,
        Incoming::Control(Control::Error { reason, .. }) => return Err(Error::Refused(reason)),
        _ => {
            framed
                .write_control(&Control::Error {
                    transfer_id: None,
                    reason: "expected hello".into(),
                })
                .await?;
            return Err(Error::Protocol("expected hello".into()));
        }
    };

    if let Err(err) = hello.validate() {
        let _ = framed
            .write_control(&Control::Error {
                transfer_id: hello.transfer_id().map(str::to_string),
                reason: err.to_string(),
            })
            .await;
        return Err(err);
    }

    let meta = match &hello {
        Control::Hello {
            version,
            transfer_id,
            chunk_size,
            file_name,
            file_size,
            alias,
            ..
        } => {
            if *version != PROTOCOL_VERSION {
                return Err(Error::VersionMismatch {
                    got: *version,
                    expected: PROTOCOL_VERSION,
                });
            }
            if *file_size > MAX_FILE_SIZE {
                return Err(Error::Protocol("file too large".into()));
            }
            Meta {
                transfer_id: transfer_id.clone(),
                file_name: file_name.clone(),
                file_size: *file_size,
                chunk_size: *chunk_size,
                sender_alias: alias.clone(),
                mode,
            }
        }
        _ => unreachable!(),
    };

    let ephemeral = EphemeralSecret::generate();
    framed
        .write_control(&Control::KeyOffer {
            eph_pub: ephemeral.public(),
        })
        .await?;
    let mut session = Session::receiver(&hello, &ephemeral)?;

    state.emit(Event::Sas {
        transfer_id: meta.transfer_id.clone(),
        code: session.sas.clone(),
    });

    let (decision_tx, decision_rx) = oneshot::channel::<bool>();
    {
        let mut pending = state.pending.lock().await;
        pending.insert(meta.transfer_id.clone(), decision_tx);
    }

    state.emit(Event::Incoming(IncomingTransfer {
        transfer_id: meta.transfer_id.clone(),
        sender_alias: meta.sender_alias.clone(),
        file_name: meta.file_name.clone(),
        file_size: meta.file_size,
        transport: mode,
        sas: session.sas.clone(),
    }));

    let decision = tokio::time::timeout(state.cfg.accept_timeout, decision_rx).await;
    {
        let mut pending = state.pending.lock().await;
        pending.remove(&meta.transfer_id);
    }

    let accepted = match decision {
        Ok(Ok(value)) => value,
        Ok(Err(_)) => false,
        Err(_) => {
            state.emit(Event::IncomingExpired {
                transfer_id: meta.transfer_id.clone(),
            });
            false
        }
    };

    if !accepted {
        let sealed = session.seal(&serde_json::to_vec(&Control::Refuse {
            transfer_id: meta.transfer_id.clone(),
            reason: "refused".into(),
        })?)?;
        let _ = framed.write_sealed_control(&sealed).await;
        state.emit(Event::Failed {
            transfer_id: meta.transfer_id,
            direction: Direction::Receive,
            file_name: meta.file_name,
            reason: "refusé ou délai dépassé".into(),
            transport: mode,
            bytes: meta.file_size,
        });
        return Ok(());
    }

    let sealed = session.seal(&serde_json::to_vec(&Control::Accept {
        transfer_id: meta.transfer_id.clone(),
    })?)?;
    framed.write_sealed_control(&sealed).await?;

    let result = run_transfer(&state, &mut framed, &mut session, &meta).await;

    match result {
        Ok(final_path) => {
            // Emit the terminal event **before** acking `Done`: the sender releases its
            // per-device lock as soon as it reads the ack, so acking first lets the next
            // transfer start before the receiver has published this one's completion. That
            // broke the "one transfer per device" guarantee under load.
            state.emit(Event::Completed {
                transfer_id: meta.transfer_id.clone(),
                direction: Direction::Receive,
                file_name: meta.file_name,
                path: Some(final_path),
                transport: mode,
                bytes: meta.file_size,
            });
            state.progress_remove(&meta.transfer_id);
            let sealed = session.seal(&serde_json::to_vec(&Control::Done {
                transfer_id: meta.transfer_id,
                ok: true,
                reason: None,
            })?)?;
            let _ = framed.write_sealed_control(&sealed).await;
            Ok(())
        }
        Err(err) => {
            state.emit(Event::Failed {
                transfer_id: meta.transfer_id.clone(),
                direction: Direction::Receive,
                file_name: meta.file_name,
                reason: err.to_string(),
                transport: mode,
                bytes: meta.file_size,
            });
            state.progress_remove(&meta.transfer_id);
            let sealed = session.seal(&serde_json::to_vec(&Control::Done {
                transfer_id: meta.transfer_id,
                ok: false,
                reason: Some(err.to_string()),
            })?)?;
            let _ = framed.write_sealed_control(&sealed).await;
            Err(err)
        }
    }
}

async fn run_transfer<S>(
    state: &Arc<EngineState>,
    framed: &mut FramedStream<S>,
    session: &mut Session,
    meta: &Meta,
) -> Result<PathBuf>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let base_name = sanitize_file_name(&meta.file_name)?;
    let (part_path, mut file) = fs_util::create_part(&state.cfg.receive_dir, &base_name).await?;

    let outcome = transfer_body(state, framed, session, meta, &mut file).await;

    let result = match outcome {
        Ok(actual) => {
            if !hash::verify(&actual.expected_hash, &actual.actual_hash) {
                Err(Error::HashMismatch)
            } else if actual.bytes != meta.file_size {
                Err(Error::Protocol(format!(
                    "size mismatch: got {}, expected {}",
                    actual.bytes, meta.file_size
                )))
            } else {
                file.flush().await?;
                file.sync_all().await?;
                drop(file);
                fs_util::publish(&part_path, &state.cfg.receive_dir, &base_name).await
            }
        }
        Err(err) => Err(err),
    };

    if result.is_err() {
        let _ = tokio::fs::remove_file(&part_path).await;
    }
    result
}

struct Outcome {
    bytes: u64,
    expected_hash: [u8; 32],
    actual_hash: [u8; 32],
}

async fn transfer_body<S>(
    state: &Arc<EngineState>,
    framed: &mut FramedStream<S>,
    session: &mut Session,
    meta: &Meta,
    file: &mut tokio::fs::File,
) -> Result<Outcome>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let max_payload = meta.chunk_size as usize + droplink_core::protocol::AEAD_TAG_LEN;
    let mut hasher = Hasher::new();
    let mut bytes: u64 = 0;

    loop {
        if state.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let frame = tokio::time::timeout(IDLE_TIMEOUT, framed.read_frame())
            .await
            .map_err(|_| Error::Timeout)??;
        match frame {
            Incoming::Data(ciphertext) => {
                if ciphertext.len() > max_payload {
                    return Err(Error::FrameTooLarge(ciphertext.len()));
                }
                let plain = session.open(&ciphertext)?;
                if bytes + plain.len() as u64 > meta.file_size {
                    return Err(Error::Protocol("received more bytes than announced".into()));
                }
                hasher.update(&plain);
                file.write_all(&plain).await?;
                bytes += plain.len() as u64;
                state.progress_set(ProgressSnapshot {
                    transfer_id: meta.transfer_id.clone(),
                    file_name: meta.file_name.clone(),
                    bytes,
                    total: meta.file_size,
                    direction: Direction::Receive,
                    transport: meta.mode,
                });
            }
            Incoming::SealedControl(ciphertext) => {
                let plain = session.open(&ciphertext)?;
                match serde_json::from_slice::<Control>(&plain)? {
                    Control::End { hash, .. } => {
                        let actual_hash = hasher.finalize();
                        return Ok(Outcome {
                            bytes,
                            expected_hash: hash,
                            actual_hash,
                        });
                    }
                    Control::Error { reason, .. } => return Err(Error::Refused(reason)),
                    _ => return Err(Error::Protocol("expected end".into())),
                }
            }
            Incoming::Control(control) => {
                return Err(Error::Protocol(format!(
                    "unexpected plaintext control: {control:?}"
                )));
            }
        }
    }
}
