use std::collections::BTreeMap;

use serde::Deserialize;

use crate::engine::{
    check_http_status, map_reqwest_err, BoxFuture, EngineError, EngineRequest, SearchEngine,
};
use crate::models::{Category, RawResult};
use crate::text::{clean_text, sanitize_lang};

pub struct WikipediaEngine;

const CATS: &[Category] = &[Category::General, Category::Wiki];
const BANGS: &[&str] = &["!w", "!wiki"];

impl SearchEngine for WikipediaEngine {
    fn name(&self) -> &'static str {
        "wikipedia"
    }

    fn description(&self) -> &'static str {
        "Encyclopedia articles via MediaWiki Action API"
    }

    fn categories(&self) -> &'static [Category] {
        CATS
    }

    fn bangs(&self) -> &'static [&'static str] {
        BANGS
    }

    fn weight(&self) -> f64 {
        1.15
    }

    fn search<'a>(
        &'a self,
        client: &'a reqwest::Client,
        req: &'a EngineRequest,
    ) -> BoxFuture<'a, Result<Vec<RawResult>, EngineError>> {
        Box::pin(async move {
            let lang = sanitize_lang(&req.lang);
            let offset = req.page.saturating_sub(1) * 10;
            let endpoint = format!("https://{lang}.wikipedia.org/w/api.php");

            let resp = client
                .get(&endpoint)
                .query(&[
                    ("action", "query"),
                    ("list", "search"),
                    ("srsearch", req.query.as_str()),
                    ("srlimit", "10"),
                    ("sroffset", &offset.to_string()),
                    ("format", "json"),
                    ("utf8", "1"),
                ])
                .header(reqwest::header::USER_AGENT, &req.user_agent)
                .send()
                .await
                .map_err(|e| map_reqwest_err("wikipedia", e))?;

            check_http_status(resp.status(), "wikipedia")?;
            let body = resp
                .text()
                .await
                .map_err(|e| map_reqwest_err("wikipedia", e))?;
            parse_wikipedia_json(&body, &lang)
        })
    }
}

#[derive(Deserialize)]
struct WikiResponse {
    query: Option<WikiQuery>,
}

#[derive(Deserialize)]
struct WikiQuery {
    #[serde(default)]
    search: Vec<WikiHit>,
}

#[derive(Deserialize)]
struct WikiHit {
    title: String,
    pageid: u64,
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    wordcount: u64,
    #[serde(default)]
    timestamp: Option<String>,
}

pub(crate) fn parse_wikipedia_json(body: &str, lang: &str) -> Result<Vec<RawResult>, EngineError> {
    let parsed: WikiResponse =
        serde_json::from_str(body).map_err(|e| EngineError::Parse(format!("wikipedia: {e}")))?;
    let hits = parsed.query.map(|q| q.search).unwrap_or_default();

    let mut out = Vec::with_capacity(hits.len());
    for (idx, hit) in hits.into_iter().enumerate() {
        let mut metadata = BTreeMap::new();
        metadata.insert("wordcount".to_string(), serde_json::json!(hit.wordcount));
        out.push(RawResult {
            title: clean_text(&hit.title),
            url: format!("https://{lang}.wikipedia.org/?curid={}", hit.pageid),
            snippet: clean_text(&hit.snippet),
            engine: "wikipedia".to_string(),
            rank: idx + 1,
            category: Category::Wiki,
            published: hit.timestamp,
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
    fn parses_wikipedia_json_fixture() {
        let json = r#"{
            "query": {
                "search": [{
                    "title": "Rust (programming language)",
                    "pageid": 29414838,
                    "wordcount": 11088,
                    "snippet": "<span class=\"searchmatch\">Rust</span> is fast &amp; safe.",
                    "timestamp": "2026-10-01T12:00:00Z"
                }]
            }
        }"#;
        let res = parse_wikipedia_json(json, "en").expect("valid json");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].url, "https://en.wikipedia.org/?curid=29414838");
        assert_eq!(res[0].snippet, "Rust is fast & safe.");
    }
}
