use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::sync::watch;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

use crate::config::{Config, SshTunnelConfig};

pub async fn run(config: Arc<Config>, mut shutdown: watch::Receiver<bool>) {
    let t = match &config.ssh_tunnel {
        Some(t) => t,
        None => return,
    };

    loop {
        info!(
            "SSH tunnel: connecting {}@{}:{} → 127.0.0.1:{}{}",
            t.user, t.host, t.ssh_port, t.local_port,
            if t.password.is_some() { " (password auth)" } else { " (key auth)" }
        );

        let child = match spawn_ssh(t) {
            Ok(c) => c,
            Err(e) => {
                error!("SSH spawn failed: {}", e);
                if *shutdown.borrow() { return; }
                sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        let mut child = child;

        if !wait_port(t.local_port).await {
            warn!("SSH tunnel port {} did not come up — retrying", t.local_port);
            let _ = child.kill().await;
            if *shutdown.borrow() { return; }
            sleep(Duration::from_secs(5)).await;
            continue;
        }

        info!("SSH tunnel up on 127.0.0.1:{}", t.local_port);

        tokio::select! {
            status = child.wait() => {
                match status {
                    Ok(s) => warn!("SSH tunnel exited: {}", s),
                    Err(e) => warn!("SSH tunnel wait error: {}", e),
                }
            }
            _ = shutdown.changed() => {
                let _ = child.kill().await;
                info!("SSH tunnel stopped");
                return;
            }
        }

        if *shutdown.borrow() { return; }
        warn!("SSH tunnel died — restarting in 5s");
        sleep(Duration::from_secs(5)).await;
    }
}

fn spawn_ssh(t: &SshTunnelConfig) -> std::io::Result<tokio::process::Child> {
    let mut cmd = Command::new("ssh");
    cmd.args([
        "-D", &format!("127.0.0.1:{}", t.local_port),
        "-N",
        "-o", "ExitOnForwardFailure=yes",
        "-o", "ServerAliveInterval=30",
        "-o", "ServerAliveCountMax=3",
        "-o", "ConnectTimeout=10",
        "-o", "StrictHostKeyChecking=accept-new",
        "-p", &t.ssh_port.to_string(),
        &format!("{}@{}", t.user, t.host),
    ]);

    if let Some(password) = &t.password {
        // Write a temp askpass script that echoes the password.
        // SSH calls SSH_ASKPASS when it needs a passphrase and no tty is attached.
        let script = askpass_script(password)?;
        cmd.env("SSH_ASKPASS", &script);
        cmd.env("SSH_ASKPASS_REQUIRE", "force");
        // stdin must not be a tty for SSH_ASKPASS to be invoked
        cmd.stdin(std::process::Stdio::null());
    } else {
        cmd.args(["-o", "BatchMode=yes"]);
    }

    cmd.kill_on_drop(true).spawn()
}

fn askpass_script(password: &str) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join("talpa-askpass.sh");
    // Safely embed password: replace single-quotes so the shell can't break out
    let safe = password.replace('\'', "'\\''");
    std::fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", safe))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

async fn wait_port(port: u16) -> bool {
    for _ in 0..30 {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return true;
        }
        sleep(Duration::from_millis(500)).await;
    }
    false
}
