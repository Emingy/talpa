use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::core::matcher::Matcher;

const VER: u8 = 5;
const METHOD_NO_AUTH: u8 = 0x00;
const METHOD_NONE: u8 = 0xFF;
const CMD_CONNECT: u8 = 0x01;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;
const REP_OK: u8 = 0x00;
const REP_FAIL: u8 = 0x01;
const REP_NO_CMD: u8 = 0x07;

#[derive(Debug)]
enum Target {
    Ip(SocketAddr),
    Domain(String, u16),
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Ip(a) => write!(f, "{}", a),
            Target::Domain(d, p) => write!(f, "{}:{}", d, p),
        }
    }
}

pub async fn run(config: Arc<Config>, matcher: Arc<Matcher>) -> std::io::Result<()> {
    let addr = format!("{}:{}", config.listen.addr, config.listen.port);
    let listener = TcpListener::bind(&addr).await?;
    info!("SOCKS5 listening on {}", addr);

    loop {
        let (stream, peer) = listener.accept().await?;
        let config = config.clone();
        let matcher = matcher.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(stream, peer, config, matcher).await {
                debug!("connection {}: {}", peer, e);
            }
        });
    }
}

async fn handle(
    mut client: TcpStream,
    peer: SocketAddr,
    config: Arc<Config>,
    matcher: Arc<Matcher>,
) -> std::io::Result<()> {
    // --- auth negotiation ---
    let mut hdr = [0u8; 2];
    client.read_exact(&mut hdr).await?;
    if hdr[0] != VER {
        return Err(err("not a SOCKS5 client"));
    }
    let nmethods = hdr[1] as usize;
    let mut methods = vec![0u8; nmethods];
    client.read_exact(&mut methods).await?;

    if methods.contains(&METHOD_NO_AUTH) {
        client.write_all(&[VER, METHOD_NO_AUTH]).await?;
    } else {
        client.write_all(&[VER, METHOD_NONE]).await?;
        return Ok(());
    }

    // --- read request ---
    let mut req = [0u8; 4];
    client.read_exact(&mut req).await?;
    if req[0] != VER {
        return Err(err("bad version in request"));
    }
    if req[1] != CMD_CONNECT {
        client.write_all(&reply(REP_NO_CMD)).await?;
        return Ok(());
    }

    let target = read_target(&mut client, req[3]).await?;

    // --- routing decision ---
    let via_upstream = match &target {
        Target::Domain(domain, _) => {
            let matched = matcher.matches_domain(domain);
            if matched {
                info!("[{}] {} → upstream proxy", peer, domain);
            } else {
                debug!("[{}] {} → direct", peer, domain);
            }
            matched
        }
        Target::Ip(addr) => {
            let matched = matcher.matches_ip(addr.ip());
            if matched {
                info!("[{}] {} → upstream proxy (IP rule)", peer, addr);
            } else {
                debug!("[{}] {} → direct (IP)", peer, addr);
            }
            matched
        }
    };

    // --- connect ---
    match if via_upstream {
        connect_upstream(&target, &config).await
    } else {
        connect_direct(&target).await
    } {
        Ok(mut upstream) => {
            client.write_all(&reply(REP_OK)).await?;
            tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
        }
        Err(e) => {
            warn!("[{}] connect to {} failed: {}", peer, target, e);
            client.write_all(&reply(REP_FAIL)).await?;
        }
    }

    Ok(())
}

async fn read_target(stream: &mut TcpStream, atyp: u8) -> std::io::Result<Target> {
    match atyp {
        ATYP_IPV4 => {
            let mut octets = [0u8; 4];
            stream.read_exact(&mut octets).await?;
            let port = read_port(stream).await?;
            Ok(Target::Ip(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(octets)), port)))
        }
        ATYP_IPV6 => {
            let mut octets = [0u8; 16];
            stream.read_exact(&mut octets).await?;
            let port = read_port(stream).await?;
            Ok(Target::Ip(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(octets)), port)))
        }
        ATYP_DOMAIN => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut domain = vec![0u8; len[0] as usize];
            stream.read_exact(&mut domain).await?;
            let port = read_port(stream).await?;
            Ok(Target::Domain(
                String::from_utf8_lossy(&domain).into_owned(),
                port,
            ))
        }
        _ => Err(err("unsupported address type")),
    }
}

async fn read_port(stream: &mut TcpStream) -> std::io::Result<u16> {
    let mut buf = [0u8; 2];
    stream.read_exact(&mut buf).await?;
    Ok(u16::from_be_bytes(buf))
}

async fn connect_direct(target: &Target) -> std::io::Result<TcpStream> {
    match target {
        Target::Ip(addr) => TcpStream::connect(addr).await,
        Target::Domain(host, port) => TcpStream::connect((host.as_str(), *port)).await,
    }
}

async fn connect_upstream(target: &Target, config: &Config) -> std::io::Result<TcpStream> {
    let mut up = TcpStream::connect((config.upstream.addr.as_str(), config.upstream.port)).await?;

    // SOCKS5 handshake
    up.write_all(&[VER, 1, METHOD_NO_AUTH]).await?;
    let mut rsp = [0u8; 2];
    up.read_exact(&mut rsp).await?;
    if rsp[1] != METHOD_NO_AUTH {
        return Err(err("upstream requires auth"));
    }

    // CONNECT request
    let req = build_connect_request(target);
    up.write_all(&req).await?;

    // Read reply and skip BND address
    let mut rep = [0u8; 4];
    up.read_exact(&mut rep).await?;
    if rep[1] != REP_OK {
        return Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            format!("upstream refused: code {}", rep[1]),
        ));
    }
    skip_addr(&mut up, rep[3]).await?;

    Ok(up)
}

fn build_connect_request(target: &Target) -> Vec<u8> {
    let mut req = vec![VER, CMD_CONNECT, 0x00];
    match target {
        Target::Domain(host, port) => {
            let b = host.as_bytes();
            req.push(ATYP_DOMAIN);
            req.push(b.len() as u8);
            req.extend_from_slice(b);
            req.extend_from_slice(&port.to_be_bytes());
        }
        Target::Ip(SocketAddr::V4(a)) => {
            req.push(ATYP_IPV4);
            req.extend_from_slice(&a.ip().octets());
            req.extend_from_slice(&a.port().to_be_bytes());
        }
        Target::Ip(SocketAddr::V6(a)) => {
            req.push(ATYP_IPV6);
            req.extend_from_slice(&a.ip().octets());
            req.extend_from_slice(&a.port().to_be_bytes());
        }
    }
    req
}

async fn skip_addr(stream: &mut TcpStream, atyp: u8) -> std::io::Result<()> {
    let n = match atyp {
        ATYP_IPV4 => 4 + 2,
        ATYP_IPV6 => 16 + 2,
        ATYP_DOMAIN => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            len[0] as usize + 2
        }
        _ => return Ok(()),
    };
    let mut buf = vec![0u8; n];
    stream.read_exact(&mut buf).await?;
    Ok(())
}

fn reply(code: u8) -> [u8; 10] {
    // VER REP RSV ATYP(IPv4) 0.0.0.0:0
    [VER, code, 0x00, ATYP_IPV4, 0, 0, 0, 0, 0, 0]
}

fn err(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg)
}
