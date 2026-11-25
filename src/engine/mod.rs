pub mod arxiv;
pub mod crates_io;
pub mod duckduckgo;
pub mod github;
pub mod hackernews;
pub mod npm;
pub mod stackexchange;
pub mod wiby;
pub mod wikimedia;
pub mod wikipedia;

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::join_all;
use tokio::sync::Mutex;

use crate::bangs::parse_bangs;
use crate::cache::SearchCache;
use crate::config::SearchConfig;
use crate::dns::build_http_client;
use crate::models::{
    Category, EngineInfo, EngineReport, EngineStatus, Infobox, RawResult, SearchQuery,
    SearchResponse, SuggestResponse,
};
use crate::ranker::rank_and_dedupe;
use crate::suggest::fetch_suggestions;
use crate::text::sanitize_lang;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Request parameters forwarded to each upstream [`SearchEngine`].
#[derive(Debug, Clone)]
pub struct EngineRequest {
    pub query: String,
    pub lang: String,
    pub page: u32,
    pub user_agent: String,
    pub browser_user_agent: String,
}

/// Errors returned by individual search engine implementations.
#[derive(Debug, Clone)]
pub enum EngineError {
    RateLimited(String),
    Http(String),
    Parse(String),
    Timeout,
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RateLimited(msg) => write!(f, "rate limited: {msg}"),
            Self::Http(msg) => write!(f, "http error: {msg}"),
            Self::Parse(msg) => write!(f, "parse error: {msg}"),
            Self::Timeout => f.write_str("request timed out"),
        }
    }
}

impl std::error::Error for EngineError {}

/// Trait implemented by every upstream search source.
pub trait SearchEngine: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn categories(&self) -> &'static [Category];
    fn bangs(&self) -> &'static [&'static str];
    fn weight(&self) -> f64;
    fn search<'a>(
        &'a self,
        client: &'a reqwest::Client,
        req: &'a EngineRequest,
    ) -> BoxFuture<'a, Result<Vec<RawResult>, EngineError>>;
}

pub(crate) fn map_reqwest_err(engine: &str, err: reqwest::Error) -> EngineError {
    if err.is_timeout() {
        EngineError::Timeout
    } else {
        EngineError::Http(format!("{engine}: {err}"))
    }
}

pub(crate) fn check_http_status(
    status: reqwest::StatusCode,
    engine: &str,
) -> Result<(), EngineError> {
    let code = status.as_u16();
    if code == 429 || code == 202 {
        return Err(EngineError::RateLimited(format!("{engine} HTTP {code}")));
    }
    if !status.is_success() {
        return Err(EngineError::Http(format!("{engine} HTTP {code}")));
    }
    Ok(())
}

#[derive(Debug, Default)]
struct EngineHealth {
    consecutive_errors: u32,
    cooldown_until: Option<Instant>,
    success_count: u64,
    error_count: u64,
    total_latency_ms: u64,
}

impl EngineHealth {
    fn cooldown_remaining(&self, now: Instant) -> u64 {
        self.cooldown_until
            .and_then(|until| until.checked_duration_since(now))
            .map(|d| d.as_secs().max(1))
            .unwrap_or(0)
    }

    fn record_success(&mut self, took_ms: u64) {
        self.consecutive_errors = 0;
        self.cooldown_until = None;
        self.success_count += 1;
        self.total_latency_ms += took_ms;
    }

    fn record_failure(&mut self, now: Instant, rate_limited: bool, took_ms: u64) {
        self.error_count += 1;
        self.total_latency_ms += took_ms;
        self.consecutive_errors += 1;
        if rate_limited || self.consecutive_errors >= 3 {
            self.cooldown_until = Some(now + Duration::from_secs(60));
        }
    }
}

/// Core metasearch orchestrator managing engines, circuit breakers, and response caching.
pub struct SearchEngineService {
    client: reqwest::Client,
    engines: Vec<Arc<dyn SearchEngine>>,
    health: HashMap<&'static str, Mutex<EngineHealth>>,
    cache: SearchCache,
    config: SearchConfig,
    started_at: Instant,
}

