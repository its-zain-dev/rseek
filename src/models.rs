use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// Search categories supported by rseek.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    #[default]
    General,
    Code,
    News,
    Science,
    Packages,
    Wiki,
    Images,
    All,
}

impl Category {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Code => "code",
            Self::News => "news",
            Self::Science => "science",
            Self::Packages => "packages",
            Self::Wiki => "wiki",
            Self::Images => "images",
            Self::All => "all",
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Category {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "general" | "web" | "default" => Ok(Self::General),
            "code" | "dev" | "it" => Ok(Self::Code),
            "news" => Ok(Self::News),
            "science" | "sci" | "academic" => Ok(Self::Science),
            "packages" | "pkg" | "crates" | "npm" => Ok(Self::Packages),
            "wiki" | "wikipedia" => Ok(Self::Wiki),
            "images" | "img" | "image" => Ok(Self::Images),
            "all" => Ok(Self::All),
            other => Err(format!("unknown category '{other}'")),
        }
    }
}

/// Input query passed to [`crate::engine::SearchEngineService::search`].
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub query: String,
    pub category: Category,
    pub engines: Vec<String>,
    pub lang: String,
    pub page: u32,
    pub limit: usize,
    pub timeout_ms: u64,
    pub no_cache: bool,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            category: Category::General,
            engines: Vec::new(),
            lang: "en".to_string(),
            page: 1,
            limit: 15,
            timeout_ms: 5000,
            no_cache: false,
        }
    }
}

/// Single result produced by an individual upstream engine before fusion/deduplication.
#[derive(Debug, Clone)]
pub struct RawResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub engine: String,
    pub rank: usize,
    pub category: Category,
    pub published: Option<String>,
    pub thumbnail: Option<String>,
    pub metadata: BTreeMap<String, serde_json::Value>,
}

/// Deduplicated, ranked search result returned to API and CLI consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub engines: Vec<String>,
    pub score: f64,
    pub category: Category,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

/// Optional instant-answer summary card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Infobox {
    pub title: String,
    #[serde(rename = "abstract")]
    pub abstract_text: String,
    pub url: String,
    pub source: String,
}

/// Execution status of an engine for a single search request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineStatus {
    Ok,
    Empty,
    Timeout,
    RateLimited,
    CircuitOpen,
    Error,
}

/// Per-engine telemetry included in every [`SearchResponse`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineReport {
    pub name: String,
    pub status: EngineStatus,
    pub results: usize,
    pub took_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Full metasearch response returned by `/api/v1/search` and the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub effective_query: String,
    pub category: Category,
    pub page: u32,
    pub took_ms: u64,
    pub cached: bool,
    pub total_results: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub infobox: Option<Infobox>,
    pub results: Vec<SearchResult>,
    pub engines: Vec<EngineReport>,
}

/// Engine metadata and live circuit-breaker health metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineInfo {
    pub name: String,
    pub description: String,
    pub categories: Vec<Category>,
    pub bangs: Vec<String>,
    pub weight: f64,
    pub healthy: bool,
    pub success_count: u64,
    pub error_count: u64,
    pub avg_latency_ms: u64,
    pub cooldown_remaining_secs: u64,
}

/// Autocomplete suggestions payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestResponse {
    pub query: String,
    pub suggestions: Vec<String>,
    pub source: String,
}
