use tao::event_loop::EventLoopProxy;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

use super::UserEvent;
use crate::config::Config;
use crate::core::dns::Server as DnsServer;
use crate::core::socks::Proxy;
use crate::core::tunnel::Tunnel;

/// Pipeline state, mirrored by the tray icon (see `tray::status_icon`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Stopped,
    /// A start or stop is running on the runtime; the UI is awaiting its result.
    Processing,
    Running,
    /// The last start failed; the pipeline was torn back down.
    Error,
}

/// Owns the tokio runtime and the running state of the proxy pipeline.
///
/// `start`/`stop` are **non-blocking**: they flip the state to [`Status::Processing`],
/// spawn the (de)initialisation on the runtime, and return immediately so the
/// event loop can repaint the tray. When the background task finishes it posts a
/// [`UserEvent`] back into the loop, which calls [`Controller::on_started`] /
/// [`Controller::on_stopped`] to settle the final state. `reload`/`shutdown` are
/// blocking — they run to completion before returning.
pub struct Controller {
    rt: Runtime,
    shutdown: Option<CancellationToken>,
    config_path: String,
    status: Status,
    proxy: EventLoopProxy<UserEvent>,
}

impl Controller {
    pub fn new(config_path: String, proxy: EventLoopProxy<UserEvent>) -> anyhow::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
        Ok(Self { rt, shutdown: None, config_path, status: Status::Stopped, proxy })
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn is_running(&self) -> bool {
        self.status == Status::Running
    }

    pub fn config_path(&self) -> &str {
        &self.config_path
    }

    /// Brings up proxy → tunnel → DNS in order.
    async fn bring_up(token: CancellationToken) -> anyhow::Result<()> {
        Proxy::start().await?;
        Tunnel::start(token.clone()).await?;
        DnsServer::start(token).await?;
        Ok(())
    }

    /// Tears down DNS → tunnel → proxy (reverse of `bring_up`).
    async fn tear_down() {
        DnsServer::stop().await;
        Tunnel::stop().await;
        Proxy::stop().await;
    }

    /// Spawns the pipeline bring-up. Settled later by [`on_started`].
    pub fn start(&mut self) {
        if matches!(self.status, Status::Processing | Status::Running) {
            return;
        }
        self.status = Status::Processing;
        let token = CancellationToken::new();
        self.shutdown = Some(token.clone());
        let proxy = self.proxy.clone();
        self.rt.spawn(async move {
            let res = Self::bring_up(token.clone()).await;
            if let Err(e) = &res {
                log::error!("[UI] start failed: {e}");
                token.cancel();
                Self::tear_down().await;
            }
            let _ = proxy.send_event(UserEvent::PipelineStarted(res.is_ok()));
        });
    }

    /// Spawns the pipeline teardown. Settled later by [`on_stopped`].
    pub fn stop(&mut self) {
        if self.status == Status::Processing {
            return;
        }
        let Some(token) = self.shutdown.take() else {
            // Nothing running (also clears a lingering Error state).
            self.status = Status::Stopped;
            return;
        };
        self.status = Status::Processing;
        let proxy = self.proxy.clone();
        self.rt.spawn(async move {
            token.cancel();
            Self::tear_down().await;
            let _ = proxy.send_event(UserEvent::PipelineStopped);
        });
    }

    /// Settles state after a spawned `start` completes.
    pub fn on_started(&mut self, ok: bool) {
        if ok {
            self.status = Status::Running;
            log::info!("[UI] pipeline started");
        } else {
            self.shutdown = None;
            self.status = Status::Error;
        }
    }

    /// Settles state after a spawned `stop` completes.
    pub fn on_stopped(&mut self) {
        self.status = Status::Stopped;
        log::info!("[UI] pipeline stopped");
    }

    /// Re-reads the config file. If the pipeline was running, restarts it so the
    /// new values (TUN params, DNS bind, domains) take effect. Blocking, so the
    /// caller can rebuild the menu immediately afterwards.
    pub fn reload(&mut self) {
        if self.status == Status::Processing {
            return;
        }
        let was_running = self.is_running();
        if let Some(token) = self.shutdown.take() {
            self.rt.block_on(async move {
                token.cancel();
                Self::tear_down().await;
            });
        }

        match Config::reload(&self.config_path) {
            Ok(()) => log::info!("[UI] config reloaded from {}", self.config_path),
            Err(e) => log::error!("[UI] config reload failed: {e}"),
        }

        if was_running {
            let token = CancellationToken::new();
            match self.rt.block_on(Self::bring_up(token.clone())) {
                Ok(()) => {
                    self.shutdown = Some(token);
                    self.status = Status::Running;
                }
                Err(e) => {
                    log::error!("[UI] restart after reload failed: {e}");
                    token.cancel();
                    self.rt.block_on(Self::tear_down());
                    self.status = Status::Error;
                }
            }
        } else {
            self.status = Status::Stopped;
        }
    }

    /// Re-applies split-DNS to match current domain toggles (used when a domain
    /// is toggled while the pipeline is running).
    pub fn reconcile_dns(&self) {
        self.rt.block_on(async {
            DnsServer::reconcile().await;
        });
    }

    /// Synchronous teardown for process exit — runs to completion before return.
    pub fn shutdown(&mut self) {
        if let Some(token) = self.shutdown.take() {
            self.rt.block_on(async move {
                token.cancel();
                Self::tear_down().await;
            });
        }
    }
}
