//! Linux backend: iproute2 (`ip route`), systemd-resolved split-DNS
//! (`resolvectl`), `kill`, `xdg-open`.
//!
//! Split-DNS requires `systemd-resolved`. Routing attaches host/CIDR routes to
//! the TUN device by name.

use anyhow::{Context, Result};
use tokio::process::Command;

use super::{mask_base, normalize_prefix, DnsConfigurator, ProcessControl, RouteManager, ShellOpen, TunHandle};
use crate::config;

pub struct Linux;

impl RouteManager for Linux {
    async fn add_route(tun: &TunHandle, dst: &str) -> Result<()> {
        let prefix = normalize_prefix(dst);
        // `replace` is idempotent: adds if missing, updates if present — also
        // covers re-adding a route a VPN flushed.
        let status = Command::new("ip")
            .args(["route", "replace", &prefix, "via", &tun.gateway, "dev", &tun.name])
            .status()
            .await
            .context("failed to run ip route replace")?;
        if !status.success() {
            anyhow::bail!("ip route replace failed for {dst}");
        }
        Ok(())
    }

    async fn del_route(dst: &str) -> Result<()> {
        let prefix = normalize_prefix(dst);
        Command::new("ip")
            .args(["route", "del", &prefix])
            .status()
            .await
            .context("failed to run ip route del")?;
        Ok(())
    }

    async fn route_present(tun: &TunHandle, dst: &str) -> bool {
        let prefix = normalize_prefix(dst);
        Command::new("ip")
            .args(["-o", "route", "show", &prefix])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .await
            .map(|o| {
                let out = String::from_utf8_lossy(&o.stdout);
                o.status.success() && out.contains(&format!("dev {}", tun.name))
            })
            .unwrap_or(false)
    }

    fn configure_tun(c: &mut tun::Configuration, tun: &config::Tun) {
        // Linux TUN: a deterministic name lets routing/DNS target it; bring it
        // up immediately. No peer address. PI defaults to false (IFF_NO_PI).
        c.tun_name("talpa")
            .address(&tun.address)
            .netmask(&tun.netmask)
            .mtu(tun.mtu)
            .up();
    }
}

impl DnsConfigurator for Linux {
    /// Routes the enabled masks to `listen_addr` via the TUN link using
    /// systemd-resolved: `resolvectl dns <link> <ip>` plus routing-only domains
    /// (`~domain`). Reverting the link clears any previously-set masks.
    async fn apply(tun: Option<&TunHandle>, listen_addr: &str, masks: &[String]) -> Result<()> {
        let link = tun.map(|t| t.name.as_str()).context("no TUN link for DNS config")?;
        let ip = listen_addr.rsplit_once(':').map(|(h, _)| h).unwrap_or("127.0.0.1");

        // Start clean so disabled masks don't linger on the link.
        let _ = Command::new("resolvectl").args(["revert", link]).status().await;

        if masks.is_empty() {
            return Ok(());
        }

        let status = Command::new("resolvectl")
            .args(["dns", link, ip])
            .status()
            .await
            .context("failed to run resolvectl dns")?;
        if !status.success() {
            anyhow::bail!("resolvectl dns failed for {link}");
        }

        // `~domain` = routing-only (not a search domain).
        let mut args = vec!["domain".to_string(), link.to_string()];
        for mask in masks {
            args.push(format!("~{}", mask_base(mask)));
        }
        let status = Command::new("resolvectl").args(&args).status().await
            .context("failed to run resolvectl domain")?;
        if !status.success() {
            anyhow::bail!("resolvectl domain failed for {link}");
        }
        log::info!("[DNS] systemd-resolved routing {} domain(s) via {link}", masks.len());
        Ok(())
    }

    async fn clear(tun: Option<&TunHandle>, _masks: &[String]) {
        if let Some(t) = tun {
            let _ = Command::new("resolvectl").args(["revert", &t.name]).status().await;
            log::info!("[DNS] systemd-resolved link reverted: {}", t.name);
        }
    }

    async fn flush_cache() {
        let _ = Command::new("resolvectl").arg("flush-caches").status().await;
        log::info!("[DNS] cache flushed");
    }
}

impl ProcessControl for Linux {
    async fn terminate_pid(pid: u32) {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .await;
    }
}

impl ShellOpen for Linux {
    fn open_path(path: &str) -> Result<()> {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
        Ok(())
    }

    fn open_url(url: &str) -> Result<()> {
        // xdg-open handles URLs as well as file paths.
        std::process::Command::new("xdg-open").arg(url).spawn()?;
        Ok(())
    }
}
