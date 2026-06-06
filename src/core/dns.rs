/// Minimal DNS proxy (UDP).
/// Forwards queries for matching domains to `upstream_dns`,
/// everything else to `fallback_dns`.
///
/// Users point macOS DNS (or dnsmasq/etc.) to 127.0.0.1:5300.
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::time::{timeout, Duration};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::core::matcher::Matcher;

pub async fn run(config: Arc<Config>, matcher: Arc<Matcher>) -> std::io::Result<()> {
    let dns_cfg = match &config.dns {
        Some(c) => c,
        None => return Ok(()),
    };

    let addr = format!("127.0.0.1:{}", dns_cfg.listen_port);
    let socket = Arc::new(UdpSocket::bind(&addr).await?);
    info!("DNS proxy listening on {}", addr);

    let mut buf = vec![0u8; 4096];
    loop {
        let (len, src) = socket.recv_from(&mut buf).await?;
        let query = buf[..len].to_vec();
        let socket = socket.clone();
        let matcher = matcher.clone();
        let upstream = dns_cfg.upstream_dns.clone();
        let fallback = dns_cfg.fallback_dns.clone();

        tokio::spawn(async move {
            let server = match parse_qname(&query) {
                Some(domain) if matcher.matches_domain(&domain) => {
                    debug!("DNS {} → upstream ({})", domain, upstream);
                    upstream
                }
                Some(domain) => {
                    debug!("DNS {} → fallback ({})", domain, fallback);
                    fallback
                }
                None => fallback,
            };

            match forward(&query, &server).await {
                Ok(resp) => {
                    if let Err(e) = socket.send_to(&resp, src).await {
                        warn!("DNS send to {}: {}", src, e);
                    }
                }
                Err(e) => warn!("DNS forward to {}: {}", server, e),
            }
        });
    }
}

/// Extract the first QNAME from a DNS message (header + question section).
fn parse_qname(buf: &[u8]) -> Option<String> {
    if buf.len() < 12 {
        return None;
    }
    let mut pos = 12; // skip 12-byte header
    let mut parts = Vec::new();

    loop {
        if pos >= buf.len() {
            return None;
        }
        let label_len = buf[pos] as usize;
        if label_len == 0 {
            break;
        }
        // compression pointer — stop
        if label_len & 0xC0 == 0xC0 {
            break;
        }
        pos += 1;
        if pos + label_len > buf.len() {
            return None;
        }
        parts.push(String::from_utf8_lossy(&buf[pos..pos + label_len]).into_owned());
        pos += label_len;
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("."))
    }
}

async fn forward(query: &[u8], server: &str) -> std::io::Result<Vec<u8>> {
    let sock = UdpSocket::bind("0.0.0.0:0").await?;
    sock.send_to(query, server).await?;

    let mut resp = vec![0u8; 4096];
    let (n, _) = timeout(Duration::from_secs(5), sock.recv_from(&mut resp))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "DNS timeout"))??;

    resp.truncate(n);
    Ok(resp)
}
