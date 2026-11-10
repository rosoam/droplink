//! End-to-end encryption for DropLink transfers.
//!
//! Construction (normative, see `plans/2026-11-10-droplink-rust-v1.md`):
//!
//! ```text
//! shared     = X25519(A_secret, B_public)              (must be contributory)
//! transcript = BLAKE3("droplink/v2" || hello_canon || B_public)
//! key_s2r    = HKDF-SHA256(shared, salt=transcript, info="droplink/s2r")
//! key_r2s    = HKDF-SHA256(shared, salt=transcript, info="droplink/r2s")
//! sas        = format!("{:06}", u32_be(BLAKE3("droplink/v2" || transcript || "/sas")[0..4]) % 1_000_000)
//! ```
//!
//! Every direction has its own key and its own nonce counter, so nonces are never reused across
//! directions.

use crate::protocol::{Control, Direction};
use crate::{Error, Result};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroize;

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;
const DOMAIN: &[u8] = b"droplink/v2";

pub struct EphemeralSecret {
    inner: StaticSecret,
    public: [u8; 32],
}

impl EphemeralSecret {
    pub fn generate() -> Self {
        let inner = StaticSecret::random_from_rng(rand_core::OsRng);
        let public = PublicKey::from(&inner).to_bytes();
        Self { inner, public }
    }

    /// Deterministic constructor, for tests and known-answer vectors only.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        let inner = StaticSecret::from(bytes);
        let public = PublicKey::from(&inner).to_bytes();
        Self { inner, public }
    }

    pub fn public(&self) -> [u8; 32] {
        self.public
    }
}

/// An established session: transcript, short authentication string and per-direction AEAD state.
pub struct Session {
    pub transcript: [u8; 32],
    pub sas: String,
    sealer: Sealer,
    opener: Opener,
}

impl Session {
    /// Sender side: owns `A`, received `B`.
    pub fn sender(hello: &Control, a: &EphemeralSecret, b_public: [u8; 32]) -> Result<Self> {
        Self::derive(hello, a, b_public, Direction::Send)
    }

    /// Receiver side: owns `B`, received `A` from the hello.
    pub fn receiver(hello: &Control, b: &EphemeralSecret) -> Result<Self> {
        let a_public = match hello {
            Control::Hello { eph_pub, .. } => *eph_pub,
            _ => return Err(Error::Protocol("receiver requires a Hello".into())),
        };
        Self::derive(hello, b, a_public, Direction::Receive)
    }

    fn derive(
        hello: &Control,
        own: &EphemeralSecret,
        peer_public: [u8; 32],
        direction: Direction,
    ) -> Result<Self> {
        let peer = PublicKey::from(peer_public);
        let shared = own.inner.diffie_hellman(&peer);
        if !shared.was_contributory() {
            return Err(Error::Crypto("non-contributory shared secret".into()));
        }

        let hello_canon = hello_canonical(hello)?;
        let b_public = match direction {
            Direction::Send => peer_public,
            Direction::Receive => own.public,
        };
        let mut hasher = blake3::Hasher::new();
        hasher.update(DOMAIN);
        hasher.update(&hello_canon);
        hasher.update(&b_public);
        let transcript = *hasher.finalize().as_bytes();

        let key_s2r = hkdf_expand(shared.as_bytes(), &transcript, b"droplink/s2r")?;
        let key_r2s = hkdf_expand(shared.as_bytes(), &transcript, b"droplink/r2s")?;

        let sas = sas_code(&transcript);

        let (sealer, opener) = match direction {
            Direction::Send => (
                Sealer::new(key_s2r, transcript),
                Opener::new(key_r2s, transcript),
            ),
            Direction::Receive => (
                Sealer::new(key_r2s, transcript),
                Opener::new(key_s2r, transcript),
            ),
        };

        Ok(Self {
            transcript,
            sas,
            sealer,
            opener,
        })
    }

    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        self.sealer.seal(plaintext)
    }

    pub fn open(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        self.opener.open(ciphertext)
    }

    pub fn sealer(&mut self) -> &mut Sealer {
        &mut self.sealer
    }

    pub fn opener(&mut self) -> &mut Opener {
        &mut self.opener
    }
}

