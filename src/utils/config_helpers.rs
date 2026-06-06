use crate::config::{Config, DnsConfig, SshTunnelConfig, SystemProxyConfig};

pub fn ensure_tunnel(c: &mut Config) -> &mut SshTunnelConfig {
    if c.ssh_tunnel.is_none() {
        c.ssh_tunnel = Some(SshTunnelConfig {
            user: "user".into(),
            host: "ssh.example.com".into(),
            ssh_port: 22,
            local_port: 10808,
            password: None,
        });
    }
    c.ssh_tunnel.as_mut().unwrap()
}

pub fn ensure_sp(c: &mut Config) -> &mut SystemProxyConfig {
    if c.system_proxy.is_none() {
        c.system_proxy = Some(SystemProxyConfig::default());
    }
    c.system_proxy.as_mut().unwrap()
}

pub fn ensure_dns(c: &mut Config) -> &mut DnsConfig {
    if c.dns.is_none() {
        c.dns = Some(DnsConfig::default());
    }
    c.dns.as_mut().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ListenConfig, UpstreamConfig};

    fn minimal_config() -> Config {
        Config {
            domains: vec![],
            ips: vec![],
            listen: ListenConfig { addr: "127.0.0.1".into(), port: 1080 },
            upstream: UpstreamConfig { addr: "127.0.0.1".into(), port: 10808 },
            ssh_tunnel: None,
            system_proxy: None,
            dns: None,
        }
    }

    #[test]
    fn ensure_tunnel_creates_when_absent() {
        let mut c = minimal_config();
        let t = ensure_tunnel(&mut c);
        assert_eq!(t.ssh_port, 22);
        assert_eq!(t.local_port, 10808);
        assert!(t.password.is_none());
    }

    #[test]
    fn ensure_tunnel_preserves_existing() {
        let mut c = minimal_config();
        c.ssh_tunnel = Some(SshTunnelConfig {
            user: "alice".into(),
            host: "proxy.test".into(),
            ssh_port: 2222,
            local_port: 9999,
            password: None,
        });
        let t = ensure_tunnel(&mut c);
        assert_eq!(t.host, "proxy.test");
        assert_eq!(t.ssh_port, 2222);
    }

    #[test]
    fn ensure_sp_creates_when_absent() {
        let mut c = minimal_config();
        let sp = ensure_sp(&mut c);
        assert!(sp.enabled);
        assert!(sp.configure_npm);
    }

    #[test]
    fn ensure_sp_preserves_existing() {
        let mut c = minimal_config();
        c.system_proxy = Some(SystemProxyConfig {
            enabled: false,
            configure_npm: false,
            configure_git: false,
            configure_curl: false,
        });
        let sp = ensure_sp(&mut c);
        assert!(!sp.enabled);
        assert!(!sp.configure_npm);
    }

    #[test]
    fn ensure_dns_creates_when_absent() {
        let mut c = minimal_config();
        let d = ensure_dns(&mut c);
        assert_eq!(d.listen_port, 5300);
    }

    #[test]
    fn ensure_dns_preserves_existing() {
        let mut c = minimal_config();
        c.dns = Some(DnsConfig {
            listen_port: 1053,
            upstream_dns: "1.1.1.1:53".into(),
            fallback_dns: "9.9.9.9:53".into(),
        });
        let d = ensure_dns(&mut c);
        assert_eq!(d.listen_port, 1053);
        assert_eq!(d.upstream_dns, "1.1.1.1:53");
    }
}
