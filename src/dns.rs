use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde::Deserialize;
use tokio::sync::RwLock;

struct CachedIps {
    ips: Vec<IpAddr>,
    expires_at: Instant,
}

/// Custom DNS resolver using native system resolution with an automatic,
/// cached DNS-over-HTTPS (`1.1.1.1` / `8.8.8.8`) fallback for Android/Termux
/// and networks where direct UDP port 53 is unavailable.
pub struct SmartDnsResolver {
    cache: Arc<RwLock<HashMap<String, CachedIps>>>,
    doh_client: reqwest::Client,
}

impl SmartDnsResolver {
    pub fn new() -> Result<Self, String> {
        let doh_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(2000))
            .timeout(Duration::from_millis(2500))
            .build()
            .map_err(|e| format!("failed to build DoH client: {e}"))?;

        Ok(Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            doh_client,
        })
    }

    #[cfg(test)]
    pub(crate) async fn resolve_host(&self, host: &str) -> Result<Vec<IpAddr>, std::io::Error> {
        resolve_host_with(&self.cache, &self.doh_client, host).await
    }
}

impl Resolve for SmartDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        let cache = Arc::clone(&self.cache);
        let doh_client = self.doh_client.clone();

        Box::pin(async move {
            let ips = resolve_host_with(&cache, &doh_client, &host)
                .await
                .map_err(|err| Box::new(err) as Box<dyn std::error::Error + Send + Sync>)?;
            Ok(ips_to_addrs(ips))
        })
    }
}

async fn resolve_host_with(
    cache: &RwLock<HashMap<String, CachedIps>>,
    doh_client: &reqwest::Client,
    host: &str,
) -> Result<Vec<IpAddr>, std::io::Error> {
    if host.eq_ignore_ascii_case("localhost") {
        return Ok(vec![
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
        ]);
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }

    if let Some(ips) = get_cached(cache, host).await {
        return Ok(ips);
    }

    if let Ok(Ok(addrs)) = tokio::time::timeout(
        Duration::from_millis(1500),
        tokio::net::lookup_host((host, 0)),
    )
    .await
    {
        let ips: Vec<IpAddr> = addrs.map(|a| a.ip()).collect();
        if !ips.is_empty() {
            insert_cached(cache, host.to_string(), ips.clone(), 120).await;
            return Ok(ips);
        }
    }

    if let Some((ips, ttl)) = resolve_via_doh(doh_client, host).await {
        insert_cached(cache, host.to_string(), ips.clone(), ttl).await;
        return Ok(ips);
    }

    Err(std::io::Error::other(format!(
        "DNS resolution failed for {host}"
    )))
}

fn ips_to_addrs(ips: Vec<IpAddr>) -> Addrs {
    let addrs: Vec<SocketAddr> = ips.into_iter().map(|ip| SocketAddr::new(ip, 0)).collect();
    Box::new(addrs.into_iter())
}

async fn get_cached(cache: &RwLock<HashMap<String, CachedIps>>, host: &str) -> Option<Vec<IpAddr>> {
    let now = Instant::now();
    let guard = cache.read().await;
    guard
        .get(host)
        .filter(|entry| entry.expires_at > now)
        .map(|entry| entry.ips.clone())
}

async fn insert_cached(
    cache: &RwLock<HashMap<String, CachedIps>>,
    host: String,
    ips: Vec<IpAddr>,
    ttl_secs: u64,
) {
    let now = Instant::now();
    let mut guard = cache.write().await;
    if guard.len() > 256 {
        guard.retain(|_, v| v.expires_at > now);
    }
    guard.insert(
        host,
        CachedIps {
            ips,
            expires_at: now + Duration::from_secs(ttl_secs),
        },
    );
}

async fn resolve_via_doh(client: &reqwest::Client, host: &str) -> Option<(Vec<IpAddr>, u64)> {
    let endpoints = ["https://1.1.1.1/dns-query", "https://8.8.8.8/resolve"];
    for endpoint in endpoints {
        let Ok(resp) = client
            .get(endpoint)
            .query(&[("name", host), ("type", "A")])
            .header(reqwest::header::ACCEPT, "application/dns-json")
            .send()
            .await
        else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(body) = resp.text().await else {
            continue;
        };
        if let Some(parsed) = parse_doh_json(&body) {
            return Some(parsed);
        }
    }
    None
}

#[derive(Deserialize)]
struct DohResponse {
    #[serde(rename = "Answer", default)]
    answer: Vec<DohAnswer>,
}

#[derive(Deserialize)]
struct DohAnswer {
    #[serde(rename = "type", default)]
    record_type: u16,
    #[serde(rename = "TTL", default)]
    ttl: u64,
    #[serde(default)]
    data: String,
}

pub(crate) fn parse_doh_json(body: &str) -> Option<(Vec<IpAddr>, u64)> {
    let parsed: DohResponse = serde_json::from_str(body).ok()?;
    let mut ips = Vec::new();
    let mut min_ttl: Option<u64> = None;

    for ans in parsed.answer {
        if ans.record_type != 1 && ans.record_type != 28 {
            continue;
        }
        if let Ok(ip) = ans.data.trim().parse::<IpAddr>() {
            if !ips.contains(&ip) {
                ips.push(ip);
            }
            if ans.ttl > 0 {
                min_ttl = Some(min_ttl.map_or(ans.ttl, |cur| cur.min(ans.ttl)));
            }
        }
    }

    if ips.is_empty() {
        None
    } else {
        Some((ips, min_ttl.unwrap_or(120).clamp(30, 300)))
    }
}

/// Builds a shared, pooled `reqwest::Client` using `rustls` and `SmartDnsResolver`.
pub fn build_http_client(timeout_ms: u64, proxy: Option<&str>) -> Result<reqwest::Client, String> {
    let resolver = Arc::new(SmartDnsResolver::new()?);
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
    fn parses_doh_json_with_cname_and_a_records() {
        let json = r#"{
            "Status": 0,
            "Answer": [
                {"name": "en.wikipedia.org", "type": 5, "TTL": 86400, "data": "dyna.wikimedia.org."},
                {"name": "dyna.wikimedia.org", "type": 1, "TTL": 122, "data": "185.15.59.224"}
            ]
        }"#;
        let (ips, ttl) = parse_doh_json(json).expect("should parse A record");
        assert_eq!(ips, vec![IpAddr::V4(Ipv4Addr::new(185, 15, 59, 224))]);
        assert_eq!(ttl, 122);
    }

    #[test]
    fn parses_doh_aaaa_and_rejects_empty() {
        let json = r#"{"Status":0,"Answer":[{"type":28,"TTL":10,"data":"2001:db8::1"}]}"#;
        let (ips, ttl) = parse_doh_json(json).expect("should parse AAAA record");
        assert_eq!(ips, vec!["2001:db8::1".parse::<IpAddr>().unwrap()]);
        assert_eq!(ttl, 30);

        assert!(parse_doh_json(r#"{"Status":0,"Answer":[]}"#).is_none());
    }

    #[tokio::test]
    async fn resolves_localhost_and_ip_literals() {
        let resolver = SmartDnsResolver::new().unwrap();
        let ips = resolver.resolve_host("localhost").await.unwrap();
        assert!(ips.contains(&IpAddr::V4(Ipv4Addr::LOCALHOST)));

        let ip_only = resolver.resolve_host("127.0.0.1").await.unwrap();
        assert_eq!(ip_only, vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]);
    }
}
