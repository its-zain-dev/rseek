use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{ConnectInfo, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::config::ServerConfig;
use crate::engine::SearchEngineService;
use crate::models::{Category, SearchQuery};

const UI_HTML: &str = include_str!("ui.html");

/// Shared server state passed to Axum handlers and middleware.
pub struct AppState {
    pub service: Arc<SearchEngineService>,
    pub config: ServerConfig,
    rate_limiter: Mutex<HashMap<String, (u32, Instant)>>,
}

impl AppState {
    pub fn new(service: Arc<SearchEngineService>, config: ServerConfig) -> Self {
        Self {
            service,
            config,
            rate_limiter: Mutex::new(HashMap::new()),
        }
    }
}

/// Builds the Axum [`Router`] with Web UI, JSON API endpoints, OpenSearch, auth, and CORS.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(ui_handler))
        .route("/api/v1/search", get(search_handler))
        .route("/search", get(search_handler))
        .route("/api/v1/engines", get(engines_handler))
        .route("/api/v1/suggest", get(suggest_handler))
        .route("/api/v1/health", get(health_handler))
        .route("/opensearch.xml", get(opensearch_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            guard_and_cors,
        ))
        .with_state(state)
}

/// Starts the HTTP server until `Ctrl+C` shutdown signal.
pub async fn serve(service: Arc<SearchEngineService>, config: ServerConfig) -> Result<(), String> {
    let bind_addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .map_err(|e| format!("failed to bind {bind_addr}: {e}"))?;

    let state = Arc::new(AppState::new(service, config.clone()));
    let app = build_router(state);

    eprintln!(
        "rseek listening on http://{}:{} (rate limit: {}/min, auth: {})",
        config.host,
        config.port,
        config.rate_limit_per_min,
        if config.api_key.is_some() {
            "api-key"
        } else {
            "open"
        }
    );

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
    .map_err(|e| format!("server error: {e}"))
}

async fn ui_handler() -> Html<&'static str> {
    Html(UI_HTML)
}

#[derive(Deserialize)]
pub struct SearchParams {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub categories: Option<String>,
    #[serde(default)]
    pub engines: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub page: Option<u32>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub no_cache: Option<bool>,
}

async fn search_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> Response {
    if params.q.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "missing required query parameter 'q'" })),
        )
            .into_response();
    }

    let raw_cat = params
        .category
        .as_deref()
        .or(params.categories.as_deref())
        .unwrap_or("general");
    let category: Category = raw_cat.parse().unwrap_or(Category::General);

    let engines: Vec<String> = params
        .engines
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let query = SearchQuery {
        query: params.q,
        category,
        engines,
        lang: params.lang.unwrap_or_else(|| "en".to_string()),
        page: params.page.unwrap_or(1),
        limit: params.limit.unwrap_or(15),
        timeout_ms: params.timeout_ms.unwrap_or(0),
        no_cache: params.no_cache.unwrap_or(false),
    };

    let resp = state.service.search(query).await;
    (StatusCode::OK, Json(resp)).into_response()
}

async fn engines_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let infos = state.service.engine_infos().await;
    Json(serde_json::json!({ "engines": infos }))
}

#[derive(Deserialize)]
pub struct SuggestParams {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub lang: Option<String>,
}

async fn suggest_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SuggestParams>,
) -> impl IntoResponse {
    let lang = params.lang.as_deref().unwrap_or("en");
    let resp = state.service.suggest(&params.q, lang).await;
    Json(resp)
}

async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let infos = state.service.engine_infos().await;
    let healthy_count = infos.iter().filter(|e| e.healthy).count();
    let cache_entries = state.service.cache_len().await;
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": state.service.uptime_secs(),
        "engines_total": infos.len(),
        "engines_healthy": healthy_count,
        "cache_entries": cache_entries,
    }))
}

async fn opensearch_handler(headers: HeaderMap) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost:8080");
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .filter(|p| *p == "http" || *p == "https")
        .unwrap_or("http");
    let escaped_base = xml_escape(&format!("{proto}://{host}"));

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<OpenSearchDescription xmlns="http://a9.com/-/spec/opensearch/1.1/">
  <ShortName>rseek</ShortName>
  <Description>Lightweight free Rust metasearch engine</Description>
  <InputEncoding>UTF-8</InputEncoding>
  <Url type="text/html" method="get" template="{escaped_base}/?q={{searchTerms}}" />
  <Url type="application/x-suggestions+json" method="get" template="{escaped_base}/api/v1/suggest?q={{searchTerms}}" />
</OpenSearchDescription>
"#
    );

    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/opensearchdescription+xml; charset=utf-8",
        )],
        xml,
    )
        .into_response()
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

