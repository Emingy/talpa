//! Platform abstraction layer.
//!
//! Every OS-specific system operation (route table, split-DNS, DNS-cache flush,
//! process termination, opening a file, TUN configuration, tray indication) is
//! expressed here as a trait. Each supported OS provides one zero-sized backend
//! (`MacOs` / `Linux` / `Windows`) implementing those traits; the active backend
//! is selected at compile time and re-exported as [`Sys`]. The rest of the code
//! calls `platform::Sys::*` and never touches an OS command directly.
//!
//! Traits use native `async fn` in traits (Rust edition 2024) with static
//! dispatch through [`Sys`], so no `dyn`/`async-trait` machinery is needed.

use anyhow::Result;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::MacOs as Sys;
#[cfg(target_os = "linux")]
pub use linux::Linux as Sys;
#[cfg(target_os = "windows")]
pub use windows::Windows as Sys;

/// Identifies the live TUN interface for OS calls that need it (Linux routing &
/// split-DNS use the interface name; macOS routes via the peer `gateway`).
/// Built by `Tunnel::start` once the device exists.
#[derive(Clone, Debug)]
pub struct TunHandle {
    /// Interface name (e.g. `utun5` on macOS, `talpa` on Linux/Windows).
    pub name: String,
    /// Peer / gateway address used as the route target.
    pub gateway: String,
}

/// Route-table operations. `dst` is a single IP (`1.2.3.4`) or a CIDR
/// (`1.2.3.0/24`); backends normalise a bare IP to a host route themselves.
#[allow(async_fn_in_trait)]
pub trait RouteManager {
    /// Add (or refresh) a route for `dst` through the tunnel. Idempotent.
    async fn add_route(tun: &TunHandle, dst: &str) -> Result<()>;
    /// Remove the route for `dst`. Best-effort.
    async fn del_route(dst: &str) -> Result<()>;
    /// Whether a route for `dst` currently points through the tunnel.
    async fn route_present(tun: &TunHandle, dst: &str) -> bool;

    /// Fill in the OS-specific bits of the TUN [`tun::Configuration`].
    fn configure_tun(c: &mut tun::Configuration, tun: &crate::config::Tun);
    /// Whether `ipstack` should do its own 4-byte PI handling. The `tun` crate
    /// already normalises this on all current targets, so it is `false`.
    fn ipstack_packet_information() -> bool {
        false
    }
}

/// System split-DNS configuration: make the OS resolve only the configured
/// (enabled) domain masks via our local server, surgically and reversibly.
#[allow(async_fn_in_trait)]
pub trait DnsConfigurator {
    /// Point resolution of `masks` at `listen_addr` and clear it for any mask
    /// not listed. `tun` is the live interface (needed on Linux).
    async fn apply(tun: Option<&TunHandle>, listen_addr: &str, masks: &[String]) -> Result<()>;
    /// Undo everything `apply` set up for `masks`.
    async fn clear(tun: Option<&TunHandle>, masks: &[String]);
    /// Flush the OS DNS resolver cache.
    async fn flush_cache();
}

/// Process control for the spawned ssh proxy.
#[allow(async_fn_in_trait)]
pub trait ProcessControl {
    /// Terminate the process with the given PID (graceful where possible).
    async fn terminate_pid(pid: u32);
}

/// Open a path in the user's default handler (config editor).
pub trait ShellOpen {
    fn open_path(path: &str) -> Result<()>;
    /// Open a URL in the user's default web browser.
    fn open_url(url: &str) -> Result<()>;
}

/// Windows `CreateProcess` flag that suppresses a console window for a child
/// process. Without it, every helper we spawn (powershell, ssh, taskkill, …)
/// flashes a console window because the app itself is a GUI binary with no
/// console (`windows_subsystem = "windows"`).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Suppresses the child's console window on Windows; no-op on other platforms.
/// Wrap a built `tokio::process::Command` before `spawn`/`status`/`output`.
pub fn quiet(cmd: &mut tokio::process::Command) -> &mut tokio::process::Command {
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Extracts the base domain from a mask (`**.example.com` / `*.example.com`
/// → `example.com`). Shared by the DNS backends.
pub fn mask_base(mask: &str) -> &str {
    mask.trim_start_matches("**.").trim_start_matches("*.")
}

/// Normalises a route destination to an explicit prefix: a bare IP becomes a
/// `/32` host route; an existing CIDR is returned unchanged. Used by the Linux
/// and Windows backends.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn normalize_prefix(dst: &str) -> std::borrow::Cow<'_, str> {
    if dst.contains('/') {
        std::borrow::Cow::Borrowed(dst)
    } else {
        std::borrow::Cow::Owned(format!("{dst}/32"))
    }
}
