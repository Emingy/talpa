use std::path::PathBuf;
use tokio::process::Command;
use tracing::{info, warn};

use crate::config::SystemProxyConfig;

const MARKER_BEGIN: &str = "# --- talpa BEGIN ---";
const MARKER_END: &str = "# --- talpa END ---";

// --- Tool-specific config files (take effect without terminal restart) ---

pub async fn configure_tools(cfg: &SystemProxyConfig, port: u16) {
    let url = format!("socks5h://127.0.0.1:{}", port);
    if cfg.configure_npm  { configure_npmrc(port); }
    if cfg.configure_curl { configure_curlrc(port); }
    if cfg.configure_git  { configure_git(&url).await; }
}

pub async fn unconfigure_tools(cfg: &SystemProxyConfig) {
    if cfg.configure_npm  { unconfigure_npmrc(); }
    if cfg.configure_curl { unconfigure_curlrc(); }
    if cfg.configure_git  { unconfigure_git().await; }
}

fn configure_npmrc(port: u16) {
    let path = match home_file(".npmrc") { Some(p) => p, None => return };
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    if content.contains(MARKER_BEGIN) { return; }
    let block = format!(
        "\n{b}\nproxy=socks5h://127.0.0.1:{p}\nhttps-proxy=socks5h://127.0.0.1:{p}\n{e}\n",
        b = MARKER_BEGIN, e = MARKER_END, p = port,
    );
    if std::fs::write(&path, format!("{}{}", content, block)).is_ok() {
        info!("~/.npmrc: proxy configured (effective immediately)");
    }
}

fn unconfigure_npmrc() {
    remove_from_file(home_file(".npmrc"), "~/.npmrc");
}

fn configure_curlrc(port: u16) {
    let path = match home_file(".curlrc") { Some(p) => p, None => return };
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    if content.contains(MARKER_BEGIN) { return; }
    let block = format!(
        "\n{b}\nsocks5-hostname=127.0.0.1:{p}\n{e}\n",
        b = MARKER_BEGIN, e = MARKER_END, p = port,
    );
    if std::fs::write(&path, format!("{}{}", content, block)).is_ok() {
        info!("~/.curlrc: proxy configured (effective immediately)");
    }
}

fn unconfigure_curlrc() {
    remove_from_file(home_file(".curlrc"), "~/.curlrc");
}

async fn configure_git(url: &str) {
    let ok = Command::new("git")
        .args(["config", "--global", "http.proxy", url])
        .status().await.map(|s| s.success()).unwrap_or(false)
        && Command::new("git")
        .args(["config", "--global", "https.proxy", url])
        .status().await.map(|s| s.success()).unwrap_or(false);
    if ok { info!("~/.gitconfig: http.proxy configured (effective immediately)"); }
}

async fn unconfigure_git() {
    let _ = Command::new("git").args(["config", "--global", "--unset", "http.proxy"]).status().await;
    let _ = Command::new("git").args(["config", "--global", "--unset", "https.proxy"]).status().await;
    info!("~/.gitconfig: http.proxy removed");
}

fn home_file(name: &str) -> Option<PathBuf> {
    std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(name))
}

fn remove_from_file(path: Option<PathBuf>, label: &str) {
    let path = match path { Some(p) => p, None => return };
    let content = match std::fs::read_to_string(&path) { Ok(c) => c, Err(_) => return };
    if !content.contains(MARKER_BEGIN) { return; }
    if std::fs::write(&path, remove_marked_block(&content)).is_ok() {
        info!("{}: proxy config removed", label);
    }
}

// --- launchctl: affects all new processes system-wide immediately ---

pub async fn launchctl_set(port: u16) {
    let val = format!("socks5h://127.0.0.1:{}", port);
    for var in ["ALL_PROXY", "all_proxy", "HTTPS_PROXY", "HTTP_PROXY"] {
        let _ = Command::new("launchctl")
            .args(["setenv", var, &val])
            .status().await;
    }
    info!("launchctl: ALL_PROXY set to {} (affects all new processes)", val);
}

pub async fn launchctl_unset() {
    for var in ["ALL_PROXY", "all_proxy", "HTTPS_PROXY", "HTTP_PROXY"] {
        let _ = Command::new("launchctl")
            .args(["unsetenv", var])
            .status().await;
    }
    info!("launchctl: ALL_PROXY unset");
}

// --- ~/.zshrc: persists across reboots, picked up by new terminal sessions ---