/// Canonical, unambiguous byte encoding of the `Hello` payload that is authenticated.
pub fn hello_canonical(hello: &Control) -> Result<Vec<u8>> {
    let Control::Hello {
        version,
        transfer_id,
        chunk_size,
        file_name,
        file_size,
        eph_pub,
        alias,
    } = hello
    else {
        return Err(Error::Protocol("hello_canonical requires a Hello".into()));
    };

    let mut out = Vec::with_capacity(256);
    out.push(*version);
    put_len_prefixed(&mut out, transfer_id.as_bytes())?;
    out.extend_from_slice(&file_size.to_be_bytes());
    out.extend_from_slice(&chunk_size.to_be_bytes());
    put_len_prefixed(&mut out, file_name.as_bytes())?;
    put_len_prefixed(&mut out, alias.as_bytes())?;
    out.extend_from_slice(eph_pub);
    Ok(out)
}

fn put_len_prefixed(out: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    if bytes.len() > u32::MAX as usize {
        return Err(Error::Protocol("length prefix overflow".into()));
    }
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn hkdf_expand(ikm: &[u8], salt: &[u8; 32], info: &[u8]) -> Result<[u8; KEY_LEN]> {
    let hk = Hkdf::<Sha256>::new(Some(salt), ikm);
    let mut out = [0u8; KEY_LEN];
    hk.expand(info, &mut out)
        .map_err(|_| Error::Crypto("hkdf expand failed".into()))?;
    Ok(out)
}

fn sas_code(transcript: &[u8; 32]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN);
    hasher.update(transcript);
    hasher.update(b"/sas");
    let hash = hasher.finalize();
    let value = u32::from_be_bytes([
        hash.as_bytes()[0],
        hash.as_bytes()[1],
        hash.as_bytes()[2],
        hash.as_bytes()[3],
    ]);
    format!("{:06}", value % 1_000_000)
}

pub struct Sealer {
    cipher: ChaCha20Poly1305,
    key: [u8; KEY_LEN],
    aad: [u8; 32],
    counter: u64,
}

impl Sealer {
    fn new(key: [u8; KEY_LEN], aad: [u8; 32]) -> Self {
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
        Self {
            cipher,
            key,
            aad,
            counter: 0,
        }
    }

    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce = nonce_for(self.counter);
        let out = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &self.aad,
                },
            )
            .map_err(|_| Error::Crypto("seal failed".into()))?;
        self.counter = self
            .counter
            .checked_add(1)
            .ok_or_else(|| Error::Crypto("nonce counter exhausted".into()))?;
        Ok(out)
    }

    pub fn counter(&self) -> u64 {
        self.counter
    }
}

impl Drop for Sealer {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

pub struct Opener {
    cipher: ChaCha20Poly1305,
    key: [u8; KEY_LEN],
    aad: [u8; 32],
    counter: u64,
}

impl Opener {
    fn new(key: [u8; KEY_LEN], aad: [u8; 32]) -> Self {
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
        Self {
            cipher,
            key,
            aad,
            counter: 0,
        }
    }

    pub fn open(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let nonce = nonce_for(self.counter);
        let out = self
            .cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: ciphertext,
                    aad: &self.aad,
                },
            )
            .map_err(|_| Error::Crypto("authentication failed".into()))?;
        self.counter = self
            .counter
            .checked_add(1)
            .ok_or_else(|| Error::Crypto("nonce counter exhausted".into()))?;
        Ok(out)
    }

    pub fn counter(&self) -> u64 {
        self.counter
    }
}

