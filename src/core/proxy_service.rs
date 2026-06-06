use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{error, info};

use crate::config::Config;
use crate::core::matcher::Matcher;

pub enum Cmd { Start, Stop, Quit, Reload(Arc<Config>) }

pub struct ProxyState {
    pub running: AtomicBool,
    pub tunnel_up: AtomicBool,
}

impl ProxyState {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            tunnel_up: AtomicBool::new(false),
        }
    }
}

struct Service {
    shutdown_tx: watch::Sender<bool>,
    handles: Vec<JoinHandle<()>>,
    config: Arc<Config>,
}

impl Service {
    async fn start(
        config: Arc<Config>,
        matcher: Arc<Matcher>,
        state: Arc<ProxyState>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let mut handles: Vec<JoinHandle<()>> = Vec::new();

        // SSH tunnel
        if config.ssh_tunnel.is_some() {
            let cfg = config.clone();
            let rx = shutdown_rx.clone();
            let st = state.clone();
            handles.push(tokio::spawn(async move {
                st.tunnel_up.store(false, Ordering::Relaxed);
                crate::core::tunnel::run(cfg, rx).await;
                st.tunnel_up.store(false, Ordering::Relaxed);
            }));
            // Wait for tunnel port to come up
            let port = config.upstream.port;
            for _ in 0..20 {
                if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                    state.tunnel_up.store(true, Ordering::Relaxed);
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }

        // System proxy + tool configs
        let manage = config.system_proxy.as_ref().map(|s| s.enabled).unwrap_or(false);
        if manage {
            let sp = config.system_proxy.as_ref().unwrap();
            crate::core::system_proxy::enable(&config.listen.addr, config.listen.port).await;
            crate::core::system_proxy::launchctl_set(config.listen.port).await;
            crate::core::system_proxy::zshrc_add(config.listen.port);
            crate::core::system_proxy::configure_tools(sp, config.listen.port).await;
        }

        // SOCKS5
        let cfg = config.clone();
        let m = matcher.clone();
        handles.push(tokio::spawn(async move {
            if let Err(e) = crate::core::socks5::run(cfg, m).await {
                error!("SOCKS5: {}", e);
            }
        }));

        // DNS
        if config.dns.is_some() {
            let cfg = config.clone();
            let m = matcher.clone();
            handles.push(tokio::spawn(async move {
                if let Err(e) = crate::core::dns::run(cfg, m).await {
                    error!("DNS: {}", e);
                }
            }));
        }

        state.running.store(true, Ordering::Relaxed);
        info!("Proxy started");
        Self { shutdown_tx, handles, config }
    }

    async fn stop(self, state: &ProxyState) {
        let _ = self.shutdown_tx.send(true);
        for h in &self.handles { h.abort(); }

        let manage = self.config.system_proxy.as_ref().map(|s| s.enabled).unwrap_or(false);
        if manage {
            let sp = self.config.system_proxy.as_ref().unwrap();
            crate::core::system_proxy::disable().await;
            crate::core::system_proxy::launchctl_unset().await;
            crate::core::system_proxy::zshrc_remove();
            crate::core::system_proxy::unconfigure_tools(sp).await;
        }

        state.running.store(false, Ordering::Relaxed);
        state.tunnel_up.store(false, Ordering::Relaxed);
        info!("Proxy stopped");
    }
}

pub fn run_thread(
    config: Arc<Config>,
    matcher: Arc<Matcher>,
    state: Arc<ProxyState>,
    cmd_rx: std::sync::mpsc::Receiver<Cmd>,
    done_tx: std::sync::mpsc::SyncSender<()>,
) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let mut svc: Option<Service> = None;

    for cmd in cmd_rx {
        match cmd {
            Cmd::Start => {
                if svc.is_none() {
                    svc = Some(rt.block_on(Service::start(
                        config.clone(), matcher.clone(), state.clone(),
                    )));
                }
            }
            Cmd::Stop => {
                if let Some(s) = svc.take() {
                    rt.block_on(s.stop(&state));
                }
            }
            Cmd::Reload(new_config) => {
                if let Some(s) = svc.take() {
                    rt.block_on(s.stop(&state));
                }
                let new_matcher =
                    Arc::new(Matcher::new(&new_config.domains, &new_config.ips));
                svc = Some(rt.block_on(Service::start(
                    new_config, new_matcher, state.clone(),
                )));
            }
            Cmd::Quit => {
                if let Some(s) = svc.take() {
                    rt.block_on(s.stop(&state));
                }
                break;
            }
        }
    }

    let _ = done_tx.try_send(());
}
