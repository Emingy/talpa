use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UdpSocket;
use tokio_socks::tcp::Socks5Stream;

use crate::config::config;

pub struct Forwarder;

impl Forwarder {
    pub async fn forward(packet: &[u8]) -> Result<Vec<u8>> {
        let upstream = UdpSocket::bind("0.0.0.0:0").await?;
        upstream.send_to(packet, &config().dns.upstream_addr).await?;

        let mut buf = [0u8; 4096];
        let (size, _) = upstream.recv_from(&mut buf).await?;

        Ok(buf[..size].to_vec())
    }

    pub async fn forward_via_socks(packet: &[u8]) -> Result<Vec<u8>> {
        let cfg = config();
        let mut stream: Socks5Stream<tokio::net::TcpStream> =
            Socks5Stream::connect(cfg.socks.addr().as_str(), cfg.dns.upstream_socks_addr.as_str())
                .await?;

        // DNS over TCP: 2-byte big-endian length prefix
        let len = (packet.len() as u16).to_be_bytes();
        stream.write_all(&len).await?;
        stream.write_all(packet).await?;

        let mut len_buf = [0u8; 2];
        stream.read_exact(&mut len_buf).await?;
        let resp_len = u16::from_be_bytes(len_buf) as usize;

        let mut resp = vec![0u8; resp_len];
        stream.read_exact(&mut resp).await?;

        Ok(resp)
    }
}
