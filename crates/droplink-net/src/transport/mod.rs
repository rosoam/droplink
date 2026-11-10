pub mod direct;
pub mod relay;

use tokio::io::{AsyncRead, AsyncWrite};

/// Blanket trait for anything the transfer engine can run over.
pub trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}
