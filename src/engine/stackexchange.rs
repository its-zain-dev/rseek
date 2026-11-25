use std::collections::BTreeMap;

use serde::Deserialize;

use crate::engine::{check_http_status, map_reqwest_err, BoxFuture, EngineError, EngineRequest, SearchEngine};
use crate::models::{Category, RawResult};
use crate::text::clean_text;

pub struct StackExchangeEngine;

const CATS: &[Category] = &[Category::Code];
const BANGS: &[&str] = &["!so"];

impl SearchEngine for StackExchangeEngine {
    fn name(&self) -> &'static str {
        "stackexchange"
    }

    fn description(&self) -> &'static str {
        "Developer Q&A via Stack Exchange (Stack Overflow) API"
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
            let resp = client
                .get("https://api.stackexchange.com/2.3/search/advanced")
                .query(&[
                    ("order", "desc"),
                    ("sort", "relevance"),
                    ("q", req.query.as_str()),
                    ("site", "stackoverflow"),
                    ("pagesize", "10"),
                    ("page", &req.page.to_string()),
                ])
                .header(reqwest::header::USER_AGENT, &req.user_agent)
                .send()
                .await
                .map_err(|e| map_reqwest_err("stackexchange", e))?;

            check_http_status(resp.status(), "stackexchange")?;
            let body = resp
                .text()
                .await
                .map_err(|e| map_reqwest_err("stackexchange", e))?;
            parse_stackexchange_json(&body)
        })
    }
}

#[derive(Deserialize)]
struct SeResponse {
    #[serde(default)]
    items: Vec<SeItem>,
}

#[derive(Deserialize)]
struct SeItem {
    title: String,
    link: String,
    #[serde(default)]
    score: i64,
    #[serde(default)]
    answer_count: u64,
    #[serde(default)]
    is_answered: bool,
    #[serde(default)]
    tags: Vec<String>,
}

pub(crate) fn parse_stackexchange_json(body: &str) -> Result<Vec<RawResult>, EngineError> {
    let parsed: SeResponse = serde_json::from_str(body)
        .map_err(|e| EngineError::Parse(format!("stackexchange: {e}")))?;

    let mut out = Vec::with_capacity(parsed.items.len());
    for (idx, item) in parsed.items.into_iter().enumerate() {
        let solved = if item.is_answered { "✓ solved" } else { "open" };
        let tags_str = item.tags.join(", ");
        let snippet = format!(
            "Score: {} | Answers: {} ({solved}) | Tags: [{tags_str}]",
            item.score, item.answer_count
        );

        let mut metadata = BTreeMap::new();
        metadata.insert("score".into(), serde_json::json!(item.score));
        metadata.insert("answers".into(), serde_json::json!(item.answer_count));
        metadata.insert("answered".into(), serde_json::json!(item.is_answered));
        metadata.insert("tags".into(), serde_json::json!(item.tags));

        out.push(RawResult {
            title: clean_text(&item.title),
            url: item.link,
            snippet,
            engine: "stackexchange".to_string(),
            rank: idx + 1,
            category: Category::Code,
            published: None,
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
    fn parses_stackexchange_json_fixture() {
        let json = r#"{
            "items": [{
                "title": "How to implement async traits in Rust &amp; Tokio?",
                "link": "https://stackoverflow.com/questions/123",
                "score": 45,
                "answer_count": 3,
                "is_answered": true,
                "tags": ["rust", "tokio"]
            }]
        }"#;
        let res = parse_stackexchange_json(json).expect("valid se json");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].title, "How to implement async traits in Rust & Tokio?");
        assert!(res[0].snippet.contains("solved"));
    }
}
