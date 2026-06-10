use hickory_proto::{op::Message, rr::RData};
use std::net::IpAddr;

use crate::config::{config, domain_enabled};

pub fn match_domain(packet: &[u8]) -> bool {
    if let Ok(msg) = Message::from_vec(packet)
        && let Some(query) = msg.queries.first()
    {
        let raw = query.name().to_string();
        let domain = raw.trim_end_matches('.').to_ascii_lowercase();
        return config()
            .domains
            .iter()
            .any(|mask| domain_enabled(mask) && domain_matches(&domain, &mask.to_ascii_lowercase()));
    }
    false
}

fn domain_matches(domain: &str, mask: &str) -> bool {
    if let Some(base) = mask.strip_prefix("**.") {
        // any depth: base itself or anything.base
        domain == base || domain.ends_with(&format!(".{base}"))
    } else if let Some(base) = mask.strip_prefix("*.") {
        // exactly one level: <label>.base, no extra dots in the prefix
        if let Some(prefix) = domain.strip_suffix(&format!(".{base}")) {
            !prefix.contains('.')
        } else {
            false
        }
    } else {
        // exact match
        domain == mask
    }
}

pub fn get_addresses_from_response(resp: &[u8]) -> Vec<(IpAddr, u32)> {
    let mut addresses = vec![];

    if let Ok(reply) = Message::from_vec(resp) {
        for record in reply.answers {
            let ttl = record.ttl;
            match record.data {
                RData::A(ip) => addresses.push((IpAddr::V4(ip.0), ttl)),
                RData::AAAA(ip) => addresses.push((IpAddr::V6(ip.0), ttl)),
                _ => {}
            }
        }
    }

    addresses
}