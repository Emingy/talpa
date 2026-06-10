use anyhow::Result;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio_util::sync::CancellationToken;

use crate::config::{self, config};
use crate::core::tunnel;
use crate::platform::{self, DnsConfigurator};

mod forwarder;
mod handler;
mod utils;

use handler::Handler;

pub struct Server;

impl Server {
    pub async fn start(shutdown: CancellationToken) -> Result<()> {
        Self::apply_split_dns().await;
        platform::Sys::flush_cache().await;

        let cfg = config();
        let socket = Arc::new(UdpSocket::bind(&cfg.dns.listen_addr).await?);
        // Also listen on TCP: resolvers fall back to DNS-over-TCP when a UDP
        // answer is truncated (TC bit, responses > 512 bytes — common for long
        // CNAME chains / many A records). Without it, Windows' TCP retry hits a
        // closed port and fails the whole lookup with a connection reset.
        let tcp = TcpListener::bind(&cfg.dns.listen_addr).await?;
        log::info!("DNS proxy running on {} (udp+tcp)", cfg.dns.listen_addr);

        let udp_shutdown = shutdown.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            loop {
                tokio::select! {
                    recv = socket.recv_from(&mut buf) => {
                        let (size, client_addr) = match recv {
                            Ok(v) => v,
                            // Windows surfaces the ICMP "port unreachable" from a
                            // previous send_to (client already closed its ephemeral
                            // port) as WSAECONNRESET (10054) on the *next* recv_from.
                            // It is spurious — the socket is still fine — so keep
                            // serving instead of letting the whole listener die.
                            Err(e) => {
                                log::debug!("[DNS] udp recv error (ignored): {e}");
                                continue;
                            }
                        };
                        let packet = buf[..size].to_vec();
                        let socket = socket.clone();
                        tokio::spawn(async move {
                            Handler::handle(packet, client_addr, socket).await;
                        });
                    }
                    // On stop, break so the socket is dropped and :53 is released.
                    _ = udp_shutdown.cancelled() => break,
                }
            }
        });

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    accept = tcp.accept() => {
                        match accept {
                            Ok((stream, _)) => { tokio::spawn(Self::serve_tcp(stream)); }
                            // Don't let a transient accept error kill the listener.
                            Err(e) => log::debug!("[DNS] tcp accept error (ignored): {e}"),
                        }
                    }
                    _ = shutdown.cancelled() => break,
                }
            }
        });

        Ok(())
    }

    /// Serves DNS-over-TCP on one connection: each query is a 2-byte big-endian
    /// length prefix followed by the message; we reply in the same framing and
    /// keep reading until the client closes (resolvers may pipeline queries).
    async fn serve_tcp(mut stream: TcpStream) {
        loop {
            let mut len_buf = [0u8; 2];
            // A clean EOF here just means the client is done with the connection.
            if stream.read_exact(&mut len_buf).await.is_err() {
                break;
            }
            let len = u16::from_be_bytes(len_buf) as usize;
            let mut packet = vec![0u8; len];
            if stream.read_exact(&mut packet).await.is_err() {
                break;
            }

            match Handler::resolve(&packet).await {
                Ok(resp) => {
                    let prefix = (resp.len() as u16).to_be_bytes();
                    if stream.write_all(&prefix).await.is_err()
                        || stream.write_all(&resp).await.is_err()
                    {
                        break;
                    }
                }
                Err(e) => {
                    log::error!("upstream error (tcp): {e}");
                    break;
                }
            }
        }
    }

    /// Points the OS at our server for the currently-enabled domain masks and
    /// clears any disabled ones, via the platform split-DNS backend.
    async fn apply_split_dns() {
        let cfg = config();
        let tun = tunnel::active_tun();
        let (enabled, disabled): (Vec<String>, Vec<String>) = cfg
            .domains
            .iter()
            .cloned()
            .partition(|m| config::domain_enabled(m));

        if !disabled.is_empty() {
            platform::Sys::clear(tun.as_ref(), &disabled).await;
        }
        if let Err(e) = platform::Sys::apply(tun.as_ref(), &cfg.dns.listen_addr, &enabled).await {
            log::error!("[DNS] split-DNS apply failed: {e}");
        }
    }

    /// Re-applies split-DNS to match the current domain toggles and flushes the
    /// DNS cache. Called by the UI when a domain is toggled while running.
    pub async fn reconcile() {
        Self::apply_split_dns().await;
        platform::Sys::flush_cache().await;
    }

    pub async fn stop() {
        let tun = tunnel::active_tun();
        platform::Sys::clear(tun.as_ref(), &config().domains).await;
        platform::Sys::flush_cache().await;
    }
}