use serde::{Deserialize, Serialize};

/// Stable identifier owned by the client (UUIDv4 as string).
pub type DeviceId = String;
/// Ephemeral identifier assigned by the relay server (UUIDv4 as string).
pub type PeerId = String;
/// Identifier of a single transfer (UUIDv4 as string).
pub type TransferId = String;

pub const PROTOCOL_VERSION: u8 = 2;
pub const SERVICE_TYPE: &str = "_droplink._tcp.local.";
pub const MAX_CONTROL_LEN: u32 = 64 * 1024;
pub const MAX_CHUNK: u32 = 1024 * 1024;
pub const DEFAULT_CHUNK: u32 = 256 * 1024;
pub const MAX_FILE_SIZE: u64 = 64 * 1024 * 1024 * 1024;
pub const HASH_LEN: usize = 32;
pub const MAX_ALIAS_LEN: usize = 64;
pub const AEAD_TAG_LEN: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Send,
    Receive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportMode {
    Direct,
    Relay,
}

/// Plaintext control frames of the DropLink v2 handshake.
///
/// Sealing rules (normative):
/// - `Hello` and `KeyOffer` travel in clear.
/// - Every following control frame (`Accept`, `Refuse`, `End`, `Done`, `Error`) is AEAD sealed and
///   consumes the next nonce of its direction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Control {
    Hello {
        version: u8,
        transfer_id: TransferId,
        chunk_size: u32,
        file_name: String,
        file_size: u64,
        eph_pub: [u8; 32],
        alias: String,
    },
    KeyOffer {
        eph_pub: [u8; 32],
    },
    Accept {
        transfer_id: TransferId,
    },
    Refuse {
        transfer_id: TransferId,
        reason: String,
    },
    End {
        transfer_id: TransferId,
        hash: [u8; 32],
    },
    Done {
        transfer_id: TransferId,
        ok: bool,
        reason: Option<String>,
    },
    Error {
        transfer_id: Option<TransferId>,
        reason: String,
    },
}

impl Control {
    pub fn transfer_id(&self) -> Option<&str> {
        match self {
            Control::Hello { transfer_id, .. }
            | Control::Accept { transfer_id }
            | Control::Refuse { transfer_id, .. }
            | Control::End { transfer_id, .. }
            | Control::Done { transfer_id, .. } => Some(transfer_id),
            Control::KeyOffer { .. } => None,
            Control::Error { transfer_id, .. } => transfer_id.as_deref(),
        }
    }

    /// Validate the fields that are attacker-controlled when received from the network.
    pub fn validate(&self) -> crate::Result<()> {
        match self {
            Control::Hello {
                version,
                file_name,
                file_size,
                chunk_size,
                alias,
                ..
            } => {
                if *version != PROTOCOL_VERSION {
                    return Err(crate::Error::VersionMismatch {
                        got: *version,
                        expected: PROTOCOL_VERSION,
                    });
                }
                if *file_size > MAX_FILE_SIZE {
                    return Err(crate::Error::Protocol(format!(
                        "file_size {file_size} exceeds {MAX_FILE_SIZE}"
                    )));
                }
                if *chunk_size == 0 || *chunk_size > MAX_CHUNK {
                    return Err(crate::Error::Protocol(format!(
                        "chunk_size {chunk_size} out of range"
                    )));
                }
                if file_name.trim().is_empty()
                    || file_name == ".."
                    || file_name.contains('/')
                    || file_name.contains('\\')
                    || file_name.contains('\0')
                {
                    return Err(crate::Error::InvalidFileName(file_name.clone()));
                }
                if alias.trim().is_empty() || alias.chars().count() > MAX_ALIAS_LEN {
                    return Err(crate::Error::Protocol("invalid alias".into()));
                }
            }
            Control::KeyOffer { .. }
            | Control::Accept { .. }
            | Control::Refuse { .. }
            | Control::End { .. }
            | Control::Done { .. }
            | Control::Error { .. } => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello() -> Control {
        Control::Hello {
            version: PROTOCOL_VERSION,
            transfer_id: "11111111-1111-4111-8111-111111111111".into(),
            chunk_size: DEFAULT_CHUNK,
            file_name: "photo.jpg".into(),
            file_size: 1024,
            eph_pub: [0u8; 32],
            alias: "Mac".into(),
        }
    }

    #[test]
    fn hello_validates() {
        assert!(hello().validate().is_ok());
    }

    #[test]
    fn wrong_version_rejected() {
        let mut h = hello();
        if let Control::Hello { version, .. } = &mut h {
            *version = 1;
        }
        assert!(matches!(
            h.validate(),
            Err(crate::Error::VersionMismatch { got: 1, .. })
        ));
    }

    #[test]
    fn oversize_file_rejected() {
        let mut h = hello();
        if let Control::Hello { file_size, .. } = &mut h {
            *file_size = MAX_FILE_SIZE + 1;
        }
        assert!(matches!(h.validate(), Err(crate::Error::Protocol(_))));
    }

    #[test]
    fn zero_chunk_rejected() {
        let mut h = hello();
        if let Control::Hello { chunk_size, .. } = &mut h {
            *chunk_size = 0;
        }
        assert!(matches!(h.validate(), Err(crate::Error::Protocol(_))));
    }

    #[test]
    fn empty_alias_rejected() {
        let mut h = hello();
        if let Control::Hello { alias, .. } = &mut h {
            *alias = "   ".into();
        }
        assert!(matches!(h.validate(), Err(crate::Error::Protocol(_))));
    }

    #[test]
    fn transfer_id_accessor() {
        assert_eq!(
            hello().transfer_id(),
            Some("11111111-1111-4111-8111-111111111111")
        );
        assert_eq!(Control::KeyOffer { eph_pub: [0; 32] }.transfer_id(), None);
    }
}
