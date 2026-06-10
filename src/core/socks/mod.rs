use anyhow::Result;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::process::Command;

use crate::config::config;
use crate::platform::{self, ProcessControl};

// How long to wait for ssh to bring the SOCKS port up before giving up.
const READY_TIMEOUT: Duration = Duration::from_secs(15);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(200);

// Stores the PID of the running ssh process so stop() can kill it.
static SSH_PID: LazyLock<Mutex<Option<u32>>> = LazyLock::new(|| Mutex::new(None));

pub struct Proxy;

impl Proxy {
    pub async fn start() -> Result<()> {
        let cfg = config();
        let socks_port = cfg.socks.port.to_string();

        let mut child = platform::quiet(
            Command::new("ssh").args([
                "-D", &socks_port,
                "-N",
                "-o", "BatchMode=yes",
                "-o", "StrictHostKeyChecking=no",
                "-o", "ExitOnForwardFailure=yes",
                &cfg.ssh.target,
            ]),
        )
        .spawn()?;

        *SSH_PID.lock().unwrap() = child.id();
        log::info!("[SOCKS5] tunnel via {} on 0.0.0.0:{socks_port}", cfg.ssh.target);

        tokio::spawn(async move {
            let status = child.wait().await;
            let was_stopped = SSH_PID.lock().unwrap().take().is_none();
            if !was_stopped
                && let Ok(s) = status
            {
                log::error!("[SOCKS5] ssh exited unexpectedly with {s}");
            }
        });

        // Block until the SOCKS port actually accepts connections, so the next
        // stages (TUN, DNS) only start once the proxy is really up.
        Self::wait_until_ready(cfg.socks.addr().as_str()).await?;

        Ok(())
    }

    /// Polls the SOCKS address until it accepts a TCP connection, or fails after
    /// [`READY_TIMEOUT`]. ssh's `-N -D` listener appears a moment after spawn.
    async fn wait_until_ready(addr: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
        loop {
            match TcpStream::connect(addr).await {
                Ok(_) => {
                    log::info!("[SOCKS5] proxy ready on {addr}");
                    return Ok(());
                }
                Err(_) if tokio::time::Instant::now() < deadline => {
                    // ssh hasn't bound the port yet — but bail early if it died.
                    if SSH_PID.lock().unwrap().is_none() {
                        anyhow::bail!("ssh exited before the SOCKS port came up");
                    }
                    tokio::time::sleep(READY_POLL_INTERVAL).await;
                }
                Err(e) => anyhow::bail!("SOCKS proxy did not become ready on {addr}: {e}"),
            }
        }
    }

    pub async fn stop() {
        let pid = SSH_PID.lock().unwrap().take();
        if let Some(pid) = pid {
            platform::Sys::terminate_pid(pid).await;
            log::info!("[SOCKS5] tunnel stopped (pid {pid})");
        }
    }
}
