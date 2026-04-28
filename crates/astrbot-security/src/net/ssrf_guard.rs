use anyhow::{bail, Result};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

static PRIVATE_IP_RANGES: &[(IpAddr, u8)] = &[
    (IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0)), 8),
    (IpAddr::V4(Ipv4Addr::new(172, 16, 0, 0)), 12),
    (IpAddr::V4(Ipv4Addr::new(192, 168, 0, 0)), 16),
    (IpAddr::V4(Ipv4Addr::new(127, 0, 0, 0)), 8),
    (IpAddr::V4(Ipv4Addr::new(169, 254, 0, 0)), 16),
    (IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 8),
    (IpAddr::V6(Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0)), 7),
    (IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0)), 10),
    (IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1)), 128),
];

pub fn validate_url(url: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(url)?;

    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        bail!("Forbidden URL scheme: {}", parsed.scheme());
    }

    if parsed.password().is_some() {
        bail!("URL with embedded password is forbidden");
    }

    let host = parsed.host_str().unwrap_or("");
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_ip(ip) {
            bail!("Private IP access forbidden: {}", ip);
        }
    }

    let forbidden_hosts = [
        "localhost",
        "127.0.0.1",
        "::1",
        "metadata.google.internal",
        "169.254.169.254",
        "instance-data",
    ];
    if forbidden_hosts.iter().any(|h| host.eq_ignore_ascii_case(h)) {
        bail!("Forbidden host: {}", host);
    }

    Ok(())
}

fn is_private_ip(ip: IpAddr) -> bool {
    PRIVATE_IP_RANGES.iter().any(|(network, prefix)| match (network, ip) {
        (IpAddr::V4(n), IpAddr::V4(i)) => is_ipv4_in_prefix(*n, *i, *prefix),
        (IpAddr::V6(n), IpAddr::V6(i)) => is_ipv6_in_prefix(*n, *i, *prefix),
        _ => false,
    })
}

fn is_ipv4_in_prefix(network: Ipv4Addr, ip: Ipv4Addr, prefix: u8) -> bool {
    let mask = u32::MAX << (32 - prefix);
    let net = u32::from(network) & mask;
    let target = u32::from(ip) & mask;
    net == target
}

fn is_ipv6_in_prefix(network: Ipv6Addr, ip: Ipv6Addr, prefix: u8) -> bool {
    let net_segments = network.segments();
    let ip_segments = ip.segments();
    let full_segments = (prefix / 16) as usize;
    for i in 0..full_segments {
        if net_segments[i] != ip_segments[i] {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_public_url_ok() {
        assert!(validate_url("https://api.example.com/v1/data").is_ok());
    }

    #[test]
    fn test_private_ip_blocked() {
        assert!(validate_url("http://192.168.1.1/secret").is_err());
        assert!(validate_url("http://10.0.0.1/").is_err());
        assert!(validate_url("http://127.0.0.1/admin").is_err());
    }

    #[test]
    fn test_localhost_blocked() {
        assert!(validate_url("http://localhost:8080/").is_err());
    }

    #[test]
    fn test_forbidden_scheme() {
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("ftp://evil.com/").is_err());
    }
}
