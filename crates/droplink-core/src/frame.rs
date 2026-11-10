use crate::protocol::{Control, AEAD_TAG_LEN, MAX_CHUNK, MAX_CONTROL_LEN, PROTOCOL_VERSION};
use crate::{Error, Result};

pub const KIND_CONTROL: u8 = 0;
pub const KIND_DATA: u8 = 1;
/// Sealed control frame (ciphertext of a `Control` JSON payload).
pub const KIND_SEALED_CONTROL: u8 = 2;
/// `kind` (1) + `version` (1) + `len` (4, big-endian).
pub const HEADER_LEN: usize = 6;

/// A decoded frame. `Data` borrows from the caller's buffer: no copy, no allocation.
#[derive(Debug)]
pub enum Frame<'a> {
    Control(Control),
    Data(&'a [u8]),
    SealedControl(&'a [u8]),
}

pub fn encode_control(dst: &mut Vec<u8>, c: &Control) -> Result<()> {
    let payload = serde_json::to_vec(c)?;
    encode_header(dst, KIND_CONTROL, payload.len(), MAX_CONTROL_LEN as usize)?;
    dst.extend_from_slice(&payload);
    Ok(())
}

pub fn encode_data(dst: &mut Vec<u8>, payload: &[u8]) -> Result<()> {
    encode_header(
        dst,
        KIND_DATA,
        payload.len(),
        MAX_CHUNK as usize + AEAD_TAG_LEN,
    )?;
    dst.extend_from_slice(payload);
    Ok(())
}

pub fn encode_sealed_control(dst: &mut Vec<u8>, ciphertext: &[u8]) -> Result<()> {
    encode_header(
        dst,
        KIND_SEALED_CONTROL,
        ciphertext.len(),
        MAX_CONTROL_LEN as usize + AEAD_TAG_LEN,
    )?;
    dst.extend_from_slice(ciphertext);
    Ok(())
}

fn encode_header(dst: &mut Vec<u8>, kind: u8, len: usize, max: usize) -> Result<()> {
    let header = header_with_max(kind, len, max)?;
    dst.reserve(HEADER_LEN + len);
    dst.extend_from_slice(&header);
    Ok(())
}

/// Header for `kind`, validating `len` against the kind's maximum. Lets callers stream a payload
/// without building an intermediate buffer.
pub fn header_bytes(kind: u8, len: usize) -> Result<[u8; HEADER_LEN]> {
    let max = match kind {
        KIND_CONTROL => MAX_CONTROL_LEN as usize,
        KIND_DATA => MAX_CHUNK as usize + AEAD_TAG_LEN,
        KIND_SEALED_CONTROL => MAX_CONTROL_LEN as usize + AEAD_TAG_LEN,
        other => return Err(Error::Protocol(format!("unknown frame kind {other}"))),
    };
    header_with_max(kind, len, max)
}

fn header_with_max(kind: u8, len: usize, max: usize) -> Result<[u8; HEADER_LEN]> {
    if len > max {
        return Err(Error::FrameTooLarge(len));
    }
    let mut header = [0u8; HEADER_LEN];
    header[0] = kind;
    header[1] = PROTOCOL_VERSION;
    header[2..].copy_from_slice(&(len as u32).to_be_bytes());
    Ok(header)
}

/// Decode one frame from `buf`.
///
/// Returns `Ok(None)` when the buffer does not hold a complete frame yet. Never panics on
/// arbitrary input and never allocates on the data path.
pub fn decode(buf: &[u8]) -> Result<Option<(Frame<'_>, usize)>> {
    if buf.len() < HEADER_LEN {
        return Ok(None);
    }
    let kind = buf[0];
    let version = buf[1];
    if version != PROTOCOL_VERSION {
        return Err(Error::VersionMismatch {
            got: version,
            expected: PROTOCOL_VERSION,
        });
    }
    let len = u32::from_be_bytes([buf[2], buf[3], buf[4], buf[5]]) as usize;
    let max = match kind {
        KIND_CONTROL => MAX_CONTROL_LEN as usize,
        KIND_DATA => MAX_CHUNK as usize + AEAD_TAG_LEN,
        KIND_SEALED_CONTROL => MAX_CONTROL_LEN as usize + AEAD_TAG_LEN,
        other => {
            return Err(Error::Protocol(format!("unknown frame kind {other}")));
        }
    };
    if len > max {
        return Err(Error::FrameTooLarge(len));
    }
    let end = HEADER_LEN
        .checked_add(len)
        .ok_or(Error::FrameTooLarge(len))?;
    if buf.len() < end {
        return Ok(None);
    }
    let payload = &buf[HEADER_LEN..end];
    match kind {
        KIND_CONTROL => {
            let control: Control = serde_json::from_slice(payload)?;
            control.validate()?;
            Ok(Some((Frame::Control(control), end)))
        }
        KIND_DATA => Ok(Some((Frame::Data(payload), end))),
        _ => Ok(Some((Frame::SealedControl(payload), end))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::TransferId;

    fn hello() -> Control {
        Control::Hello {
            version: PROTOCOL_VERSION,
            transfer_id: "11111111-1111-4111-8111-111111111111".to_string(),
            chunk_size: 4096,
            file_name: "photo.jpg".to_string(),
            file_size: 1024,
            eph_pub: [7u8; 32],
            alias: "MacBook".to_string(),
        }
    }

    #[test]
    fn control_roundtrip() {
        let mut buf = Vec::new();
        encode_control(&mut buf, &hello()).unwrap();
        let (frame, consumed) = decode(&buf).unwrap().unwrap();
        assert_eq!(consumed, buf.len());
        match frame {
            Frame::Control(c) => assert_eq!(c, hello()),
            _ => panic!("expected control"),
        }
    }

    #[test]
    fn data_roundtrip() {
        let payload: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
        let mut buf = Vec::new();
        encode_data(&mut buf, &payload).unwrap();
        let (frame, consumed) = decode(&buf).unwrap().unwrap();
        assert_eq!(consumed, buf.len());
        match frame {
            Frame::Data(d) => assert_eq!(d, payload.as_slice()),
            _ => panic!("expected data"),
        }
    }

    #[test]
    fn partial_buffer_returns_none() {
        let mut buf = Vec::new();
        encode_control(&mut buf, &hello()).unwrap();
        for cut in 0..buf.len() {
            assert!(decode(&buf[..cut]).unwrap().is_none(), "cut={cut}");
        }
    }

    #[test]
    fn two_frames_in_one_buffer() {
        let mut buf = Vec::new();
        encode_data(&mut buf, b"abc").unwrap();
        encode_control(&mut buf, &hello()).unwrap();
        let (first, n1) = decode(&buf).unwrap().unwrap();
        assert!(matches!(first, Frame::Data(b"abc")));
        let (second, n2) = decode(&buf[n1..]).unwrap().unwrap();
        assert!(matches!(second, Frame::Control(Control::Hello { .. })));
        assert_eq!(n1 + n2, buf.len());
    }

    #[test]
    fn huge_length_rejected() {
        let buf = [KIND_DATA, PROTOCOL_VERSION, 0xFF, 0xFF, 0xFF, 0xFF];
        assert!(matches!(decode(&buf), Err(Error::FrameTooLarge(_))));
    }

    #[test]
    fn bad_version_rejected() {
        let mut buf = Vec::new();
        encode_control(&mut buf, &hello()).unwrap();
        buf[1] = 1;
        assert!(matches!(
            decode(&buf),
            Err(Error::VersionMismatch { got: 1, .. })
        ));
    }

    #[test]
    fn unknown_kind_rejected() {
        let buf = [9u8, PROTOCOL_VERSION, 0, 0, 0, 0];
        assert!(matches!(decode(&buf), Err(Error::Protocol(_))));
    }

    #[test]
    fn invalid_json_is_protocol_error() {
        let payload = b"{not json}";
        let mut buf = vec![KIND_CONTROL, PROTOCOL_VERSION, 0, 0, 0, payload.len() as u8];
        buf.extend_from_slice(payload);
        assert!(matches!(decode(&buf), Err(Error::Json(_))));
    }

    #[test]
    fn hello_with_path_traversal_rejected() {
        let mut c = hello();
        if let Control::Hello { file_name, .. } = &mut c {
            *file_name = "../evil".to_string();
        }
        let mut buf = Vec::new();
        encode_control(&mut buf, &c).unwrap();
        assert!(matches!(decode(&buf), Err(Error::InvalidFileName(_))));
    }

    #[test]
    fn accept_roundtrip() {
        let c = Control::Accept {
            transfer_id: TransferId::from("t1"),
        };
        let mut buf = Vec::new();
        encode_control(&mut buf, &c).unwrap();
        assert!(matches!(
            decode(&buf).unwrap().unwrap().0,
            Frame::Control(Control::Accept { .. })
        ));
    }

    mod proptest_tests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #![proptest_config(ProptestConfig { cases: 10_000, ..ProptestConfig::default() })]

            #[test]
            fn data_roundtrip_prop(payload in proptest::collection::vec(any::<u8>(), 0..4096)) {
                let mut buf = Vec::new();
                encode_data(&mut buf, &payload).unwrap();
                let (frame, consumed) = decode(&buf).unwrap().unwrap();
                prop_assert_eq!(consumed, buf.len());
                match frame {
                    Frame::Data(d) => prop_assert_eq!(d, payload.as_slice()),
                    _ => prop_assert!(false, "expected data frame"),
                }
            }

            #[test]
            fn decode_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
                let _ = decode(&bytes);
            }
        }
    }
}
