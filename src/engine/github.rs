use std::collections::BTreeMap;

use serde::Deserialize;

use crate::engine::{
    check_http_status, map_reqwest_err, BoxFuture, EngineError, EngineRequest, SearchEngine,
};
use crate::models::{Category, RawResult};
use crate::text::clean_text;

pub struct GitHubEngine;

const CATS: &[Category] = &[Category::Code];
const BANGS: &[&str] = &["!gh", "!github"];

impl SearchEngine for GitHubEngine {
    fn name(&self) -> &'static str {
        "github"
    }

    fn description(&self) -> &'static str {
        "Open-source repositories via GitHub REST Search API"
    }

    fn categories(&self) -> &'static [Category] {
        CATS
    }

    fn bangs(&self) -> &'static [&'static str] {
        BANGS
    }

    fn weight(&self) -> f64 {
        1.25
    }

    fn search<'a>(
        &'a self,
        client: &'a reqwest::Client,
        req: &'a EngineRequest,
    ) -> BoxFuture<'a, Result<Vec<RawResult>, EngineError>> {
        Box::pin(async move {
            let mut request = client
                .get("https://api.github.com/search/repositories")
                .query(&[
                    ("q", req.query.as_str()),
                    ("per_page", "10"),
                    ("page", &req.page.to_string()),
                ])
                .header(reqwest::header::USER_AGENT, &req.user_agent)
                .header(reqwest::header::ACCEPT, "application/vnd.github+json");

            if let Ok(token) = std::env::var("GITHUB_TOKEN") {
                if !token.trim().is_empty() {
                    request = request.bearer_auth(token.trim());
                }
            }

            let resp = request
                .send()
                .await
                .map_err(|e| map_reqwest_err("github", e))?;

            if resp.status().as_u16() == 403 {
                return Err(EngineError::RateLimited("github API rate limit".into()));
            }
            check_http_status(resp.status(), "github")?;
            let body = resp
                .text()
                .await
                .map_err(|e| map_reqwest_err("github", e))?;
            parse_github_json(&body)
        })
    }
}

#[derive(Deserialize)]
struct GhSearchResponse {
    #[serde(default)]
    items: Vec<GhRepo>,
}

#[derive(Deserialize)]
struct GhRepo {
    full_name: String,
    html_url: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    stargazers_count: u64,
    #[serde(default)]
    forks_count: u64,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

pub(crate) fn parse_github_json(body: &str) -> Result<Vec<RawResult>, EngineError> {
    let parsed: GhSearchResponse =
        serde_json::from_str(body).map_err(|e| EngineError::Parse(format!("github: {e}")))?;

    let mut out = Vec::with_capacity(parsed.items.len());
    for (idx, repo) in parsed.items.into_iter().enumerate() {
        let lang = repo.language.unwrap_or_else(|| "Unknown".to_string());
        let desc = repo
            .description
            .map(|d| clean_text(&d))
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "No description provided.".to_string());

        let snippet = format!(
            "[★ {} | Forks {} | {lang}] {desc}",
            repo.stargazers_count, repo.forks_count
        );

        let mut metadata = BTreeMap::new();
        metadata.insert("stars".into(), serde_json::json!(repo.stargazers_count));
        metadata.insert("forks".into(), serde_json::json!(repo.forks_count));
        metadata.insert("language".into(), serde_json::json!(lang));

        out.push(RawResult {
            title: repo.full_name,
            url: repo.html_url,
            snippet,
            engine: "github".to_string(),
            rank: idx + 1,
            category: Category::Code,
            published: repo.updated_at,
            thumbnail: None,
            metadata,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_json_fixture() {
        let json = r#"{
            "items": [{
                "full_name": "tokio-rs/axum",
                "html_url": "https://github.com/tokio-rs/axum",
                "description": "Ergonomic and modular web framework",
                "stargazers_count": 20500,
                "forks_count": 1100,
                "language": "Rust",
                "updated_at": "2026-11-01T00:00:00Z"
            }]
        }"#;
        let res = parse_github_json(json).expect("valid github json");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].title, "tokio-rs/axum");
        assert!(res[0].snippet.contains("20500") && res[0].snippet.contains("Rust"));
    }
}