impl Drop for Opener {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

fn nonce_for(counter: u64) -> [u8; NONCE_LEN] {
    let mut nonce = [0u8; NONCE_LEN];
    nonce[4..].copy_from_slice(&counter.to_be_bytes());
    nonce
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{DeviceId, TransferId, DEFAULT_CHUNK, PROTOCOL_VERSION};

    fn hello(eph_pub: [u8; 32]) -> Control {
        Control::Hello {
            version: PROTOCOL_VERSION,
            transfer_id: TransferId::from("11111111-1111-4111-8111-111111111111"),
            chunk_size: DEFAULT_CHUNK,
            file_name: "photo.jpg".into(),
            file_size: 1024,
            eph_pub,
            alias: "Mac".into(),
        }
    }

    fn sessions() -> (Session, Session) {
        let a = EphemeralSecret::from_bytes([1u8; 32]);
        let b = EphemeralSecret::from_bytes([2u8; 32]);
        let h = hello(a.public());
        let sender = Session::sender(&h, &a, b.public()).unwrap();
        let receiver = Session::receiver(&h, &b).unwrap();
        (sender, receiver)
    }

    #[test]
    fn transcript_and_sas_match() {
        let (s, r) = sessions();
        assert_eq!(s.transcript, r.transcript);
        assert_eq!(s.sas, r.sas);
        assert_eq!(s.sas.len(), 6);
    }

    #[test]
    fn sas_is_six_digits() {
        let (s, _) = sessions();
        assert!(s.sas.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn sender_seal_receiver_open() {
        let (mut s, mut r) = sessions();
        for msg in [&b"hello"[..], &b"world"[..], &[] as &[u8]] {
            let sealed = s.seal(msg).unwrap();
            assert_eq!(r.open(&sealed).unwrap(), msg);
        }
    }

    #[test]
    fn direction_keys_differ() {
        let mut s = sessions().0;
        let sealed = s.seal(b"data").unwrap();
        // Opening our own sealed frame with our opener (the other direction key) must fail.
        assert!(s.opener().open(&sealed).is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let (mut s, mut r) = sessions();
        let mut sealed = s.seal(b"payload").unwrap();
        sealed[0] ^= 0x01;
        assert!(matches!(r.open(&sealed), Err(Error::Crypto(_))));
    }

    #[test]
    fn replayed_frame_fails() {
        let (mut s, mut r) = sessions();
        let sealed = s.seal(b"once").unwrap();
        assert!(r.open(&sealed).is_ok());
        assert!(r.open(&sealed).is_err());
    }

    #[test]
    fn non_contributory_public_key_rejected() {
        let a = EphemeralSecret::from_bytes([1u8; 32]);
        let h = hello(a.public());
        let result = Session::sender(&h, &a, [0u8; 32]);
        assert!(matches!(result, Err(Error::Crypto(_))));
    }

    #[test]
    fn transcript_changes_with_metadata() {
        let a = EphemeralSecret::from_bytes([1u8; 32]);
        let base = hello(a.public());
        let mut changed = base.clone();
        if let Control::Hello { file_name, .. } = &mut changed {
            *file_name = "invoice.exe".into();
        }
        assert_ne!(
            hello_canonical(&base).unwrap(),
            hello_canonical(&changed).unwrap()
        );

        let mut bigger = base.clone();
        if let Control::Hello { file_size, .. } = &mut bigger {
            *file_size += 1;
        }
        assert_ne!(
            hello_canonical(&base).unwrap(),
            hello_canonical(&bigger).unwrap()
        );

        let mut renamed = base.clone();
        if let Control::Hello { alias, .. } = &mut renamed {
            *alias = "Other".into();
        }
        assert_ne!(
            hello_canonical(&base).unwrap(),
            hello_canonical(&renamed).unwrap()
        );
    }

    #[test]
    fn known_answer_vector() {
        // Frozen vector: changing the derivation must break this test.
        let a = EphemeralSecret::from_bytes([1u8; 32]);
        let b = EphemeralSecret::from_bytes([2u8; 32]);
        let h = hello(a.public());
        let s = Session::sender(&h, &a, b.public()).unwrap();
        assert_eq!(
            hex(&s.transcript),
            "8e2bce97a1a8076e592aa71f513b01e1266d6cd2b1ff777bf2221dfad1b53b90"
        );
        assert_eq!(s.sas, "247788");
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn device_id_type_is_string() {
        let _: DeviceId = "11111111-1111-4111-8111-111111111111".into();
    }
}