impl SearchEngineService {
    /// Creates a new [`SearchEngineService`] with all built-in free engines registered.
    pub fn new(config: SearchConfig) -> Result<Self, String> {
        let client = build_http_client(config.timeout_ms, config.proxy.as_deref())?;
        let engines: Vec<Arc<dyn SearchEngine>> = vec![
            Arc::new(duckduckgo::DuckDuckGoEngine),
            Arc::new(wikipedia::WikipediaEngine),
            Arc::new(wiby::WibyEngine),
            Arc::new(hackernews::HackerNewsEngine),
            Arc::new(github::GitHubEngine),
            Arc::new(stackexchange::StackExchangeEngine),
            Arc::new(arxiv::ArxivEngine),
            Arc::new(crates_io::CratesIoEngine),
            Arc::new(npm::NpmEngine),
            Arc::new(wikimedia::WikimediaEngine),
        ];

        let mut health = HashMap::new();
        for engine in &engines {
            health.insert(engine.name(), Mutex::new(EngineHealth::default()));
        }

        let cache = SearchCache::new(config.cache_ttl_secs, config.cache_max_entries);

        Ok(Self {
            client,
            engines,
            health,
            cache,
            config,
            started_at: Instant::now(),
        })
    }

    /// Executes a metasearch query across selected engines concurrently.
    pub async fn search(&self, req: SearchQuery) -> SearchResponse {
        let start = Instant::now();
        let parsed = parse_bangs(&req.query);
        let effective_query = parsed.effective_query;
        let effective_category = parsed.category_override.unwrap_or(req.category);
        let page = req.page.max(1);
        let limit = req.limit.clamp(1, 50);
        let lang = sanitize_lang(&req.lang);
        let timeout_ms = if req.timeout_ms > 0 {
            req.timeout_ms.clamp(500, 30_000)
        } else {
            self.config.timeout_ms
        };

        let explicit_engines: Vec<String> = if !req.engines.is_empty() {
            req.engines
                .iter()
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            parsed.engines_override
        };

        if effective_query.is_empty() {
            return SearchResponse {
                query: req.query,
                effective_query,
                category: effective_category,
                page,
                took_ms: start.elapsed().as_millis() as u64,
                cached: false,
                total_results: 0,
                infobox: None,
                results: Vec::new(),
                engines: Vec::new(),
            };
        }

        let selected = self.select_engines(effective_category, &explicit_engines);
        let engine_names_key: Vec<&str> = selected.iter().map(|e| e.name()).collect();
        let cache_key = format!(
            "{}|{}|{}|{}|{}|{}",
            effective_query.to_ascii_lowercase(),
            effective_category,
            lang,
            page,
            limit,
            engine_names_key.join(",")
        );

        if !req.no_cache {
            if let Some(cached) = self.cache.get(&cache_key).await {
                return cached;
            }
        }

        let engine_req = EngineRequest {
            query: effective_query.clone(),
            lang: lang.clone(),
            page,
            user_agent: self.config.user_agent.clone(),
            browser_user_agent: self.config.browser_user_agent.clone(),
        };

        let want_infobox = page == 1
            && explicit_engines.is_empty()
            && matches!(effective_category, Category::General | Category::Wiki);

        let engine_futures = selected
            .iter()
            .map(|engine| self.run_single_engine(engine, &engine_req, timeout_ms));

        let infobox_future = async {
            if want_infobox {
                tokio::time::timeout(
                    Duration::from_millis(timeout_ms.min(2500)),
                    duckduckgo::fetch_infobox(
                        &self.client,
                        &effective_query,
                        &self.config.user_agent,
                    ),
                )
                .await
                .unwrap_or(None)
            } else {
                None
            }
        };

        let (outcomes, infobox): (Vec<SingleOutcome>, Option<Infobox>) =
            tokio::join!(join_all(engine_futures), infobox_future);

        let mut raw_all = Vec::new();
        let mut reports = Vec::with_capacity(outcomes.len());
        let mut weights = HashMap::new();

        for outcome in outcomes {
            weights.insert(outcome.report.name.clone(), outcome.weight);
            reports.push(outcome.report);
            raw_all.extend(outcome.results);
        }

        let results = rank_and_dedupe(&effective_query, raw_all, &weights, limit);
        let total_results = results.len();
        let took_ms = start.elapsed().as_millis() as u64;

        let response = SearchResponse {
            query: req.query,
            effective_query,
            category: effective_category,
            page,
            took_ms,
            cached: false,
            total_results,
            infobox,
            results,
            engines: reports,
        };

        if total_results > 0 && !req.no_cache {
            self.cache.insert(cache_key, response.clone()).await;
        }

        response
    }

    fn select_engines(
        &self,
        category: Category,
        explicit: &[String],
    ) -> Vec<Arc<dyn SearchEngine>> {
        if !explicit.is_empty() {
            return self
                .engines
                .iter()
                .filter(|e| explicit.iter().any(|name| name == e.name()))
                .cloned()
                .collect();
        }

        self.engines
            .iter()
            .filter(|e| match category {
                Category::All => !e.categories().contains(&Category::Images),
                other => e.categories().contains(&other),
            })
            .cloned()
            .collect()
    }

