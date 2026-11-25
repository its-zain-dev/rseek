use std::collections::BTreeMap;

use serde::Deserialize;

use crate::engine::{
    check_http_status, map_reqwest_err, BoxFuture, EngineError, EngineRequest, SearchEngine,
};
use crate::models::{Category, RawResult};
use crate::text::clean_text;

pub struct NpmEngine;

const CATS: &[Category] = &[Category::Packages, Category::Code];
const BANGS: &[&str] = &["!npm", "!js"];

impl SearchEngine for NpmEngine {
    fn name(&self) -> &'static str {
        "npm"
    }

    fn description(&self) -> &'static str {
        "JavaScript & TypeScript packages via npm Registry API"
    }

    fn categories(&self) -> &'static [Category] {
        CATS
    }

    fn bangs(&self) -> &'static [&'static str] {
        BANGS
    }

    fn weight(&self) -> f64 {
        1.0
    }

    fn search<'a>(
        &'a self,
        client: &'a reqwest::Client,
        req: &'a EngineRequest,
    ) -> BoxFuture<'a, Result<Vec<RawResult>, EngineError>> {
        Box::pin(async move {
            let from = req.page.saturating_sub(1) * 10;
            let resp = client
                .get("https://registry.npmjs.org/-/v1/search")
                .query(&[
                    ("text", req.query.as_str()),
                    ("size", "10"),
                    ("from", &from.to_string()),
                ])
                .header(reqwest::header::USER_AGENT, &req.user_agent)
                .send()
                .await
                .map_err(|e| map_reqwest_err("npm", e))?;

            check_http_status(resp.status(), "npm")?;
            let body = resp.text().await.map_err(|e| map_reqwest_err("npm", e))?;
            parse_npm_json(&body)
        })
    }
}

#[derive(Deserialize)]
struct NpmResponse {
    #[serde(default)]
    objects: Vec<NpmObject>,
}

#[derive(Deserialize)]
struct NpmObject {
    package: NpmPackage,
}

#[derive(Deserialize)]
struct NpmPackage {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    links: Option<NpmLinks>,
}

#[derive(Deserialize)]
struct NpmLinks {
    #[serde(default)]
    npm: Option<String>,
    #[serde(default)]
    repository: Option<String>,
}

pub(crate) fn parse_npm_json(body: &str) -> Result<Vec<RawResult>, EngineError> {
    let parsed: NpmResponse =
        serde_json::from_str(body).map_err(|e| EngineError::Parse(format!("npm: {e}")))?;

    let mut out = Vec::with_capacity(parsed.objects.len());
    for (idx, obj) in parsed.objects.into_iter().enumerate() {
        let pkg = obj.package;
        let desc = pkg
            .description
            .map(|d| clean_text(&d))
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "No description provided.".to_string());

        let url = pkg
            .links
            .as_ref()
            .and_then(|l| l.npm.clone())
            .unwrap_or_else(|| format!("https://www.npmjs.com/package/{}", pkg.name));

        let mut metadata = BTreeMap::new();
        metadata.insert("version".into(), serde_json::json!(pkg.version));
        if let Some(repo) = pkg.links.and_then(|l| l.repository) {
            metadata.insert("repository".into(), serde_json::json!(repo));
        }

        out.push(RawResult {
            title: format!("{} (v{})", pkg.name, pkg.version),
            url,
            snippet: format!("[npm v{}] {desc}", pkg.version),
            engine: "npm".to_string(),
            rank: idx + 1,
            category: Category::Packages,
            published: pkg.date,
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
    fn parses_npm_json_fixture() {
        let json = r#"{
            "objects": [{
                "package": {
                    "name": "minisearch",
                    "version": "7.1.0",
                    "description": "Tiny and powerful full-text search engine",
                    "date": "2026-08-12T00:00:00Z",
                    "links": {
                        "npm": "https://www.npmjs.com/package/minisearch",
                        "repository": "https://github.com/lucaong/minisearch"
                    }
                }
            }]
        }"#;
        let res = parse_npm_json(json).expect("valid npm json");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].url, "https://www.npmjs.com/package/minisearch");
        assert!(res[0].snippet.contains("v7.1.0"));
    }
}
