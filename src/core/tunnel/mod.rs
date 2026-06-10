use anyhow::{Context, Result};
use ipstack::{IpStack, IpStackConfig, IpStackStream};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tokio::task::AbortHandle;
use tokio_socks::tcp::Socks5Stream;
use tokio_util::sync::CancellationToken;
use tun::{AbstractDevice, Configuration};

use crate::config::config;
use crate::platform::{self, RouteManager, TunHandle};

static ADDED_ROUTES: LazyLock<Mutex<HashMap<IpAddr, AbortHandle>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// Number of live proxied TCP connections per destination IP. A DNS-derived host
// route must not be torn down while traffic is still flowing over it (removing
// the /32 would send the connection's packets out the default route and stall
// it), so `expire_route` re-arms its timer instead of deleting while this is > 0.
static ACTIVE_CONNS: LazyLock<Mutex<HashMap<IpAddr, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Increments the live-connection count for `ip` on creation and decrements it
/// on drop, so the count tracks the lifetime of a proxied connection.
struct ConnGuard(IpAddr);

impl ConnGuard {
    fn new(ip: IpAddr) -> Self {
        *ACTIVE_CONNS.lock().unwrap().entry(ip).or_insert(0) += 1;
        ConnGuard(ip)
    }
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        let mut map = ACTIVE_CONNS.lock().unwrap();
        if let Some(n) = map.get_mut(&self.0) {
            *n -= 1;
            if *n == 0 {
                map.remove(&self.0);
            }
        }
    }
}

fn active_conn_count(ip: IpAddr) -> usize {
    ACTIVE_CONNS.lock().unwrap().get(&ip).copied().unwrap_or(0)
}

// The live TUN interface, set on start / cleared on stop. Read by the routing
// helpers and by the DNS server (Linux split-DNS needs the interface name).
static ACTIVE_TUN: LazyLock<Mutex<Option<TunHandle>>> = LazyLock::new(|| Mutex::new(None));

/// A snapshot of the live TUN interface, or `None` if the pipeline is stopped.
pub fn active_tun() -> Option<TunHandle> {
    ACTIVE_TUN.lock().unwrap().clone()
}

pub struct Tunnel;

