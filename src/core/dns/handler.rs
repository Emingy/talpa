use anyhow::Result;
use hickory_proto::op::Message;
use std::{net::SocketAddr, sync::Arc};
use tokio::net::UdpSocket;

use crate::core::tunnel::Tunnel;

use super::forwarder::Forwarder;
use super::utils::{get_addresses_from_response, match_domain};

/// Logs the incoming query name/type and which upstream path it takes, so a
/// "wrong answer" can be traced to either the match decision or the upstream.
fn log_query(packet: &[u8], matched: bool) {
    if let Ok(msg) = Message::from_vec(packet)
        && let Some(q) = msg.queries.first()
    {
        log::debug!(
            "[DNS] query {} {:?} -> {}",
            q.name(),
            q.query_type(),
            if matched { "socks (matched)" } else { "direct" }
        );
    }
}

/// Logs the upstream response code and answer count.
fn log_response(resp: &[u8]) {
    if let Ok(msg) = Message::from_vec(resp) {
        log::debug!(
            "[DNS] response {:?}, {} answer(s)",
            msg.metadata.response_code,
            msg.answers.len()
        );
    }
}

pub struct Handler;

impl Handler {
    /// Handle a single UDP query: resolve and send the answer back to the client.
    pub async fn handle(packet: Vec<u8>, client_addr: SocketAddr, socket: Arc<UdpSocket>) {
        match Self::resolve(&packet).await {
            Ok(resp) => {
                let _ = socket.send_to(&resp, client_addr).await;
            }
            Err(e) => log::error!("upstream error: {e}"),
        }
    }

    /// Resolve a raw DNS query packet. Matched domains are forwarded through the
    /// SOCKS tunnel and their resolved IPs get a host route through the TUN;
    /// everything else goes to the plain upstream resolver. Transport-agnostic,
    /// so both the UDP and TCP listeners share this.
    pub async fn resolve(packet: &[u8]) -> Result<Vec<u8>> {
        let matched = match_domain(packet);
        log_query(packet, matched);

        let resp = if matched {
            Forwarder::forward_via_socks(packet).await?
        } else {
            Forwarder::forward(packet).await?
        };

        log_response(&resp);

        if matched {
            for (ip, ttl) in get_addresses_from_response(&resp) {
                if let Err(e) = Tunnel::add_route(ip, ttl).await {
                    log::error!("[TUN] route error: {e}");
                }
            }
        }

        Ok(resp)
    }
}