async fn guard_and_cors(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let origin = state.config.cors_origin.as_str();

    if request.method() == Method::OPTIONS {
        let mut resp = StatusCode::NO_CONTENT.into_response();
        apply_cors_headers(resp.headers_mut(), origin);
        return resp;
    }

    let path = request.uri().path();
    let protected = path.starts_with("/api/") || path == "/search";

    if protected {
        if let Some(expected_key) = &state.config.api_key {
            if !is_authorized(&request, expected_key) {
                let mut resp = (
                    StatusCode::UNAUTHORIZED,
                    Json(
                        serde_json::json!({ "error": "unauthorized: invalid or missing API key" }),
                    ),
                )
                    .into_response();
                apply_cors_headers(resp.headers_mut(), origin);
                return resp;
            }
        }

        if state.config.rate_limit_per_min > 0 {
            let client_ip = extract_client_ip(&request);
            let allowed = check_rate_limit(
                &state.rate_limiter,
                client_ip,
                state.config.rate_limit_per_min,
            )
            .await;
            if !allowed {
                let mut resp = (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(serde_json::json!({ "error": "rate limit exceeded; retry shortly" })),
                )
                    .into_response();
                apply_cors_headers(resp.headers_mut(), origin);
                return resp;
            }
        }
    }

    let mut resp = next.run(request).await;
    apply_cors_headers(resp.headers_mut(), origin);
    resp
}

fn apply_cors_headers(headers: &mut HeaderMap, origin: &str) {
    if let Ok(val) = HeaderValue::from_str(origin) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, val);
    }
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type, Authorization, X-API-Key"),
    );
}

fn is_authorized(request: &Request, expected_key: &str) -> bool {
    if let Some(header_key) = request
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
    {
        if header_key == expected_key {
            return true;
        }
    }

    if let Some(auth_val) = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(token) = auth_val.strip_prefix("Bearer ") {
            if token.trim() == expected_key {
                return true;
            }
        }
    }

    if let Some(query_str) = request.uri().query() {
        for (k, v) in url::form_urlencoded::parse(query_str.as_bytes()) {
            if k == "api_key" && v == expected_key {
                return true;
            }
        }
    }

    false
}

fn extract_client_ip(request: &Request) -> String {
    if let Some(xff) = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
    {
        if let Some(first) = xff
            .split(',')
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return first.to_string();
        }
    }
    if let Some(xri) = request
        .headers()
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return xri.to_string();
    }
    if let Some(ConnectInfo(addr)) = request.extensions().get::<ConnectInfo<SocketAddr>>() {
        return addr.ip().to_string();
    }
    "unknown".to_string()
}

async fn check_rate_limit(
    limiter: &Mutex<HashMap<String, (u32, Instant)>>,
    ip: String,
    limit: u32,
) -> bool {
    let now = Instant::now();
    let mut map = limiter.lock().await;
    if map.len() > 1024 {
        map.retain(|_, (_, started)| now.duration_since(*started).as_secs() < 60);
    }

    let entry = map.entry(ip).or_insert((0, now));
    if now.duration_since(entry.1).as_secs() >= 60 {
        *entry = (1, now);
        return true;
    }

    if entry.0 >= limit {
        false
    } else {
        entry.0 += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SearchConfig;

    async fn spawn_test_server(
        server_config: ServerConfig,
    ) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let service = Arc::new(SearchEngineService::new(SearchConfig::default()).unwrap());
        let state = Arc::new(AppState::new(service, server_config));
        let app = build_router(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        (addr, handle)
    }

    #[tokio::test]
    async fn server_serves_ui_health_engines_and_opensearch() {
        let (addr, handle) = spawn_test_server(ServerConfig::default()).await;
        let client = reqwest::Client::new();
        let base = format!("http://{addr}");

        let ui = client.get(&base).send().await.unwrap();
        assert_eq!(ui.status(), 200);
        assert!(ui.text().await.unwrap().contains("rseek"));

        let health = client
            .get(format!("{base}/api/v1/health"))
            .send()
            .await
            .unwrap();
        assert_eq!(health.status(), 200);
        let json: serde_json::Value = health.json().await.unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["engines_total"], 10);

        let engines = client
            .get(format!("{base}/api/v1/engines"))
            .send()
            .await
            .unwrap();
        assert_eq!(engines.status(), 200);

        let os = client
            .get(format!("{base}/opensearch.xml"))
            .send()
            .await
            .unwrap();
        assert_eq!(os.status(), 200);
        assert!(os.text().await.unwrap().contains("<OpenSearchDescription"));

        let missing_q = client
            .get(format!("{base}/api/v1/search"))
            .send()
            .await
            .unwrap();
        assert_eq!(missing_q.status(), 400);

        handle.abort();
    }

    #[tokio::test]
    async fn server_enforces_api_key_and_rate_limit() {
        let cfg = ServerConfig {
            api_key: Some("topsecret".to_string()),
            rate_limit_per_min: 2,
            ..ServerConfig::default()
        };
        let (addr, handle) = spawn_test_server(cfg).await;
        let client = reqwest::Client::new();
        let url = format!("http://{addr}/api/v1/health");

        let unauth = client.get(&url).send().await.unwrap();
        assert_eq!(unauth.status(), 401);

        let ok1 = client
            .get(&url)
            .header("X-API-Key", "topsecret")
            .send()
            .await
            .unwrap();
        assert_eq!(ok1.status(), 200);

        let ok2 = client
            .get(&url)
            .bearer_auth("topsecret")
            .send()
            .await
            .unwrap();
        assert_eq!(ok2.status(), 200);

        let limited = client
            .get(&url)
            .header("X-API-Key", "topsecret")
            .send()
            .await
            .unwrap();
        assert_eq!(limited.status(), 429);

        handle.abort();
    }
}
