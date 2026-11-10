use droplink_core::{Error, Result};
use std::net::IpAddr;
use std::time::Duration;
use tokio::net::TcpStream;

pub async fn connect(ip: IpAddr, port: u16, timeout: Duration) -> Result<TcpStream> {
    let stream = tokio::time::timeout(timeout, TcpStream::connect((ip, port)))
        .await
        .map_err(|_| Error::Timeout)??;
    stream.set_nodelay(true)?;
    Ok(stream)
}
