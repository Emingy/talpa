use std::net::IpAddr;
use ipnet::IpNet;

/// Combined matcher for domain patterns and IP/subnet rules.
pub struct Matcher {
    domains: Vec<DomainPattern>,
    nets: Vec<IpNet>,
}

impl Matcher {
    pub fn new(domains: &[String], ips: &[String]) -> Self {
        Self {
            domains: domains.iter().map(|s| DomainPattern::parse(s)).collect(),
            nets: ips.iter().filter_map(|s| parse_ip_rule(s)).collect(),
        }
    }

    pub fn matches_domain(&self, domain: &str) -> bool {
        let domain = domain.trim_end_matches('.');
        self.domains.iter().any(|p| p.matches(domain))
    }

    pub fn matches_ip(&self, addr: IpAddr) -> bool {
        self.nets.iter().any(|net| net.contains(&addr))
    }
}

/// Parse "1.2.3.4", "1.2.3.4/24", "::1", "fd00::/8" etc.
fn parse_ip_rule(s: &str) -> Option<IpNet> {
    // Try CIDR first
    if let Ok(net) = s.parse::<IpNet>() {
        return Some(net);
    }
    // Try bare IP → /32 or /128
    if let Ok(addr) = s.parse::<IpAddr>() {
        return Some(IpNet::from(addr)); // IpNet::from(IpAddr) creates /32 or /128
    }
    None
}

// ── Domain pattern matching ───────────────────────────────────────────────────

enum DomainPattern {
    Exact(String),
    Single(String),  // *.example.com  → ".example.com" suffix, no extra dots
    Deep(String),    // **.example.com → any depth + exact
}

impl DomainPattern {
    fn parse(s: &str) -> Self {
        if let Some(rest) = s.strip_prefix("**.") {
            DomainPattern::Deep(format!(".{}", rest.to_lowercase()))
        } else if let Some(rest) = s.strip_prefix("*.") {
            DomainPattern::Single(format!(".{}", rest.to_lowercase()))
        } else {
            DomainPattern::Exact(s.to_lowercase())
        }
    }

    fn matches(&self, domain: &str) -> bool {
        let d = domain.to_lowercase();
        match self {
            DomainPattern::Exact(e) => d == *e,
            DomainPattern::Single(suffix) => {
                d.strip_suffix(suffix.as_str())
                    .map(|prefix| !prefix.is_empty() && !prefix.contains('.'))
                    .unwrap_or(false)
            }
            DomainPattern::Deep(suffix) => {
                d == suffix.trim_start_matches('.') || d.ends_suffix(suffix)
            }
        }
    }
}

trait EndsSuffix {
    fn ends_suffix(&self, s: &str) -> bool;
}
impl EndsSuffix for str {
    fn ends_suffix(&self, s: &str) -> bool { self.ends_with(s) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_single_level() {
        let m = Matcher::new(&["*.acme.test".into()], &[]);
        assert!(m.matches_domain("api.acme.test"));
        assert!(!m.matches_domain("sub.api.acme.test"));
        assert!(!m.matches_domain("acme.test"));
    }

    #[test]
    fn domain_deep() {
        let m = Matcher::new(&["**.acme.test".into()], &[]);
        assert!(m.matches_domain("acme.test"));
        assert!(m.matches_domain("a.acme.test"));
        assert!(m.matches_domain("a.b.acme.test"));
        assert!(!m.matches_domain("notacme.test"));
    }

    #[test]
    fn domain_exact() {
        let m = Matcher::new(&["exact.com".into()], &[]);
        assert!(m.matches_domain("exact.com"));
        assert!(!m.matches_domain("www.exact.com"));
        assert!(!m.matches_domain("notexact.com"));
    }

    #[test]
    fn domain_case_insensitive() {
        let m = Matcher::new(&["*.Example.COM".into()], &[]);
        assert!(m.matches_domain("API.EXAMPLE.COM"));
        assert!(m.matches_domain("api.example.com"));
    }

    #[test]
    fn domain_trailing_dot_stripped() {
        let m = Matcher::new(&["**.example.com".into()], &[]);
        assert!(m.matches_domain("api.example.com."));
    }

    #[test]
    fn domain_empty_matcher() {
        let m = Matcher::new(&[], &[]);
        assert!(!m.matches_domain("anything.com"));
    }

    #[test]
    fn ip_subnet() {
        let m = Matcher::new(&[], &["10.0.0.0/8".into()]);
        assert!(m.matches_ip("10.5.6.7".parse().unwrap()));
        assert!(!m.matches_ip("172.16.0.1".parse().unwrap()));
    }

    #[test]
    fn ip_exact() {
        let m = Matcher::new(&[], &["192.168.1.100".into()]);
        assert!(m.matches_ip("192.168.1.100".parse().unwrap()));
        assert!(!m.matches_ip("192.168.1.101".parse().unwrap()));
    }

    #[test]
    fn ip_ipv6() {
        let m = Matcher::new(&[], &["fd00::/8".into(), "::1".into()]);
        assert!(m.matches_ip("fd12:3456::1".parse().unwrap()));
        assert!(m.matches_ip("::1".parse().unwrap()));
        assert!(!m.matches_ip("2001:db8::1".parse().unwrap()));
    }

    #[test]
    fn ip_invalid_entries_ignored() {
        let m = Matcher::new(&[], &["not_an_ip".into(), "10.0.0.0/8".into()]);
        assert!(m.matches_ip("10.1.2.3".parse().unwrap()));
    }
}
