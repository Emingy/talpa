use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub domains: Vec<String>,
    #[serde(default)]
    pub ips: Vec<String>,
    pub listen: ListenConfig,
    pub upstream: UpstreamConfig,
    pub ssh_tunnel: Option<SshTunnelConfig>,
    pub system_proxy: Option<SystemProxyConfig>,
    pub dns: Option<DnsConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ListenConfig {
    pub addr: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UpstreamConfig {
    pub addr: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SshTunnelConfig {
    pub user: String,
    pub host: String,
    pub ssh_port: u16,
    pub local_port: u16,
    /// Plain-text SSH password. If absent, key-based auth is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SystemProxyConfig {
    pub enabled: bool,
    #[serde(default = "bool_true")]
    pub configure_npm: bool,
    #[serde(default = "bool_true")]
    pub configure_git: bool,
    #[serde(default = "bool_true")]
    pub configure_curl: bool,
}

impl Default for SystemProxyConfig {
    fn default() -> Self {
        Self { enabled: true, configure_npm: true, configure_git: true, configure_curl: true }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DnsConfig {
    pub listen_port: u16,
    pub upstream_dns: String,
    pub fallback_dns: String,
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            listen_port: 5300,
            upstream_dns: "8.8.8.8:53".into(),
            fallback_dns: "8.8.8.8:53".into(),
        }
    }
}

fn bool_true() -> bool { true }

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_toml() -> &'static str {
        r#"
        domains = ["**.example.com"]
        ips = []
        [listen]
        addr = "127.0.0.1"
        port = 1080
        [upstream]
        addr = "127.0.0.1"
        port = 10808
        "#
    }

    #[test]
    fn parse_minimal() {
        let c: Config = toml::from_str(minimal_toml()).unwrap();
        assert_eq!(c.domains, vec!["**.example.com"]);
        assert_eq!(c.listen.port, 1080);
        assert_eq!(c.upstream.port, 10808);
        assert!(c.ssh_tunnel.is_none());
        assert!(c.system_proxy.is_none());
        assert!(c.dns.is_none());
    }

    #[test]
    fn ips_default_empty() {
        let toml = r#"
        domains = []
        [listen]
        addr = "127.0.0.1"
        port = 1080
        [upstream]
        addr = "127.0.0.1"
        port = 10808
        "#;
        let c: Config = toml::from_str(toml).unwrap();
        assert!(c.ips.is_empty());
    }

    #[test]
    fn system_proxy_defaults() {
        let sp = SystemProxyConfig::default();
        assert!(sp.enabled);
        assert!(sp.configure_npm);
        assert!(sp.configure_git);
        assert!(sp.configure_curl);
    }

    #[test]
    fn dns_defaults() {
        let dns = DnsConfig::default();
        assert_eq!(dns.listen_port, 5300);
        assert_eq!(dns.upstream_dns, "8.8.8.8:53");
        assert_eq!(dns.fallback_dns, "8.8.8.8:53");
    }

    #[test]
    fn roundtrip() {
        let c: Config = toml::from_str(minimal_toml()).unwrap();
        let serialized = toml::to_string_pretty(&c).unwrap();
        let c2: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(c.domains, c2.domains);
        assert_eq!(c.listen.addr, c2.listen.addr);
        assert_eq!(c.listen.port, c2.listen.port);
        assert_eq!(c.upstream.addr, c2.upstream.addr);
        assert_eq!(c.upstream.port, c2.upstream.port);
    }

    #[test]
    fn ssh_password_skipped_when_none() {
        let c: Config = toml::from_str(minimal_toml()).unwrap();
        let serialized = toml::to_string_pretty(&c).unwrap();
        assert!(!serialized.contains("password"));
    }
}

impl Config {
    pub fn load(path: &std::path::Path) -> Self {
        use tracing::error;
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => { error!("cannot read {}: {}", path.display(), e); std::process::exit(1); }
        };
        match toml::from_str(&raw) {
            Ok(c) => c,
            Err(e) => { error!("config parse error: {}", e); std::process::exit(1); }
        }
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let s = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, s)
    }
}
