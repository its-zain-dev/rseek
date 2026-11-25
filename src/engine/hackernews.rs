use std::collections::BTreeMap;

use serde::Deserialize;

use crate::engine::{check_http_status, map_reqwest_err, BoxFuture, EngineError, EngineRequest, SearchEngine};
use crate::models::{Category, RawResult};
use crate::text::{clean_text, truncate_chars};

pub struct HackerNewsEngine;

const CATS: &[Category] = &[Category::General, Category::News];
const BANGS: &[&str] = &["!hn"];

impl SearchEngine for HackerNewsEngine {
    fn name(&self) -> &'static str {
        "hackernews"
    }

    fn description(&self) -> &'static str {
        "Tech news & discussions via Hacker News Algolia API"
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
            let page_zero = req.page.saturating_sub(1);
            let resp = client
                .get("https://hn.algolia.com/api/v1/search")
                .query(&[
                    ("query", req.query.as_str()),
                    ("hitsPerPage", "10"),
                    ("page", &page_zero.to_string()),
                ])
                .header(reqwest::header::USER_AGENT, &req.user_agent)
                .send()
                .await
                .map_err(|e| map_reqwest_err("hackernews", e))?;

            check_http_status(resp.status(), "hackernews")?;
            let body = resp
                .text()
                .await
                .map_err(|e| map_reqwest_err("hackernews", e))?;
            parse_hn_json(&body)
        })
    }
}

#[derive(Deserialize)]
struct HnResponse {
    #[serde(default)]
    hits: Vec<HnHit>,
}

#[derive(Deserialize)]
struct HnHit {
    #[serde(rename = "objectID", default)]
    object_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    story_title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    story_url: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    points: Option<i64>,
    #[serde(default)]
    num_comments: Option<i64>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    story_text: Option<String>,
}

pub(crate) fn parse_hn_json(body: &str) -> Result<Vec<RawResult>, EngineError> {
    let parsed: HnResponse =
        serde_json::from_str(body).map_err(|e| EngineError::Parse(format!("hackernews: {e}")))?;

    let mut out = Vec::new();
    for (idx, hit) in parsed.hits.into_iter().enumerate() {
        let title = hit
            .title
            .or(hit.story_title)
            .map(|t| clean_text(&t))
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        let hn_url = format!("https://news.ycombinator.com/item?id={}", hit.object_id);
        let url = hit
            .url
            .or(hit.story_url)
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(|| hn_url.clone());

        let points = hit.points.unwrap_or(0);
        let comments = hit.num_comments.unwrap_or(0);
        let author = hit.author.unwrap_or_else(|| "unknown".to_string());
        let snippet = match hit.story_text.map(|s| clean_text(&s)).filter(|s| !s.is_empty()) {
            Some(text) => format!(
                "{points} pts | {comments} comments | by {author} — {}",
                truncate_chars(&text, 160)
            ),
            None => format!("{points} pts | {comments} comments | by {author} on Hacker News"),
        };

        let mut metadata = BTreeMap::new();
        metadata.insert("points".into(), serde_json::json!(points));
        metadata.insert("comments".into(), serde_json::json!(comments));
        metadata.insert("author".into(), serde_json::json!(author));
        metadata.insert("hn_url".into(), serde_json::json!(hn_url));

        out.push(RawResult {
            title,
            url,
            snippet,
            engine: "hackernews".to_string(),
            rank: idx + 1,
            category: Category::News,
            published: hit.created_at,
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
    fn parses_hn_algolia_fixture() {
        let json = r#"{
            "hits": [{
                "objectID": "42",
                "title": "Writing a Fast Search Engine in Rust",
                "url": "https://example.org/rust-search",
                "author": "alice",
                "points": 310,
                "num_comments": 88,
                "created_at": "2026-10-10T10:00:00Z"
            }]
        }"#;
        let res = parse_hn_json(json).expect("valid hn json");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].url, "https://example.org/rust-search");
        assert!(res[0].snippet.contains("310 pts | 88 comments | by alice"));
    }
}
