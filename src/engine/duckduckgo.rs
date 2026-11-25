use std::collections::BTreeMap;

use scraper::{Html, Selector};
use serde::Deserialize;

use crate::engine::{check_http_status, map_reqwest_err, BoxFuture, EngineError, EngineRequest, SearchEngine};
use crate::models::{Category, Infobox, RawResult};
use crate::text::{clean_text, unwrap_ddg_url};

pub struct DuckDuckGoEngine;

const CATS: &[Category] = &[Category::General];
const BANGS: &[&str] = &["!g", "!ddg"];

impl SearchEngine for DuckDuckGoEngine {
    fn name(&self) -> &'static str {
        "duckduckgo"
    }

    fn description(&self) -> &'static str {
        "Web search via DuckDuckGo HTML (zero API key)"
    }

    fn categories(&self) -> &'static [Category] {
        CATS
    }

    fn bangs(&self) -> &'static [&'static str] {
        BANGS
    }

    fn weight(&self) -> f64 {
        1.3
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
                .get("https://html.duckduckgo.com/html/")
                .query(&[("q", req.query.as_str())])
                .header(reqwest::header::USER_AGENT, &req.browser_user_agent)
                .header(
                    reqwest::header::ACCEPT,
                    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                )
                .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
                .send()
                .await
                .map_err(|e| map_reqwest_err("duckduckgo", e))?;

            if resp.status().as_u16() == 403 {
                return Err(EngineError::RateLimited("duckduckgo HTTP 403".into()));
            }
            check_http_status(resp.status(), "duckduckgo")?;

            let body = resp
                .text()
                .await
                .map_err(|e| map_reqwest_err("duckduckgo", e))?;

            parse_ddg_html(&body)
        })
    }
}

pub(crate) fn parse_ddg_html(html: &str) -> Result<Vec<RawResult>, EngineError> {
    if html.contains("anomaly-modal")
        || html.contains("challenge-form")
        || html.contains("bots use DuckDuckGo too")
        || html.contains("<title>Captcha")
    {
        return Err(EngineError::RateLimited(
            "upstream challenge/rate-limit detected".into(),
        ));
    }

    let document = Html::parse_document(html);
    let result_sel = Selector::parse("div.result, div.web-result")
        .map_err(|e| EngineError::Parse(format!("selector: {e:?}")))?;
    let title_sel = Selector::parse("a.result__a")
        .map_err(|e| EngineError::Parse(format!("selector: {e:?}")))?;
    let snippet_sel = Selector::parse(".result__snippet")
        .map_err(|e| EngineError::Parse(format!("selector: {e:?}")))?;

    let mut results = Vec::new();
    for (idx, item) in document.select(&result_sel).enumerate() {
        let Some(title_el) = item.select(&title_sel).next() else {
            continue;
        };
        let Some(href) = title_el.value().attr("href") else {
            continue;
        };
        let Some(url) = unwrap_ddg_url(href) else {
            continue;
        };
        let title = clean_text(&title_el.text().collect::<Vec<_>>().join(" "));
        if title.is_empty() {
            continue;
        }
        let snippet = item
            .select(&snippet_sel)
            .next()
            .map(|el| clean_text(&el.text().collect::<Vec<_>>().join(" ")))
            .unwrap_or_default();

        results.push(RawResult {
            title,
            url,
            snippet,
            engine: "duckduckgo".to_string(),
            rank: idx + 1,
            category: Category::General,
            published: None,
            thumbnail: None,
            metadata: BTreeMap::new(),
        });
    }

    Ok(results)
}

#[derive(Deserialize)]
struct DdgInstantAnswer {
    #[serde(rename = "Heading", default)]
    heading: String,
    #[serde(rename = "AbstractText", default)]
    abstract_text: String,
    #[serde(rename = "AbstractURL", default)]
    abstract_url: String,
    #[serde(rename = "AbstractSource", default)]
    abstract_source: String,
}

/// Best-effort fetch of a DuckDuckGo Instant Answer infobox card.
pub async fn fetch_infobox(
    client: &reqwest::Client,
    query: &str,
    user_agent: &str,
) -> Option<Infobox> {
    let resp = client
        .get("https://api.duckduckgo.com/")
        .query(&[
            ("q", query),
            ("format", "json"),
            ("no_html", "1"),
            ("skip_disambig", "1"),
        ])
        .header(reqwest::header::USER_AGENT, user_agent)
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data: DdgInstantAnswer = resp.json().await.ok()?;
    let abstract_text = clean_text(&data.abstract_text);
    if abstract_text.is_empty() || data.abstract_url.trim().is_empty() {
        return None;
    }

    let title = if data.heading.trim().is_empty() {
        query.to_string()
    } else {
        clean_text(&data.heading)
    };

    Some(Infobox {
        title,
        abstract_text,
        url: data.abstract_url,
        source: if data.abstract_source.trim().is_empty() {
            "DuckDuckGo".to_string()
        } else {
            data.abstract_source
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ddg_html_fixture_and_filters_ads() {
        let html = r#"
        <html><body>
          <div class="result results_links">
            <h2 class="result__title">
              <a class="result__a" href="https://duckduckgo.com/y.js?ad_provider=test">Sponsored Ad</a>
            </h2>
          </div>
          <div class="result results_links">
            <h2 class="result__title">
              <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org%2F&amp;rut=1">Rust Programming Language</a>
            </h2>
            <a class="result__snippet">A language empowering everyone to build reliable and efficient software.</a>
          </div>
        </body></html>
        "#;

        let parsed = parse_ddg_html(html).expect("valid HTML should parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].title, "Rust Programming Language");
        assert_eq!(parsed[0].url, "https://rust-lang.org/");
        assert!(parsed[0].snippet.contains("reliable and efficient"));
    }

    #[test]
    fn detects_captcha_challenge() {
        let html = r#"<html><head><title>Captcha</title></head><body><div class="anomaly-modal"></div></body></html>"#;
        let err = parse_ddg_html(html).expect_err("challenge should return RateLimited");
        assert!(matches!(err, EngineError::RateLimited(_)));
    }
}
