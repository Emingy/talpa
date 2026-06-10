use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex, OnceLock, RwLock};

// The live config, swappable so the UI can reload from disk at runtime.
static CONFIG: OnceLock<RwLock<Arc<Config>>> = OnceLock::new();

// Runtime overlay: masks the UI has toggled off. Independent of the YAML file
// so a reload starts from a clean slate (see `Config::reload`).
static DISABLED_DOMAINS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn slot() -> &'static RwLock<Arc<Config>> {
    CONFIG.get().expect("config not loaded; call Config::load() first")
}

/// Returns a snapshot of the loaded configuration. Cheap (an `Arc` clone).
/// Panics if [`Config::load`] was not called first.
pub fn config() -> Arc<Config> {
    slot().read().unwrap().clone()
}

/// Whether a domain mask is currently active. The UI can toggle masks off at
/// runtime without rewriting the YAML file.
pub fn domain_enabled(mask: &str) -> bool {
    !DISABLED_DOMAINS.lock().unwrap().contains(mask)
}

/// Toggle a domain mask on/off at runtime.
pub fn set_domain_enabled(mask: &str, enabled: bool) {
    let mut set = DISABLED_DOMAINS.lock().unwrap();
    if enabled {
        set.remove(mask);
    } else {
        set.insert(mask.to_string());
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub ssh: Ssh,
    pub socks: Socks,
    pub tun: Tun,
    pub dns: Dns,
    /// Domain masks routed through the proxy. See `dns::utils` for syntax.
    pub domains: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ssh {
    /// `user@host` passed to ssh.
    pub target: String,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Socks {
    /// Local port the ssh `-D` SOCKS5 proxy listens on.
    pub port: u16,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tun {
    /// Address assigned to the TUN interface (e.g. `10.0.0.2`).
    pub address: String,
    /// Gateway / peer address used as the route target (e.g. `10.0.0.1`).
    pub gateway: String,
    pub netmask: String,
    pub mtu: u16,
    /// Floor for DNS-derived route TTLs, in seconds.
    pub min_ttl_secs: u64,
    /// Always-present routes. Single IP (`1.2.3.4`) or CIDR (`1.2.3.0/24`).
    pub static_routes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Dns {
    /// Address the local DNS proxy binds to (e.g. `127.0.0.1:53`).
    pub listen_addr: String,
    /// Upstream resolver for non-matched domains.
    pub upstream_addr: String,
    /// Upstream resolver reached over SOCKS for matched domains.
    pub upstream_socks_addr: String,
}

impl Socks {
    /// `127.0.0.1:<port>` — the address clients use to reach the SOCKS proxy.
    pub fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
}

impl Config {
    /// Reads and parses config from `path`, falling back to built-in defaults
    /// when the file is absent.
    fn read_from(path: &Path) -> Result<Config> {
        if path.exists() {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read config {}", path.display()))?;
            serde_yaml::from_str(&raw)
                .with_context(|| format!("failed to parse config {}", path.display()))
        } else {
            log::info!("[CONFIG] {} not found, using defaults", path.display());
            Ok(Config::default())
        }
    }

    /// Loads config into the process-global slot. Call once at startup.
    pub fn load(path: impl AsRef<Path>) -> Result<()> {
        let cfg = Self::read_from(path.as_ref())?;
        CONFIG.get_or_init(|| RwLock::new(Arc::new(cfg)));
        Ok(())
    }

    /// Re-reads config from `path` and swaps it in, clearing runtime domain
    /// toggles so the on-disk state becomes authoritative again.
    pub fn reload(path: impl AsRef<Path>) -> Result<()> {
        let cfg = Self::read_from(path.as_ref())?;
        *slot().write().unwrap() = Arc::new(cfg);
        DISABLED_DOMAINS.lock().unwrap().clear();
        Ok(())
    }
}

impl Default for Ssh {
    fn default() -> Self {
        Self { target: "user@192.168.0.1".into() }
    }
}

impl Default for Socks {
    fn default() -> Self {
        Self { port: 1080 }
    }
}

impl Default for Tun {
    fn default() -> Self {
        Self {
            address: "10.0.0.2".into(),
            gateway: "10.0.0.1".into(),
            netmask: "255.255.255.0".into(),
            mtu: 1500,
            min_ttl_secs: 60,
            static_routes: vec![],
        }
    }
}

impl Default for Dns {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:53".into(),
            upstream_addr: "1.1.1.1:53".into(),
            upstream_socks_addr: "8.8.8.8:53".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_parses() {
        let raw =
            std::fs::read_to_string("config.example.yml").expect("config.example.yml present");
        let cfg: Config = serde_yaml::from_str(&raw).expect("config.example.yml parses");
        assert_eq!(cfg.socks.addr(), format!("127.0.0.1:{}", cfg.socks.port));
        assert!(!cfg.domains.is_empty());
    }

    #[test]
    fn empty_yaml_yields_defaults() {
        let cfg: Config = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.socks.port, 1080);
        assert_eq!(cfg.tun.gateway, "10.0.0.1");
    }
}
