//! Fail-closed SSRF guard for `plugin-web-search`.
//!
//! Validates scheme (`http`/`https` only), allowed port, hostname presence, and ALL
//! resolved [`IpAddr`]s via `std::net::ToSocketAddrs`, rejecting loopback, private,
//! CGNAT, link-local/metadata, `0.0.0.0/8`, multicast, broadcast, unspecified, reserved,
//! ULA, and IPv4-mapped IPv6 addresses resolving to blocked IPv4 ranges.

use core::fmt;
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};

use url::{Host, Url};

/// Errors produced when validating a target URL against SSRF policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsrfError {
    InvalidUrl(String),
    DisallowedScheme(String),
    MissingHost,
    DisallowedPort(u16),
    DnsResolutionFailed(String),
    BlockedAddress { host: String, ip: IpAddr },
}

impl fmt::Display for SsrfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(msg) => write!(f, "invalid URL: {msg}"),
            Self::DisallowedScheme(scheme) => {
                write!(f, "disallowed URL scheme '{scheme}': only http and https are allowed")
            }
            Self::MissingHost => write!(f, "URL must include a non-empty host"),
            Self::DisallowedPort(port) => write!(f, "port {port} is not in allow_ports"),
            Self::DnsResolutionFailed(msg) => write!(f, "cannot resolve host: {msg}"),
            Self::BlockedAddress { host, ip } => {
                write!(f, "blocked private, loopback, or reserved address {ip} for host '{host}'")
            }
        }
    }
}

impl std::error::Error for SsrfError {}

/// Validated target URL along with its normalized host, port, and checked socket addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedTarget {
    pub url: Url,
    pub host: String,
    pub port: u16,
    pub resolved_addrs: Vec<SocketAddr>,
}

/// DNS resolution abstraction so production uses [`std::net::ToSocketAddrs`] while
/// offline tests can simulate multi-IP, private-IP, or rebinding DNS answers.
pub trait DnsResolver: Send + Sync {
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, String>;
}

/// Standard DNS resolver powered by [`std::net::ToSocketAddrs`].
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemDnsResolver;

impl DnsResolver for SystemDnsResolver {
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
        let addrs: Vec<SocketAddr> = (host, port)
            .to_socket_addrs()
            .map_err(|e| format!("{host}: {e}"))?
            .collect();
        if addrs.is_empty() {
            return Err(format!("{host}: resolved zero addresses"));
        }
        Ok(addrs)
    }
}

/// Offline mock [`DnsResolver`] mapping hostnames to fixed IP lists.
#[derive(Debug, Default, Clone)]
pub struct MockDnsResolver {
    records: BTreeMap<String, Vec<IpAddr>>,
}

impl MockDnsResolver {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, host: impl Into<String>, ips: Vec<IpAddr>) {
        self.records.insert(host.into().to_ascii_lowercase(), ips);
    }
}

impl DnsResolver for MockDnsResolver {
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, port)]);
        }
        let key = host.to_ascii_lowercase();
        let Some(ips) = self.records.get(&key) else {
            return Err(format!("mock DNS has no record for '{host}'"));
        };
        if ips.is_empty() {
            return Err(format!("mock DNS returned empty record for '{host}'"));
        }
        Ok(ips.iter().map(|ip| SocketAddr::new(*ip, port)).collect())
    }
}

/// Returns `true` if `ip` lies in any blocked IPv4 or IPv6 range.
#[must_use]
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => is_blocked_ipv6(v6),
    }
}

/// Returns `true` if `ip` is loopback, private, CGNAT, link-local, `0.0.0.0/8`,
/// multicast, broadcast, unspecified, documentation, benchmarking, or reserved.
#[must_use]
pub fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_loopback() // 127.0.0.0/8
        || ip.is_private() // 10/8, 172.16/12, 192.168/16
        || ip.is_link_local() // 169.254.0.0/16 (cloud metadata)
        || ip.is_multicast() // 224.0.0.0/4
        || ip.is_broadcast() // 255.255.255.255
        || ip.is_unspecified() // 0.0.0.0
        || o[0] == 0 // 0.0.0.0/8 ("this" network)
        || (o[0] == 100 && (o[1] & 0b1100_0000) == 64) // 100.64.0.0/10 (CGNAT)
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24 (IETF protocol assignments)
        || (o[0] == 192 && o[1] == 0 && o[2] == 2) // 192.0.2.0/24 (TEST-NET-1)
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18.0.0/15 (benchmarking)
        || (o[0] == 198 && o[1] == 51 && o[2] == 100) // 198.51.100.0/24 (TEST-NET-2)
        || (o[0] == 203 && o[1] == 0 && o[2] == 113) // 203.0.113.0/24 (TEST-NET-3)
        || o[0] >= 240 // 240.0.0.0/4 (reserved / future use)
}

