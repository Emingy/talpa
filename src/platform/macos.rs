//! macOS backend: BSD `route`, `/etc/resolver` split-DNS, `dscacheutil`/
//! `mDNSResponder` cache flush, `kill`, `open`.

use anyhow::{Context, Result};
use tokio::process::Command;

use super::{mask_base, DnsConfigurator, ProcessControl, RouteManager, ShellOpen, TunHandle};
use crate::config;

pub struct MacOs;

/// `-host` for a single IP, `-net` for a CIDR subnet (`a.b.c.d/n`).
fn route_flag(dst: &str) -> &'static str {
    if dst.contains('/') { "-net" } else { "-host" }
}

/// Destination usable with `route get` — strips the CIDR suffix.
fn probe_dst(dst: &str) -> &str {
    dst.split('/').next().unwrap_or(dst)
}

impl RouteManager for MacOs {
    async fn add_route(tun: &TunHandle, dst: &str) -> Result<()> {
        // Skip if it already points through us, so we don't log "File exists".
        if Self::route_present(tun, dst).await {
            return Ok(());
        }
        let status = Command::new("route")
            .args(["add", route_flag(dst), dst, &tun.gateway])
            .status()
            .await
            .context("failed to run route add")?;
        if !status.success() {
            anyhow::bail!("route add failed for {dst}");
        }
        Ok(())
    }

    async fn del_route(dst: &str) -> Result<()> {
        Command::new("route")
            .args(["delete", route_flag(dst), dst])
            .status()
            .await
            .context("failed to run route delete")?;
        Ok(())
    }

    async fn route_present(tun: &TunHandle, dst: &str) -> bool {
        Command::new("route")
            .args(["get", route_flag(dst), probe_dst(dst)])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .await
            .map(|o| {
                o.status.success()
                    && String::from_utf8_lossy(&o.stdout)
                        .contains(&format!("gateway: {}", tun.gateway))
            })
            .unwrap_or(false)
    }

    fn configure_tun(c: &mut tun::Configuration, tun: &config::Tun) {
        // utun on macOS uses a peer (destination) address; the route target is
        // that gateway. The crate strips/adds the 4-byte AF header itself
        // (packet_information defaults to true), so ipstack PI stays false.
        c.address(&tun.address)
            .netmask(&tun.netmask)
            .destination(&tun.gateway)
            .mtu(tun.mtu);
    }
}

impl DnsConfigurator for MacOs {
    /// Writes a `/etc/resolver/<base>` file per enabled mask so macOS resolves
    /// those domains via us, and removes the file for disabled ones.
    async fn apply(_tun: Option<&TunHandle>, listen_addr: &str, masks: &[String]) -> Result<()> {
        let _ = tokio::fs::create_dir_all("/etc/resolver").await;
        let ip = listen_addr.rsplit_once(':').map(|(h, _)| h).unwrap_or("127.0.0.1");
        for mask in masks {
            let path = format!("/etc/resolver/{}", mask_base(mask));
            match tokio::fs::write(&path, format!("nameserver {ip}\n")).await {
                Ok(_) => log::info!("[DNS] resolver file: {path}"),
                Err(e) => log::error!("[DNS] failed to write {path}: {e}"),
            }
        }
        Ok(())
    }

    async fn clear(_tun: Option<&TunHandle>, masks: &[String]) {
        for mask in masks {
            let _ = tokio::fs::remove_file(format!("/etc/resolver/{}", mask_base(mask))).await;
        }
        log::info!("[DNS] resolver files removed");
    }

    async fn flush_cache() {
        let _ = Command::new("dscacheutil").arg("-flushcache").status().await;
        let _ = Command::new("killall").args(["-HUP", "mDNSResponder"]).status().await;
        log::info!("[DNS] cache flushed");
    }
}

impl ProcessControl for MacOs {
    async fn terminate_pid(pid: u32) {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .await;
    }
}

impl ShellOpen for MacOs {
    fn open_path(path: &str) -> Result<()> {
        std::process::Command::new("open").args(["-t", path]).spawn()?;
        Ok(())
    }

    fn open_url(url: &str) -> Result<()> {
        // No `-t`: let the default browser handle the URL.
        std::process::Command::new("open").arg(url).spawn()?;
        Ok(())
    }
}
