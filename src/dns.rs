use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use hickory_resolver::config::{NameServerConfigGroup, ResolverConfig, ResolverOpts};
use hickory_resolver::system_conf::read_system_conf;
use hickory_resolver::TokioAsyncResolver;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// Custom DNS resolver backed by `hickory-resolver` that gracefully handles
/// Android/Termux environments lacking `/etc/resolv.conf` and static musl binaries.
#[derive(Debug, Default)]
pub struct SmartDnsResolver {
    inner: OnceLock<TokioAsyncResolver>,
}

impl SmartDnsResolver {
    pub fn new() -> Self {
        Self {
            inner: OnceLock::new(),
        }
    }

    fn resolver(&self) -> &TokioAsyncResolver {
        self.inner.get_or_init(build_tokio_resolver)
    }
}

impl Resolve for SmartDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let resolver = self.resolver().clone();
        let host = name.as_str().to_string();
        Box::pin(async move {
            let lookup = resolver
                .lookup_ip(host)
                .await
                .map_err(|err| Box::new(err) as Box<dyn std::error::Error + Send + Sync>)?;
            let addrs: Vec<SocketAddr> = lookup.iter().map(|ip| SocketAddr::new(ip, 0)).collect();
            let iter: Addrs = Box::new(addrs.into_iter());
            Ok(iter)
        })
    }
}

fn build_tokio_resolver() -> TokioAsyncResolver {
    if let Ok((cfg, opts)) = read_system_conf() {
        if !cfg.name_servers().is_empty() {
            return TokioAsyncResolver::tokio(cfg, opts);
        }
    }

    let mut ips = Vec::new();
    if let Ok(prefix) = std::env::var("PREFIX") {
        let candidate = format!("{prefix}/etc/resolv.conf");
        ips.extend(parse_nameservers_from_file(&candidate));
    }
    if ips.is_empty() {
        ips.extend(parse_nameservers_from_file(
            "/data/data/com.termux/files/usr/etc/resolv.conf",
        ));
    }
    if ips.is_empty() {
        ips = vec![
            IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)),
        ];
    }

    let group = NameServerConfigGroup::from_ips_clear(&ips, 53, true);
    let cfg = ResolverConfig::from_parts(None, Vec::new(), group);
    TokioAsyncResolver::tokio(cfg, ResolverOpts::default())
}

fn parse_nameservers_from_file(path: &str) -> Vec<IpAddr> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_nameserver_lines(&contents)
}

pub(crate) fn parse_nameserver_lines(contents: &str) -> Vec<IpAddr> {
    let mut ips = Vec::new();
    for line in contents.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut parts = line.split_whitespace();
        if matches!(parts.next(), Some("nameserver")) {
            if let Some(raw_ip) = parts.next() {
                if let Ok(ip) = raw_ip.parse::<IpAddr>() {
                    if !ips.contains(&ip) {
                        ips.push(ip);
                    }
                }
            }
        }
    }
    ips
}

/// Builds a shared, pooled `reqwest::Client` using `rustls` and `SmartDnsResolver`.
pub fn build_http_client(timeout_ms: u64, proxy: Option<&str>) -> Result<reqwest::Client, String> {
    let resolver = Arc::new(SmartDnsResolver::new());
    let mut builder = reqwest::Client::builder()
        .dns_resolver(resolver)
        .connect_timeout(Duration::from_millis(timeout_ms.clamp(1000, 15_000)))
        .timeout(Duration::from_millis(timeout_ms.max(1500)))
        .pool_idle_timeout(Duration::from_secs(60));

    if let Some(proxy_url) = proxy.filter(|s| !s.trim().is_empty()) {
        let parsed =
            reqwest::Proxy::all(proxy_url).map_err(|e| format!("invalid proxy URL: {e}"))?;
        builder = builder.proxy(parsed);
    }

    builder
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_resolv_conf_nameservers() {
        let sample = "\
# comment line
nameserver 8.8.8.8
nameserver 1.1.1.1 # inline comment
nameserver invalid-ip
nameserver 8.8.8.8
search localdomain
";
        let ips = parse_nameserver_lines(sample);
        assert_eq!(
            ips,
            vec![
                IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
                IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            ]
        );
    }
}
