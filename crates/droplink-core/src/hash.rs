//! BLAKE3 hashing helpers. The streaming API lets the transfer engine hash exactly the bytes it
//! writes, without a second disk read.

pub const HASH_ALGO_NAME: &str = "blake3";

#[derive(Default)]
pub struct Hasher {
    inner: blake3::Hasher,
}

impl Hasher {
    pub fn new() -> Self {
        Self {
            inner: blake3::Hasher::new(),
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    pub fn finalize(self) -> [u8; 32] {
        *self.inner.finalize().as_bytes()
    }

    pub fn finalize_hex(self) -> String {
        self.inner.finalize().to_hex().to_string()
    }
}

pub fn hash_bytes(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

pub fn hash_hex(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

pub fn verify(expected: &[u8; 32], actual: &[u8; 32]) -> bool {
    // Constant-time comparison is not required for integrity checks over an authenticated channel,
    // but it costs nothing and avoids accidental early-exit comparisons.
    let mut diff = 0u8;
    for (a, b) in expected.iter().zip(actual.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_matches_one_shot() {
        let data = vec![7u8; 100_000];
        let mut h = Hasher::new();
        for chunk in data.chunks(4096) {
            h.update(chunk);
        }
        assert_eq!(h.finalize(), hash_bytes(&data));
    }

    #[test]
    fn hex_is_64_chars() {
        assert_eq!(hash_hex(b"x").len(), 64);
    }

    #[test]
    fn verify_detects_difference() {
        let a = hash_bytes(b"a");
        let b = hash_bytes(b"b");
        assert!(verify(&a, &a));
        assert!(!verify(&a, &b));
    }

    #[test]
    fn empty_input_is_stable() {
        assert_eq!(
            hash_hex(b""),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }
}