impl Tunnel {
    pub async fn start(shutdown: CancellationToken) -> Result<()> {
        let cfg = config();

        let mut tun_config = Configuration::default();
        platform::Sys::configure_tun(&mut tun_config, &cfg.tun);

        let tun = tun::create_as_async(&tun_config).context("failed to create tun")?;

        // Record the live interface so routing & split-DNS can target it.
        let handle = TunHandle {
            name: tun.tun_name().context("failed to read tun name")?,
            gateway: cfg.tun.gateway.clone(),
        };
        log::info!("[TUN] interface {}", handle.name);
        *ACTIVE_TUN.lock().unwrap() = Some(handle);

        let mut stack_config = IpStackConfig::default();
        stack_config.mtu(cfg.tun.mtu);
        // The `tun` crate normalises the 4-byte PI header per OS, so ipstack must
        // not do its own handling (see platform::RouteManager::ipstack_packet_information).
        stack_config.packet_information(platform::Sys::ipstack_packet_information());

        let mut ip_stack = IpStack::new(stack_config, tun);

        log::info!("[TUN] ipstack started");

        Self::add_static_routes().await;

        let watcher_shutdown = shutdown.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                tokio::select! {
                    _ = interval.tick() => Self::revalidate_routes().await,
                    _ = watcher_shutdown.cancelled() => break,
                }
            }
        });

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = ip_stack.accept() => {
                        match result {
                            Ok(IpStackStream::Tcp(tcp)) => {
                                let dst = tcp.peer_addr();
                                log::info!("[TUN] new TCP -> {dst}");
                                tokio::spawn(Self::proxy_tcp(tcp, dst));
                            }
                            Ok(_) => {}
                            Err(e) => log::error!("[TUN] accept error: {e}"),
                        }
                    }
                    _ = shutdown.cancelled() => break,
                }
            }
        });

        Ok(())
    }

    pub async fn add_route(ip: IpAddr, ttl: u32) -> Result<()> {
        let cfg = config();
        let tun = active_tun().context("no active TUN interface")?;

        platform::Sys::add_route(&tun, &ip.to_string()).await?;
        log::info!("[TUN] route added: {ip} -> {}", tun.gateway);

        // Refresh (or create) the expiry timer even when the route already existed,
        // so a fresh DNS response resets the clock. The timer re-checks after each
        // TTL window and only tears the route down once it has lapsed *and* no
        // connection is still using the IP (removing the /32 mid-connection would
        // send its packets out the default route and stall it).
        let effective_ttl = (ttl as u64).max(cfg.tun.min_ttl_secs);
        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(effective_ttl)).await;
                let live = active_conn_count(ip);
                if live > 0 {
                    log::debug!("[TUN] route {ip} kept alive ({live} active conns)");
                    continue;
                }
                Self::expire_route(ip).await;
                break;
            }
        })
        .abort_handle();

        let old = ADDED_ROUTES.lock().unwrap().insert(ip, handle);
        if let Some(old_handle) = old {
            old_handle.abort();
        }

        log::info!("[TUN] route TTL set: {ip} expires in {effective_ttl}s");
        Ok(())
    }

    async fn add_static_routes() {
        let Some(tun) = active_tun() else { return };
        for spec in &config().tun.static_routes {
            match platform::Sys::add_route(&tun, spec).await {
                Ok(()) => log::info!("[TUN] static route added: {spec} -> {}", tun.gateway),
                Err(e) => log::error!("[TUN] static route add error {spec}: {e}"),
            }
        }
    }

    pub async fn stop() {
        for spec in &config().tun.static_routes {
            match platform::Sys::del_route(spec).await {
                Ok(()) => log::info!("[TUN] static route removed: {spec}"),
                Err(e) => log::error!("[TUN] static route delete error {spec}: {e}"),
            }
        }

        let ips: Vec<IpAddr> = {
            let mut map = ADDED_ROUTES.lock().unwrap();
            for handle in map.values() {
                handle.abort();
            }
            map.drain().map(|(ip, _)| ip).collect()
        };

        for ip in ips {
            match platform::Sys::del_route(&ip.to_string()).await {
                Ok(()) => log::info!("[TUN] route removed: {ip}"),
                Err(e) => log::error!("[TUN] route delete error {ip}: {e}"),
            }
        }

        ACTIVE_TUN.lock().unwrap().take();
    }

    async fn revalidate_routes() {
        let Some(tun) = active_tun() else { return };

        for spec in &config().tun.static_routes {
            if !platform::Sys::route_present(&tun, spec).await {
                let _ = platform::Sys::add_route(&tun, spec).await;
                log::info!("[TUN] static route restored: {spec}");
            }
        }

        let ips: Vec<IpAddr> = ADDED_ROUTES.lock().unwrap().keys().copied().collect();
        for ip in ips {
            let dst = ip.to_string();
            if !platform::Sys::route_present(&tun, &dst).await {
                let _ = platform::Sys::add_route(&tun, &dst).await;
                log::info!("[TUN] route restored: {ip}");
            }
        }
    }

    async fn expire_route(ip: IpAddr) {
        let result = platform::Sys::del_route(&ip.to_string()).await;
        ADDED_ROUTES.lock().unwrap().remove(&ip);
        match result {
            Ok(()) => log::info!("[TUN] route expired: {ip}"),
            Err(e) => log::error!("[TUN] route expire error {ip}: {e}"),
        }
    }

    async fn proxy_tcp(mut tcp: ipstack::IpStackTcpStream, dst: std::net::SocketAddr) {
        // Hold the route alive for as long as this connection is open.
        let _conn = ConnGuard::new(dst.ip());

        let socks = match Socks5Stream::connect(config().socks.addr().as_str(), dst).await {
            Ok(s) => s,
            Err(e) => {
                log::error!("[PROXY] socks5 connect to {dst} failed: {e}");
                return;
            }
        };

        let mut socks = socks.into_inner();

        if let Err(e) = tokio::io::copy_bidirectional(&mut tcp, &mut socks).await {
            // A copy error here is the connection ending, not a setup failure:
            // EOF / reset / idle-timeout are normal lifecycle events (logged at
            // debug); anything else is worth a warn but is still per-connection.
            use std::io::ErrorKind::*;
            if matches!(
                e.kind(),
                TimedOut | ConnectionReset | ConnectionAborted | BrokenPipe | UnexpectedEof | NotConnected
            ) {
                log::debug!("[PROXY] {dst} closed: {e}");
            } else {
                log::warn!("[PROXY] {dst}: {e}");
            }
        }
    }
}