/// Returns `true` if `ip` is loopback, unspecified, ULA (`fc00::/7`), link-local (`fe80::/10`),
/// site-local (`fec0::/10`), multicast, documentation, or an IPv4-mapped/compatible address
/// resolving to any blocked IPv4 range.
#[must_use]
pub fn is_blocked_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }

    if let Some(mapped_v4) = ip.to_ipv4_mapped() {
        return is_blocked_ipv4(mapped_v4);
    }

    let segs = ip.segments();
    // IPv4-compatible IPv6 (::/96, deprecated): check embedded IPv4 too.
    if segs[0..6] == [0, 0, 0, 0, 0, 0] {
        let v4 = Ipv4Addr::new(
            (segs[6] >> 8) as u8,
            (segs[6] & 0xff) as u8,
            (segs[7] >> 8) as u8,
            (segs[7] & 0xff) as u8,
        );
        return is_blocked_ipv4(v4);
    }

    // ULA fc00::/7
    if (segs[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    // Link-local fe80::/10
    if (segs[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    // Deprecated site-local fec0::/10
    if (segs[0] & 0xffc0) == 0xfec0 {
        return true;
    }
    // Documentation 2001:db8::/32
    if segs[0] == 0x2001 && segs[1] == 0x0db8 {
        return true;
    }

    false
}

/// Validates `raw_url` against `allow_ports` and resolves all addresses via `resolver`,
/// failing closed if ANY resolved address is blocked or resolution fails.
pub fn validate_and_resolve(
    raw_url: &str,
    allow_ports: &[u16],
    resolver: &dyn DnsResolver,
) -> Result<ValidatedTarget, SsrfError> {
    let trimmed = raw_url.trim();
    let parsed = Url::parse(trimmed).map_err(|e| SsrfError::InvalidUrl(e.to_string()))?;

    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(SsrfError::DisallowedScheme(scheme.to_owned()));
    }

    let port = parsed
        .port_or_known_default()
        .ok_or(SsrfError::MissingHost)?;
    if !allow_ports.contains(&port) {
        return Err(SsrfError::DisallowedPort(port));
    }

    let (host_for_resolve, host_display) = match parsed.host() {
        Some(Host::Ipv4(v4)) => {
            if is_blocked_ipv4(v4) {
                return Err(SsrfError::BlockedAddress {
                    host: v4.to_string(),
                    ip: IpAddr::V4(v4),
                });
            }
            (v4.to_string(), v4.to_string())
        }
        Some(Host::Ipv6(v6)) => {
            if is_blocked_ipv6(v6) {
                return Err(SsrfError::BlockedAddress {
                    host: format!("[{v6}]"),
                    ip: IpAddr::V6(v6),
                });
            }
            (v6.to_string(), format!("[{v6}]"))
        }
        Some(Host::Domain(domain)) => {
            let clean = domain.trim_end_matches('.');
            if clean.is_empty() {
                return Err(SsrfError::MissingHost);
            }
            if clean.eq_ignore_ascii_case("localhost")
                || clean.to_ascii_lowercase().ends_with(".localhost")
            {
                return Err(SsrfError::BlockedAddress {
                    host: clean.to_owned(),
                    ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
                });
            }
            (clean.to_owned(), clean.to_owned())
        }
        None => return Err(SsrfError::MissingHost),
    };

    let resolved_addrs = resolver
        .resolve(&host_for_resolve, port)
        .map_err(SsrfError::DnsResolutionFailed)?;
    if resolved_addrs.is_empty() {
        return Err(SsrfError::DnsResolutionFailed(format!(
            "{host_display}: zero addresses returned"
        )));
    }

    for addr in &resolved_addrs {
        if is_blocked_ip(addr.ip()) {
            return Err(SsrfError::BlockedAddress {
                host: host_display,
                ip: addr.ip(),
            });
        }
    }

    Ok(ValidatedTarget {
        url: parsed,
        host: host_display,
        port,
        resolved_addrs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_all_required_ipv4_and_ipv6_ssrf_targets() {
        let resolver = SystemDnsResolver;
        let ports = [80, 443];

        let blocked_urls = [
            "http://127.0.0.1/",
            "http://127.0.0.2:80/admin",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1/",
            "http://172.16.0.1/",
            "http://192.168.1.1/",
            "http://100.64.0.1/",
            "http://0.0.0.0/",
            "http://224.0.0.1/",
            "http://255.255.255.255/",
            "http://[::1]/",
            "http://[::]/",
            "http://[fc00::1]/",
            "http://[fe80::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:169.254.169.254]/",
            "http://localhost/",
        ];

        for raw in blocked_urls {
            let res = validate_and_resolve(raw, &ports, &resolver);
            assert!(
                matches!(res, Err(SsrfError::BlockedAddress { .. })),
                "expected BlockedAddress for '{raw}', got {res:?}"
            );
        }
    }

    #[test]
    fn blocks_disallowed_port_and_non_http_scheme() {
        let resolver = SystemDnsResolver;
        let ports = [80, 443];

        assert!(matches!(
            validate_and_resolve("http://93.184.216.34:22/", &ports, &resolver),
            Err(SsrfError::DisallowedPort(22))
        ));
        assert!(matches!(
            validate_and_resolve("file:///etc/passwd", &ports, &resolver),
            Err(SsrfError::DisallowedScheme(ref s)) if s == "file"
        ));
        assert!(matches!(
            validate_and_resolve("ftp://93.184.216.34/", &ports, &resolver),
            Err(SsrfError::DisallowedScheme(ref s)) if s == "ftp"
        ));
    }

    #[test]
    fn allows_public_ip_on_allowed_port_and_rejects_mixed_dns_records() {
        let resolver = SystemDnsResolver;
        let ports = [80, 443];

        let ok = validate_and_resolve("https://93.184.216.34/docs", &ports, &resolver).unwrap();
        assert_eq!(ok.port, 443);
        assert_eq!(ok.resolved_addrs.len(), 1);

        let mut mock = MockDnsResolver::new();
        mock.insert(
            "dual.example.com",
            vec![
                IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
                IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
            ],
        );
        let err = validate_and_resolve("https://dual.example.com/", &ports, &mock).unwrap_err();
        assert!(matches!(err, SsrfError::BlockedAddress { .. }));
    }
}
