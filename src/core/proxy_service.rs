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
    pub connecting: AtomicBool,
    pub tunnel_up: AtomicBool,
    pub tunnel_required: AtomicBool,
}

impl ProxyState {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            connecting: AtomicBool::new(false),
            tunnel_up: AtomicBool::new(false),
            tunnel_required: AtomicBool::new(false),
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
        mut cancel_rx: watch::Receiver<bool>,
    ) -> Option<Self> {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let mut handles: Vec<JoinHandle<()>> = Vec::new();

        // SSH tunnel
        state.tunnel_required.store(config.ssh_tunnel.is_some(), Ordering::Relaxed);
        if config.ssh_tunnel.is_some() {
            let cfg = config.clone();
            let rx = shutdown_rx.clone();
            let st = state.clone();
            handles.push(tokio::spawn(async move {
                st.tunnel_up.store(false, Ordering::Relaxed);
                crate::core::tunnel::run(cfg, rx).await;
                st.tunnel_up.store(false, Ordering::Relaxed);
            }));

            // Wait for tunnel port — interruptible by cancel signal
            let port = config.upstream.port;
            'wait: for _ in 0..20 {
                tokio::select! {
                    biased;
                    _ = cancel_rx.changed() => { break 'wait; }
                    result = tokio::net::TcpStream::connect(("127.0.0.1", port)) => {
                        if result.is_ok() {
                            state.tunnel_up.store(true, Ordering::Relaxed);
                            break 'wait;
                        }
                    }
                }
                tokio::select! {
                    biased;
                    _ = cancel_rx.changed() => { break 'wait; }
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(500)) => {}
                }
            }

            if *cancel_rx.borrow() {
                let _ = shutdown_tx.send(true);
                for h in &handles { h.abort(); }
                state.connecting.store(false, Ordering::Relaxed);
                state.tunnel_required.store(false, Ordering::Relaxed);
                return None;
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
        state.connecting.store(false, Ordering::Relaxed);
        info!("Proxy started");
        Some(Self { shutdown_tx, handles, config })
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
        state.connecting.store(false, Ordering::Relaxed);
        state.tunnel_up.store(false, Ordering::Relaxed);
        state.tunnel_required.store(false, Ordering::Relaxed);
        info!("Proxy stopped");
    }
}

pub fn run_thread(
    config: Arc<Config>,
    matcher: Arc<Matcher>,
    state: Arc<ProxyState>,
    cmd_rx: tokio::sync::mpsc::Receiver<Cmd>,
    done_tx: std::sync::mpsc::SyncSender<()>,
) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(run_async(config, matcher, state, cmd_rx, done_tx));
}

async fn run_async(
    config: Arc<Config>,
    matcher: Arc<Matcher>,
    state: Arc<ProxyState>,
    mut cmd_rx: tokio::sync::mpsc::Receiver<Cmd>,
    done_tx: std::sync::mpsc::SyncSender<()>,
) {
    let mut current_config = config;
    let mut current_matcher = matcher;
    let mut svc: Option<Service> = None;

    'main: loop {
        let cmd = match cmd_rx.recv().await {
            Some(c) => c,
            None => break,
        };

        match cmd {
            Cmd::Start if svc.is_none() => {
                let mut quit = false;
                svc = start_cancellable(
                    current_config.clone(),
                    current_matcher.clone(),
                    state.clone(),
                    &mut cmd_rx,
                    &mut quit,
                )
                .await;
                if quit { break 'main; }
            }
            Cmd::Stop => {
                if let Some(s) = svc.take() {
                    s.stop(&state).await;
                }
            }
            Cmd::Reload(new_config) => {
                if let Some(s) = svc.take() {
                    s.stop(&state).await;
                }
                let new_matcher =
                    Arc::new(Matcher::new(&new_config.domains, &new_config.ips));
                current_config = new_config.clone();
                current_matcher = new_matcher.clone();
                let mut quit = false;
                svc = start_cancellable(new_config, new_matcher, state.clone(), &mut cmd_rx, &mut quit).await;
                if quit { break 'main; }
            }
            Cmd::Quit => {
                if let Some(s) = svc.take() {
                    s.stop(&state).await;
                }
                break 'main;
            }
            _ => {}
        }
    }

    let _ = done_tx.try_send(());
}

/// Starts the service while concurrently processing Stop/Quit commands.
/// Sets `*quit = true` and returns `None` if Quit (or channel close) was received.
/// Returns `None` without setting `quit` if Stop cancelled the startup.
async fn start_cancellable(
    config: Arc<Config>,
    matcher: Arc<Matcher>,
    state: Arc<ProxyState>,
    cmd_rx: &mut tokio::sync::mpsc::Receiver<Cmd>,
    quit: &mut bool,
) -> Option<Service> {
    let (cancel_tx, cancel_rx) = watch::channel(false);
    state.connecting.store(true, Ordering::Relaxed);

    let mut start_handle =
        tokio::spawn(Service::start(config, matcher, state.clone(), cancel_rx));
    let mut cancelled = false;

    loop {
        tokio::select! {
            result = &mut start_handle => {
                return result.unwrap();
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(Cmd::Stop) => {
                        if !cancelled {
                            let _ = cancel_tx.send(true);
                            cancelled = true;
                        }
                        // Keep looping — wait for Service::start to exit cleanly
                    }
                    Some(Cmd::Quit) | None => {
                        if !cancelled {
                            let _ = cancel_tx.send(true);
                        }
                        *quit = true;
                        let _ = start_handle.await;
                        return None;
                    }
                    _ => {} // Ignore Start/Reload during startup
                }
            }
        }
    }
}
