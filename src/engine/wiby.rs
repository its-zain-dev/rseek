use std::collections::BTreeMap;

use serde::Deserialize;

use crate::engine::{check_http_status, map_reqwest_err, BoxFuture, EngineError, EngineRequest, SearchEngine};
use crate::models::{Category, RawResult};
use crate::text::clean_text;

pub struct WibyEngine;

const CATS: &[Category] = &[Category::General];
const BANGS: &[&str] = &["!wiby"];

impl SearchEngine for WibyEngine {
    fn name(&self) -> &'static str {
        "wiby"
    }

    fn description(&self) -> &'static str {
        "Independent small-web search via Wiby JSON API"
    }

    fn categories(&self) -> &'static [Category] {
        CATS
    }

    fn bangs(&self) -> &'static [&'static str] {
        BANGS
    }

    fn weight(&self) -> f64 {
        0.85
    }

    fn search<'a>(
        &'a self,
        client: &'a reqwest::Client,
        req: &'a EngineRequest,
    ) -> BoxFuture<'a, Result<Vec<RawResult>, EngineError>> {
        Box::pin(async move {
            if req.page > 1 {
                return Ok(Vec::new());
            }

            let resp = client
                .get("https://wiby.me/json/")
                .query(&[("q", req.query.as_str())])
                .header(reqwest::header::USER_AGENT, &req.user_agent)
                .send()
                .await
                .map_err(|e| map_reqwest_err("wiby", e))?;

            check_http_status(resp.status(), "wiby")?;
            let body = resp.text().await.map_err(|e| map_reqwest_err("wiby", e))?;
            parse_wiby_json(&body)
        })
    }
}

#[derive(Deserialize)]
struct WibyItem {
    #[serde(rename = "URL", default)]
    url: String,
    #[serde(rename = "Title", default)]
    title: String,
    #[serde(rename = "Snippet", default)]
    snippet: String,
    #[serde(rename = "Description", default)]
    description: String,
}

pub(crate) fn parse_wiby_json(body: &str) -> Result<Vec<RawResult>, EngineError> {
    if body.trim().is_empty() {
        return Ok(Vec::new());
    }
    let items: Vec<WibyItem> =
        serde_json::from_str(body).map_err(|e| EngineError::Parse(format!("wiby: {e}")))?;

    let mut out = Vec::new();
    for (idx, item) in items.into_iter().take(10).enumerate() {
        let title = clean_text(&item.title);
        let url = item.url.trim().to_string();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        let raw_snip = if item.description.trim().is_empty() {
            item.snippet
        } else {
            item.description
        };
        out.push(RawResult {
            title,
            url,
            snippet: clean_text(&raw_snip),
            engine: "wiby".to_string(),
            rank: idx + 1,
            category: Category::General,
            published: None,
            thumbnail: None,
            metadata: BTreeMap::new(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wiby_json_fixture() {
        let json = r#"[{"URL":"https://rust-osdev.com/","Title":"Rust OSDev","Snippet":"Operating systems in Rust","Description":"Writing an OS in Rust"}]"#;
        let res = parse_wiby_json(json).expect("valid wiby json");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].title, "Rust OSDev");
        assert_eq!(res[0].snippet, "Writing an OS in Rust");
    }
}
