use crate::protocol::{DeviceInfo, Message};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};

const CHUNK_SIZE: usize = 65536; // 64KB

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransferEvent {
    IncomingRequest {
        transfer_id: String,
        sender_name: String,
        file_name: String,
        file_size: u64,
    },
    Progress {
        transfer_id: String,
        bytes_done: u64,
        total: u64,
        direction: String,
    },
    Complete {
        transfer_id: String,
        file_name: String,
        saved_path: String,
    },
    Failed {
        transfer_id: String,
        reason: String,
    },
}

pub type PendingMap = Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>;

pub struct TransferServer {
    port: u16,
    pending: PendingMap,
    event_tx: mpsc::Sender<TransferEvent>,
}

impl TransferServer {
    pub fn new(port: u16, event_tx: mpsc::Sender<TransferEvent>) -> Self {
        TransferServer {
            port,
            pending: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
        }
    }

    pub fn pending_map(&self) -> PendingMap {
        self.pending.clone()
    }

    pub async fn start(&self) -> Result<(), String> {
        let listener = TcpListener::bind(format!("0.0.0.0:{}", self.port))
            .await
            .map_err(|e| e.to_string())?;

        let pending = self.pending.clone();
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            loop {
                if let Ok((stream, _addr)) = listener.accept().await {
                    let pending = pending.clone();
                    let event_tx = event_tx.clone();
                    tokio::spawn(handle_incoming(stream, pending, event_tx));
                }
            }
        });

        Ok(())
    }
}

async fn handle_incoming(
    stream: TcpStream,
    pending: PendingMap,
    event_tx: mpsc::Sender<TransferEvent>,
) {
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut writer = BufWriter::new(writer);

    // Read HELLO message (newline-delimited JSON)
    let mut line = String::new();
    if tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line)
        .await
        .is_err()
    {
        return;
    }

    let hello: Message = match serde_json::from_str(line.trim()) {
        Ok(m) => m,
        Err(_) => return,
    };

    let (transfer_id, sender_name, file_name, file_size, checksum) = match hello {
        Message::Hello {
            transfer_id,
            sender_name,
            file_name,
            file_size,
            checksum,
        } => (transfer_id, sender_name, file_name, file_size, checksum),
        _ => return,
    };

    const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024 * 1024; // 10 GB
    if file_size > MAX_FILE_SIZE {
        let error_msg = serde_json::to_string(&Message::Error {
            reason: "File too large".to_string(),
        }).unwrap_or_default();
        let _ = writer.write_all(format!("{}\n", error_msg).as_bytes()).await;
        let _ = writer.flush().await;
        return;
    }

    // Insert into pending BEFORE notifying frontend to avoid race condition
    // where frontend responds before the entry exists in pending map.
    let (decision_tx, decision_rx) = tokio::sync::oneshot::channel::<bool>();
    pending
        .lock()
        .await
        .insert(transfer_id.clone(), decision_tx);

    // Notify frontend — user must accept/refuse
    let _ = event_tx
        .send(TransferEvent::IncomingRequest {
            transfer_id: transfer_id.clone(),
            sender_name,
            file_name: file_name.clone(),
            file_size,
        })
        .await;

    // Wait for user decision (10s timeout)
    let accepted = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        decision_rx,
    )
    .await
    .unwrap_or(Ok(false))
    .unwrap_or(false);

    // Clean up pending entry in all cases (timeout or decision)
    pending.lock().await.remove(&transfer_id);

    if !accepted {
        let refuse = serde_json::to_string(&Message::Refuse {
            transfer_id: transfer_id.clone(),
        })
        .unwrap()
            + "\n";
        let _ = writer.write_all(refuse.as_bytes()).await;
        let _ = writer.flush().await;
        return;
    }

    // Send ACCEPT
    let accept =
        serde_json::to_string(&Message::Accept { transfer_id: transfer_id.clone() }).unwrap()
            + "\n";
    let _ = writer.write_all(accept.as_bytes()).await;
    let _ = writer.flush().await;

    // Receive file
    let downloads_dir = dirs_next::download_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let droplink_dir = downloads_dir.join("DropLink");
    let _ = tokio::fs::create_dir_all(&droplink_dir).await;

    let safe_name = sanitize_filename(&file_name);
    let dest_path = unique_path(&droplink_dir, &safe_name);

    let mut file = match tokio::fs::File::create(&dest_path).await {
        Ok(f) => f,
        Err(e) => {
            let _ = event_tx
                .send(TransferEvent::Failed {
                    transfer_id,
                    reason: e.to_string(),
                })
                .await;
            return;
        }
    };

    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut bytes_done: u64 = 0;
    let mut hasher = Sha256::new();

    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
                if let Err(e) = file.write_all(&buf[..n]).await {
                    let _ = event_tx
                        .send(TransferEvent::Failed {
                            transfer_id: transfer_id.clone(),
                            reason: format!("Write error: {}", e),
                        })
                        .await;
                    let _ = tokio::fs::remove_file(&dest_path).await;
                    return;
                }
                bytes_done += n as u64;

                let _ = event_tx
                    .send(TransferEvent::Progress {
                        transfer_id: transfer_id.clone(),
                        bytes_done,
                        total: file_size,
                        direction: "receiving".to_string(),
                    })
                    .await;

                if bytes_done >= file_size {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let actual_checksum = format!("{:x}", hasher.finalize());
    if actual_checksum != checksum {
        let _ = tokio::fs::remove_file(&dest_path).await;
        let _ = event_tx
            .send(TransferEvent::Failed {
                transfer_id,
                reason: "Checksum mismatch — file corrupted".to_string(),
            })
            .await;
        return;
    }

    let _ = event_tx
        .send(TransferEvent::Complete {
            transfer_id,
            file_name,
            saved_path: dest_path.to_string_lossy().to_string(),
        })
        .await;
}