pub fn zshrc_add(port: u16) {
    let path = match zshrc_path() {
        Some(p) => p,
        None => return,
    };
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    if content.contains(MARKER_BEGIN) {
        return;
    }
    let block = format!(
        "\n{begin}\nexport ALL_PROXY=socks5h://127.0.0.1:{port}\nexport HTTPS_PROXY=socks5h://127.0.0.1:{port}\nexport HTTP_PROXY=socks5h://127.0.0.1:{port}\n{end}\n",
        begin = MARKER_BEGIN,
        end = MARKER_END,
        port = port,
    );
    if std::fs::write(&path, format!("{}{}", content, block)).is_ok() {
        info!("~/.zshrc: proxy env added (new terminal sessions will pick it up automatically)");
    }
}

pub fn zshrc_remove() {
    let path = match zshrc_path() {
        Some(p) => p,
        None => return,
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };
    if !content.contains(MARKER_BEGIN) {
        return;
    }
    if std::fs::write(&path, remove_marked_block(&content)).is_ok() {
        info!("~/.zshrc: proxy env removed");
    }
}

fn zshrc_path() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".zshrc"))
}

pub(crate) fn remove_marked_block(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut inside = false;
    for line in content.lines() {
        if line.trim() == MARKER_BEGIN { inside = true;  continue; }
        if line.trim() == MARKER_END   { inside = false; continue; }
        if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim_end_matches('\n').to_owned() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_block_from_middle() {
        let input = "before\n# --- talpa BEGIN ---\nproxy=socks5h://127.0.0.1:1080\n# --- talpa END ---\nafter\n";
        assert_eq!(remove_marked_block(input), "before\nafter\n");
    }

    #[test]
    fn removes_block_at_start() {
        let input = "# --- talpa BEGIN ---\nproxy=socks5h://127.0.0.1:1080\n# --- talpa END ---\nafter\n";
        assert_eq!(remove_marked_block(input), "after\n");
    }

    #[test]
    fn removes_block_at_end() {
        let input = "before\n# --- talpa BEGIN ---\nproxy=socks5h://127.0.0.1:1080\n# --- talpa END ---\n";
        assert_eq!(remove_marked_block(input), "before\n");
    }

    #[test]
    fn no_marker_unchanged() {
        let input = "no proxy config here\n";
        assert_eq!(remove_marked_block(input), "no proxy config here\n");
    }

    #[test]
    fn empty_file() {
        assert_eq!(remove_marked_block(""), "\n");
    }

    #[test]
    fn marker_with_surrounding_whitespace() {
        let input = "before\n  # --- talpa BEGIN ---  \ndata\n  # --- talpa END ---  \nafter\n";
        assert_eq!(remove_marked_block(input), "before\nafter\n");
    }
}

// --- macOS system SOCKS proxy (networksetup) ---

pub async fn enable(addr: &str, port: u16) {
    let services = hw_services().await;
    if services.is_empty() {
        warn!("system_proxy: no hardware network services found");
        return;
    }
    info!("Enabling system SOCKS5 proxy {}:{} on: {}", addr, port, services.join(", "));
    for svc in &services {
        let _ = Command::new("networksetup")
            .args(["-setsocksfirewallproxy", svc, addr, &port.to_string()])
            .status().await;
        let _ = Command::new("networksetup")
            .args(["-setsocksfirewallproxystate", svc, "on"])
            .status().await;
    }
}

pub async fn disable() {
    let services = hw_services().await;
    if services.is_empty() { return; }
    info!("Disabling system SOCKS5 proxy on: {}", services.join(", "));
    for svc in &services {
        let _ = Command::new("networksetup")
            .args(["-setsocksfirewallproxystate", svc, "off"])
            .status().await;
    }
}

async fn hw_services() -> Vec<String> {
    let hw_out = Command::new("networksetup")
        .arg("-listallhardwareports")
        .output().await
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();

    let hw_ports: Vec<String> = hw_out
        .lines()
        .filter_map(|l| l.strip_prefix("Hardware Port: ").map(str::to_owned))
        .collect();

    let svc_out = Command::new("networksetup")
        .arg("-listallnetworkservices")
        .output().await
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();

    svc_out
        .lines()
        .skip(1)
        .map(|l| l.trim_start_matches('*').trim().to_owned())
        .filter(|s| !s.is_empty() && hw_ports.contains(s))
        .collect()
}
