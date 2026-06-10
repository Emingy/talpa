//! Windows backend: PowerShell `*-NetRoute` cmdlets for routing, NRPT rules
//! (`*-DnsClientNrptRule`) for split-DNS, `Clear-DnsClientCache`, `taskkill`,
//! `cmd /c start` for opening files.
//!
//! Requires running as Administrator. `wintun.dll` is embedded in the binary
//! (see `WINTUN_DLL` / `ensure_wintun`) and extracted at startup, so no separate
//! DLL needs to be shipped.

use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::process::Command;

use super::{mask_base, normalize_prefix, DnsConfigurator, ProcessControl, RouteManager, ShellOpen, TunHandle};
use crate::config;

/// The wintun driver DLL, embedded at compile time. Place the official x64
/// `wintun.dll` (https://www.wintun.net) at `assets/wintun.dll` before building
/// for Windows.
const WINTUN_DLL: &[u8] = include_bytes!("../../assets/wintun.dll");

/// Extracts the embedded `wintun.dll` to a temp file and returns its path, so
/// the `tun` crate can load it (`platform_config().wintun_file`). Re-uses an
/// already-extracted copy of the same size to avoid clobbering a loaded DLL.
fn ensure_wintun() -> PathBuf {
    let dir = std::env::temp_dir().join("talpa");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("wintun.dll");
    let up_to_date = std::fs::metadata(&path)
        .map(|m| m.len() == WINTUN_DLL.len() as u64)
        .unwrap_or(false);
    if !up_to_date {
        if let Err(e) = std::fs::write(&path, WINTUN_DLL) {
            log::error!("[TUN] failed to extract wintun.dll: {e}");
        }
    }
    path
}

pub struct Windows;

/// Runs a PowerShell one-liner, returning its captured stdout on success.
async fn ps(script: &str) -> Result<std::process::Output> {
    super::quiet(
        Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null()),
    )
    .output()
    .await
    .context("failed to run powershell")
}

impl RouteManager for Windows {
    async fn add_route(tun: &TunHandle, dst: &str) -> Result<()> {
        if Self::route_present(tun, dst).await {
            return Ok(());
        }
        let prefix = normalize_prefix(dst);
        let out = ps(&format!(
            "New-NetRoute -DestinationPrefix '{prefix}' -InterfaceAlias '{}' \
             -NextHop '{}' -PolicyStore ActiveStore -ErrorAction Stop",
            tun.name, tun.gateway
        ))
        .await?;
        if !out.status.success() {
            anyhow::bail!("New-NetRoute failed for {dst}");
        }
        Ok(())
    }

    async fn del_route(dst: &str) -> Result<()> {
        let prefix = normalize_prefix(dst);
        ps(&format!(
            "Remove-NetRoute -DestinationPrefix '{prefix}' -Confirm:$false \
             -ErrorAction SilentlyContinue"
        ))
        .await?;
        Ok(())
    }

    async fn route_present(tun: &TunHandle, dst: &str) -> bool {
        let prefix = normalize_prefix(dst);
        ps(&format!(
            "Get-NetRoute -DestinationPrefix '{prefix}' -InterfaceAlias '{}' \
             -ErrorAction SilentlyContinue | Select-Object -First 1",
            tun.name
        ))
        .await
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
    }

    fn configure_tun(c: &mut tun::Configuration, tun: &config::Tun) {
        // Point the tun crate at our extracted copy of the embedded wintun.dll.
        let dll = ensure_wintun();
        c.platform_config(move |p| {
            p.wintun_file(dll);
        });
        // wintun adapter named so routing/DNS can target it by alias.
        c.tun_name("talpa")
            .address(&tun.address)
            .netmask(&tun.netmask)
            .mtu(tun.mtu);
    }
}

impl DnsConfigurator for Windows {
    /// Adds an NRPT rule per enabled mask so Windows resolves those namespaces
    /// via our local server (port 53). Clears stale rules first.
    async fn apply(tun: Option<&TunHandle>, listen_addr: &str, masks: &[String]) -> Result<()> {
        Self::clear(tun, masks).await;
        let ip = listen_addr.rsplit_once(':').map(|(h, _)| h).unwrap_or("127.0.0.1");
        for mask in masks {
            let ns = format!(".{}", mask_base(mask));
            let out = ps(&format!(
                "Add-DnsClientNrptRule -Namespace '{ns}' -NameServers '{ip}' -ErrorAction Stop"
            ))
            .await?;
            if out.status.success() {
                log::info!("[DNS] NRPT rule: {ns} -> {ip}");
            } else {
                log::error!("[DNS] failed to add NRPT rule for {ns}");
            }
        }
        Ok(())
    }

    async fn clear(_tun: Option<&TunHandle>, masks: &[String]) {
        for mask in masks {
            let ns = format!(".{}", mask_base(mask));
            let _ = ps(&format!(
                "Get-DnsClientNrptRule | Where-Object {{ $_.Namespace -eq '{ns}' }} | \
                 Remove-DnsClientNrptRule -Force -ErrorAction SilentlyContinue"
            ))
            .await;
        }
        log::info!("[DNS] NRPT rules removed");
    }

    async fn flush_cache() {
        let _ = ps("Clear-DnsClientCache").await;
        log::info!("[DNS] cache flushed");
    }
}

impl ProcessControl for Windows {
    async fn terminate_pid(pid: u32) {
        let _ = super::quiet(Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]))
            .status()
            .await;
    }
}

impl ShellOpen for Windows {
    fn open_path(path: &str) -> Result<()> {
        Self::shell_start(path)
    }

    fn open_url(url: &str) -> Result<()> {
        // `start` dispatches URLs to the default browser, same as file paths.
        Self::shell_start(url)
    }
}

impl Windows {
    /// `cmd /C start "" <target>` — opens a path or URL in its default handler.
    fn shell_start(target: &str) -> Result<()> {
        use std::os::windows::process::CommandExt;
        // `start` is a cmd builtin; the empty "" is the window-title argument.
        // CREATE_NO_WINDOW (0x0800_0000) keeps the cmd console from flashing.
        std::process::Command::new("cmd")
            .args(["/C", "start", "", target])
            .creation_flags(0x0800_0000)
            .spawn()?;
        Ok(())
    }
}