pub async fn send_file(
    device: DeviceInfo,
    file_path: String,
    event_tx: mpsc::Sender<TransferEvent>,
    device_name: String,
) -> Result<(), String> {
    let path = Path::new(&file_path);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();

    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| e.to_string())?;
    let file_size = file.metadata().await.map_err(|e| e.to_string())?.len();

    // Compute checksum
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = file.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let checksum = format!("{:x}", hasher.finalize());

    // Reopen file for streaming
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| e.to_string())?;

    let transfer_id = uuid::Uuid::new_v4().to_string();
    let mut stream = TcpStream::connect(format!("{}:{}", device.ip, device.port))
        .await
        .map_err(|e| e.to_string())?;

    let hello = serde_json::to_string(&Message::Hello {
        transfer_id: transfer_id.clone(),
        sender_name: device_name,
        file_name: file_name.clone(),
        file_size,
        checksum,
    })
    .unwrap()
        + "\n";

    stream
        .write_all(hello.as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    // Wait for ACCEPT/REFUSE — scoped block releases borrow before stream re-use
    let response: Message = {
        let mut reader = BufReader::new(&mut stream);
        let mut response_line = String::new();
        tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut response_line)
            .await
            .map_err(|e| e.to_string())?;
        serde_json::from_str(response_line.trim()).map_err(|e| e.to_string())?
    };

    match response {
        Message::Refuse { .. } => {
            let _ = event_tx
                .send(TransferEvent::Failed {
                    transfer_id: transfer_id.clone(),
                    reason: "Refusé par le destinataire".to_string(),
                })
                .await;
            return Err("Refused".to_string());
        }
        Message::Accept { .. } => {}
        _ => return Err("Unexpected response".to_string()),
    }

    // Stream file
    let mut bytes_done: u64 = 0;
    loop {
        let n = file.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        stream
            .write_all(&buf[..n])
            .await
            .map_err(|e| e.to_string())?;
        bytes_done += n as u64;

        let _ = event_tx
            .send(TransferEvent::Progress {
                transfer_id: transfer_id.clone(),
                bytes_done,
                total: file_size,
                direction: "sending".to_string(),
            })
            .await;
    }

    stream.flush().await.map_err(|e| e.to_string())?;

    let _ = event_tx
        .send(TransferEvent::Complete {
            transfer_id,
            file_name,
            saved_path: file_path,
        })
        .await;

    Ok(())
}

fn sanitize_filename(name: &str) -> String {
    let no_separators: String = name.chars()
        .filter(|&c| c != '/' && c != '\\' && c != '\0')
        .collect();
    let trimmed = no_separators.trim_start_matches('.');
    if trimmed.is_empty() { "file".to_string() } else { trimmed.to_string() }
}

fn unique_path(dir: &Path, name: &str) -> std::path::PathBuf {
    let base = dir.join(name);
    if !base.exists() {
        return base;
    }
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| format!(".{}", e))
        .unwrap_or_default();
    let mut i = 1;
    loop {
        let candidate = dir.join(format!("{} ({}){}", stem, i, ext));
        if !candidate.exists() {
            return candidate;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_filename("../../../evil"), "evil");
        assert_eq!(sanitize_filename("café.jpg"), "café.jpg");
        assert_eq!(sanitize_filename("写真.png"), "写真.png");
        assert_eq!(sanitize_filename("file/path.txt"), "filepath.txt");
        assert_eq!(sanitize_filename(""), "file");
    }

    #[test]
    fn test_unique_path_no_collision() {
        let tmp = std::env::temp_dir();
        let path = unique_path(&tmp, "droplink_test_nonexistent_12345.txt");
        assert_eq!(path, tmp.join("droplink_test_nonexistent_12345.txt"));
    }
}
