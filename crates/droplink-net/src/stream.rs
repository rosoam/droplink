use droplink_core::frame::{self, Frame, KIND_DATA, KIND_SEALED_CONTROL};
use droplink_core::protocol::Control;
use droplink_core::{Error, Result};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const READ_CHUNK: usize = 64 * 1024;

/// A frame read from the wire.
#[derive(Debug)]
pub enum Incoming {
    Control(Control),
    Data(Vec<u8>),
    SealedControl(Vec<u8>),
}

/// Length-prefixed framing over any async stream.
///
/// The read buffer is reused across frames and only compacted when consumed bytes exceed one read
/// chunk, so a 256 KiB transfer does not reallocate per frame.
pub struct FramedStream<S> {
    inner: S,
    buf: Vec<u8>,
    start: usize,
}

impl<S: AsyncRead + AsyncWrite + Unpin> FramedStream<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            buf: Vec::with_capacity(READ_CHUNK * 2),
            start: 0,
        }
    }

    async fn fill(&mut self) -> Result<usize> {
        if self.start > 0 {
            if self.start == self.buf.len() {
                self.buf.clear();
            } else {
                // Keep only the unconsumed tail: resetting `start` without draining would replay
                // already-decoded bytes.
                self.buf.drain(..self.start);
            }
            self.start = 0;
        }
        let old = self.buf.len();
        self.buf.resize(old + READ_CHUNK, 0);
        let n = self.inner.read(&mut self.buf[old..]).await?;
        self.buf.truncate(old + n);
        Ok(n)
    }

    pub async fn read_frame(&mut self) -> Result<Incoming> {
        loop {
            let decoded = {
                let slice = &self.buf[self.start..];
                match frame::decode(slice)? {
                    Some((Frame::Data(d), n)) => Some((Incoming::Data(d.to_vec()), n)),
                    Some((Frame::SealedControl(d), n)) => {
                        Some((Incoming::SealedControl(d.to_vec()), n))
                    }
                    Some((Frame::Control(c), n)) => Some((Incoming::Control(c), n)),
                    None => None,
                }
            };
            if let Some((incoming, consumed)) = decoded {
                self.start += consumed;
                return Ok(incoming);
            }
            if self.fill().await? == 0 {
                return Err(Error::Io(io::Error::from(io::ErrorKind::UnexpectedEof)));
            }
        }
    }

    pub async fn write_control(&mut self, control: &Control) -> Result<()> {
        let mut out = Vec::new();
        frame::encode_control(&mut out, control)?;
        self.inner.write_all(&out).await?;
        self.inner.flush().await?;
        Ok(())
    }

    pub async fn write_data(&mut self, payload: &[u8]) -> Result<()> {
        let header = frame::header_bytes(KIND_DATA, payload.len())?;
        self.inner.write_all(&header).await?;
        self.inner.write_all(payload).await?;
        Ok(())
    }

    pub async fn write_sealed_control(&mut self, payload: &[u8]) -> Result<()> {
        let header = frame::header_bytes(KIND_SEALED_CONTROL, payload.len())?;
        self.inner.write_all(&header).await?;
        self.inner.write_all(payload).await?;
        self.inner.flush().await?;
        Ok(())
    }

    pub async fn flush(&mut self) -> Result<()> {
        self.inner.flush().await?;
        Ok(())
    }

    pub fn into_inner(self) -> S {
        self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_read_roundtrip() {
        let (client, server) = tokio::io::duplex(1024);
        let mut writer = FramedStream::new(client);
        writer.write_data(b"hello").await.unwrap();
        writer.write_sealed_control(b"sealed").await.unwrap();

        let mut reader = FramedStream::new(server);
        assert!(matches!(reader.read_frame().await.unwrap(), Incoming::Data(d) if d == b"hello"));
        assert!(
            matches!(reader.read_frame().await.unwrap(), Incoming::SealedControl(d) if d == b"sealed")
        );
    }

    #[tokio::test]
    async fn eof_mid_frame_is_error() {
        let (client, server) = tokio::io::duplex(1024);
        drop(client);
        let mut reader = FramedStream::new(server);
        assert!(reader.read_frame().await.is_err());
    }

    #[tokio::test]
    async fn partial_second_frame_does_not_replay_the_first() {
        let (client, server) = tokio::io::duplex(1024);
        let (mut client_read, mut client_write) = tokio::io::split(client);
        let mut reader = FramedStream::new(server);

        // First frame complete, second frame announced but truncated.
        let mut first = Vec::new();
        frame::encode_data(&mut first, b"first").unwrap();
        let mut second = Vec::new();
        frame::encode_data(&mut second, b"second").unwrap();
        let split_at = first.len() + 8;
        let mut combined = first.clone();
        combined.extend_from_slice(&second);
        client_write.write_all(&combined[..split_at]).await.unwrap();

        match reader.read_frame().await.unwrap() {
            Incoming::Data(data) => assert_eq!(data, b"first"),
            other => panic!("unexpected {other:?}"),
        }

        // Completing the second frame must not resurrect the first one.
        client_write.write_all(&combined[split_at..]).await.unwrap();
        match reader.read_frame().await.unwrap() {
            Incoming::Data(data) => assert_eq!(data, b"second"),
            other => panic!("unexpected {other:?}"),
        }

        let _ = &mut client_read;
    }

    #[tokio::test]
    async fn large_payload_roundtrip() {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let payload = vec![0x42u8; 256 * 1024];
        let expected = payload.clone();
        let task = tokio::spawn(async move {
            let mut writer = FramedStream::new(client);
            writer.write_data(&payload).await.unwrap();
        });
        let mut reader = FramedStream::new(server);
        let got = reader.read_frame().await.unwrap();
        task.await.unwrap();
        assert!(matches!(got, Incoming::Data(d) if d == expected));
    }
}