    async fn run_single_engine(
        &self,
        engine: &Arc<dyn SearchEngine>,
        req: &EngineRequest,
        timeout_ms: u64,
    ) -> SingleOutcome {
        let name = engine.name();
        let now = Instant::now();

        if let Some(lock) = self.health.get(name) {
            let guard = lock.lock().await;
            let rem = guard.cooldown_remaining(now);
            if rem > 0 {
                return SingleOutcome {
                    weight: engine.weight(),
                    results: Vec::new(),
                    report: EngineReport {
                        name: name.to_string(),
                        status: EngineStatus::CircuitOpen,
                        results: 0,
                        took_ms: 0,
                        error: Some(format!("cooling down for {rem}s")),
                    },
                };
            }
        }

        let started = Instant::now();
        let timed = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            engine.search(&self.client, req),
        )
        .await;
        let took_ms = started.elapsed().as_millis() as u64;

        match timed {
            Ok(Ok(items)) => {
                if let Some(lock) = self.health.get(name) {
                    lock.lock().await.record_success(took_ms);
                }
                let count = items.len();
                let status = if count == 0 {
                    EngineStatus::Empty
                } else {
                    EngineStatus::Ok
                };
                SingleOutcome {
                    weight: engine.weight(),
                    results: items,
                    report: EngineReport {
                        name: name.to_string(),
                        status,
                        results: count,
                        took_ms,
                        error: None,
                    },
                }
            }
            Ok(Err(err)) => {
                let rate_limited = matches!(err, EngineError::RateLimited(_));
                let status = match &err {
                    EngineError::RateLimited(_) => EngineStatus::RateLimited,
                    EngineError::Timeout => EngineStatus::Timeout,
                    EngineError::Http(_) | EngineError::Parse(_) => EngineStatus::Error,
                };
                if let Some(lock) = self.health.get(name) {
                    lock.lock()
                        .await
                        .record_failure(Instant::now(), rate_limited, took_ms);
                }
                SingleOutcome {
                    weight: engine.weight(),
                    results: Vec::new(),
                    report: EngineReport {
                        name: name.to_string(),
                        status,
                        results: 0,
                        took_ms,
                        error: Some(err.to_string()),
                    },
                }
            }
            Err(_) => {
                if let Some(lock) = self.health.get(name) {
                    lock.lock()
                        .await
                        .record_failure(Instant::now(), false, took_ms);
                }
                SingleOutcome {
                    weight: engine.weight(),
                    results: Vec::new(),
                    report: EngineReport {
                        name: name.to_string(),
                        status: EngineStatus::Timeout,
                        results: 0,
                        took_ms,
                        error: Some(format!("exceeded {timeout_ms}ms")),
                    },
                }
            }
        }
    }

    /// Returns metadata and live circuit-breaker telemetry for all engines.
    pub async fn engine_infos(&self) -> Vec<EngineInfo> {
        let now = Instant::now();
        let mut out = Vec::with_capacity(self.engines.len());
        for engine in &self.engines {
            let (healthy, success_count, error_count, avg_latency_ms, cooldown_remaining_secs) =
                if let Some(lock) = self.health.get(engine.name()) {
                    let g = lock.lock().await;
                    let total_calls = g.success_count + g.error_count;
                    let avg = if total_calls > 0 {
                        g.total_latency_ms / total_calls
                    } else {
                        0
                    };
                    let rem = g.cooldown_remaining(now);
                    (rem == 0, g.success_count, g.error_count, avg, rem)
                } else {
                    (true, 0, 0, 0, 0)
                };

            out.push(EngineInfo {
                name: engine.name().to_string(),
                description: engine.description().to_string(),
                categories: engine.categories().to_vec(),
                bangs: engine.bangs().iter().map(|b| (*b).to_string()).collect(),
                weight: engine.weight(),
                healthy,
                success_count,
                error_count,
                avg_latency_ms,
                cooldown_remaining_secs,
            });
        }
        out
    }

    /// Fetches query autocomplete suggestions.
    pub async fn suggest(&self, query: &str, lang: &str) -> SuggestResponse {
        fetch_suggestions(&self.client, query, lang, &self.config.user_agent).await
    }

    pub async fn cache_len(&self) -> usize {
        self.cache.len().await
    }

    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    pub fn config(&self) -> &SearchConfig {
        &self.config
    }
}

struct SingleOutcome {
    weight: f64,
    results: Vec<RawResult>,
    report: EngineReport,
}
