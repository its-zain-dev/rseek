/// Configuration for upstream search requests, caching, and user-agents.
#[derive(Debug, Clone)]
pub struct SearchConfig {
    pub timeout_ms: u64,
    pub cache_ttl_secs: u64,
    pub cache_max_entries: usize,
    pub proxy: Option<String>,
    pub user_agent: String,
    pub browser_user_agent: String,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 5000,
            cache_ttl_secs: 300,
            cache_max_entries: 500,
            proxy: None,
            user_agent: format!(
                "rseek/{} (+https://github.com/its-zain-dev/rseek)",
                env!("CARGO_PKG_VERSION")
            ),
            browser_user_agent: concat!(
                "Mozilla/5.0 (X11; Linux x86_64; rv:132.0) ",
                "Gecko/20100101 Firefox/132.0"
            )
            .to_string(),
        }
    }
}

/// Configuration for the embedded HTTP server and API access controls.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub api_key: Option<String>,
    pub cors_origin: String,
    pub rate_limit_per_min: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            api_key: None,
            cors_origin: "*".to_string(),
            rate_limit_per_min: 60,
        }
    }
}
