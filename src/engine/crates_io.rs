use std::collections::BTreeMap;

use serde::Deserialize;

use crate::engine::{
    check_http_status, map_reqwest_err, BoxFuture, EngineError, EngineRequest, SearchEngine,
};
use crate::models::{Category, RawResult};
use crate::text::clean_text;

pub struct CratesIoEngine;

const CATS: &[Category] = &[Category::Packages, Category::Code];
const BANGS: &[&str] = &["!crates", "!crate", "!rs"];

impl SearchEngine for CratesIoEngine {
    fn name(&self) -> &'static str {
        "crates"
    }

    fn description(&self) -> &'static str {
        "Rust packages via crates.io Registry API"
    }

    fn categories(&self) -> &'static [Category] {
        CATS
    }

    fn bangs(&self) -> &'static [&'static str] {
        BANGS
    }

    fn weight(&self) -> f64 {
        1.1
    }

    fn search<'a>(
        &'a self,
        client: &'a reqwest::Client,
        req: &'a EngineRequest,
    ) -> BoxFuture<'a, Result<Vec<RawResult>, EngineError>> {
        Box::pin(async move {
            let resp = client
                .get("https://crates.io/api/v1/crates")
                .query(&[
                    ("q", req.query.as_str()),
                    ("per_page", "10"),
                    ("page", &req.page.to_string()),
                ])
                .header(reqwest::header::USER_AGENT, &req.user_agent)
                .send()
                .await
                .map_err(|e| map_reqwest_err("crates", e))?;

            check_http_status(resp.status(), "crates")?;
            let body = resp
                .text()
                .await
                .map_err(|e| map_reqwest_err("crates", e))?;
            parse_crates_json(&body)
        })
    }
}

#[derive(Deserialize)]
struct CratesResponse {
    #[serde(default)]
    crates: Vec<CrateEntry>,
}

#[derive(Deserialize)]
struct CrateEntry {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    recent_downloads: Option<u64>,
    #[serde(default)]
    newest_version: String,
    #[serde(default)]
    repository: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

pub(crate) fn parse_crates_json(body: &str) -> Result<Vec<RawResult>, EngineError> {
    let parsed: CratesResponse =
        serde_json::from_str(body).map_err(|e| EngineError::Parse(format!("crates: {e}")))?;

    let mut out = Vec::with_capacity(parsed.crates.len());
    for (idx, c) in parsed.crates.into_iter().enumerate() {
        let desc = c
            .description
            .map(|d| clean_text(&d))
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "No description provided.".to_string());

        let snippet = format!("[v{} | {} downloads] {desc}", c.newest_version, c.downloads);

        let mut metadata = BTreeMap::new();
        metadata.insert("version".into(), serde_json::json!(c.newest_version));
        metadata.insert("downloads".into(), serde_json::json!(c.downloads));
        if let Some(recent) = c.recent_downloads {
            metadata.insert("recent_downloads".into(), serde_json::json!(recent));
        }
        if let Some(repo) = c.repository {
            metadata.insert("repository".into(), serde_json::json!(repo));
        }

        out.push(RawResult {
            title: format!("{} (v{})", c.name, c.newest_version),
            url: format!("https://crates.io/crates/{}", c.name),
            snippet,
            engine: "crates".to_string(),
            rank: idx + 1,
            category: Category::Packages,
            published: c.updated_at,
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
    fn parses_crates_json_fixture() {
        let json = r#"{
            "crates": [{
                "name": "axum",
                "description": "Web framework that focuses on ergonomics",
                "downloads": 55000000,
                "recent_downloads": 8000000,
                "newest_version": "0.7.9",
                "repository": "https://github.com/tokio-rs/axum",
                "updated_at": "2026-10-01T00:00:00Z"
            }]
        }"#;
        let res = parse_crates_json(json).expect("valid crates json");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].url, "https://crates.io/crates/axum");
        assert!(res[0].snippet.contains("v0.7.9"));
    }
}
