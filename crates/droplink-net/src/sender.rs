use droplink_core::crypto::{EphemeralSecret, Session};
use droplink_core::hash::Hasher;
use droplink_core::path::sanitize_file_name;
use droplink_core::protocol::{
    Control, Direction, TransportMode, DEFAULT_CHUNK, MAX_FILE_SIZE, PROTOCOL_VERSION,
};
use droplink_core::{Error, Result};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio_util::sync::CancellationToken;

use crate::engine::{EngineState, ProgressSnapshot};
use crate::stream::{FramedStream, Incoming};

/// Per-file open counter. Tests use it to prove the single-read invariant: one open, one
/// sequential read, no second pass for hashing. Keyed by path so parallel tests never interfere.
static FILE_OPENS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, usize>>,
> = std::sync::OnceLock::new();

/// How many times the sender opened `path` in this process.
pub fn opens_for(path: &Path) -> usize {
    FILE_OPENS
        .get()
        .and_then(|map| map.lock().ok())
        .and_then(|map| map.get(path).copied())
        .unwrap_or(0)
}

fn record_open(path: &Path) {
    let map = FILE_OPENS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Ok(mut map) = map.lock() {
        *map.entry(path.to_path_buf()).or_insert(0) += 1;
    }
}

const DONE_TIMEOUT: Duration = Duration::from_secs(60);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Run the sender side of a transfer over an already-connected transport.
pub(crate) async fn send_file<S>(
    state: Arc<EngineState>,
    stream: S,
    path: &Path,
    transfer_id: &str,
    mode: TransportMode,
    cancel: CancellationToken,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or_else(|| Error::InvalidFileName(path.display().to_string()))?;
    let file_name = sanitize_file_name(&file_name)?;

    record_open(path);
    let mut file = tokio::fs::File::open(path).await?;
    let file_size = file.metadata().await?.len();
    if file_size > MAX_FILE_SIZE {
        return Err(Error::Protocol("file too large".into()));
    }

    let mut framed = FramedStream::new(stream);

    let ephemeral = EphemeralSecret::generate();
    let hello = Control::Hello {
        version: PROTOCOL_VERSION,
        transfer_id: transfer_id.to_string(),
        chunk_size: DEFAULT_CHUNK,
        file_name: file_name.clone(),
        file_size,
        eph_pub: ephemeral.public(),
        alias: state.cfg.device_name.clone(),
    };
    framed.write_control(&hello).await?;

    let key_offer = read_with_timeout(&mut framed, state.cfg.handshake_timeout).await?;
    let peer_public = match key_offer {
        Incoming::Control(Control::KeyOffer { eph_pub }) => eph_pub,
        Incoming::Control(Control::Error { reason, .. }) => return Err(Error::Refused(reason)),
        _ => return Err(Error::Protocol("expected key offer".into())),
    };
    let mut session = Session::sender(&hello, &ephemeral, peer_public)?;

    state.emit(crate::engine::Event::Sas {
        transfer_id: transfer_id.to_string(),
        code: session.sas.clone(),
    });

    let decision_timeout = state.cfg.accept_timeout + Duration::from_secs(10);
    let decision = read_with_timeout(&mut framed, decision_timeout).await?;
    match decision {
        Incoming::SealedControl(ciphertext) => {
            let plain = session.open(&ciphertext)?;
            match serde_json::from_slice::<Control>(&plain)? {
                Control::Accept { .. } => {}
                Control::Refuse { reason, .. } => return Err(Error::Refused(reason)),
                Control::Error { reason, .. } => return Err(Error::Refused(reason)),
                _ => return Err(Error::Protocol("expected accept or refuse".into())),
            }
        }
        Incoming::Control(Control::Error { reason, .. }) => return Err(Error::Refused(reason)),
        _ => return Err(Error::Protocol("expected sealed decision".into())),
    }

    let mut buffer = vec![0u8; DEFAULT_CHUNK as usize];
    let mut hasher = Hasher::new();
    let mut sent: u64 = 0;

    loop {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let read = tokio::time::timeout(IDLE_TIMEOUT, file.read(&mut buffer))
            .await
            .map_err(|_| Error::Timeout)??;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        sent += read as u64;
        let sealed = session.seal(&buffer[..read])?;
        framed.write_data(&sealed).await?;
        state.progress_set(ProgressSnapshot {
            transfer_id: transfer_id.to_string(),
            file_name: file_name.clone(),
            bytes: sent,
            total: file_size,
            direction: Direction::Send,
            transport: mode,
        });
        if sent > file_size {
            return Err(Error::Protocol("source file grew during transfer".into()));
        }
    }

    if sent != file_size {
        return Err(Error::Protocol(
            "source file changed during transfer".into(),
        ));
    }

    let hash = hasher.finalize();
    let end = Control::End {
        transfer_id: transfer_id.to_string(),
        hash,
    };
    let sealed = session.seal(&serde_json::to_vec(&end)?)?;
    framed.write_sealed_control(&sealed).await?;

    let done = read_with_timeout(&mut framed, DONE_TIMEOUT).await?;
    match done {
        Incoming::SealedControl(ciphertext) => {
            let plain = session.open(&ciphertext)?;
            match serde_json::from_slice::<Control>(&plain)? {
                Control::Done { ok: true, .. } => Ok(()),
                Control::Done {
                    ok: false, reason, ..
                } => Err(Error::Refused(
                    reason.unwrap_or_else(|| "receiver rejected the file".into()),
                )),
                _ => Err(Error::Protocol("expected done".into())),
            }
        }
        _ => Err(Error::Protocol("expected sealed done".into())),
    }
}

async fn read_with_timeout<S>(framed: &mut FramedStream<S>, timeout: Duration) -> Result<Incoming>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    tokio::time::timeout(timeout, framed.read_frame())
        .await
        .map_err(|_| Error::Timeout)?
}
