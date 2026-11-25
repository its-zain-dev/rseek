use std::collections::BTreeMap;

use crate::engine::{
    check_http_status, map_reqwest_err, BoxFuture, EngineError, EngineRequest, SearchEngine,
};
use crate::models::{Category, RawResult};
use crate::text::{extract_all_xml_tags, extract_xml_blocks, extract_xml_tag, truncate_chars};

pub struct ArxivEngine;

const CATS: &[Category] = &[Category::Science];
const BANGS: &[&str] = &["!arxiv", "!paper"];

impl SearchEngine for ArxivEngine {
    fn name(&self) -> &'static str {
        "arxiv"
    }

    fn description(&self) -> &'static str {
        "Scientific preprints & papers via arXiv Atom API"
    }

    fn categories(&self) -> &'static [Category] {
        CATS
    }

    fn bangs(&self) -> &'static [&'static str] {
        BANGS
    }

    fn weight(&self) -> f64 {
        1.2
    }

    fn search<'a>(
        &'a self,
        client: &'a reqwest::Client,
        req: &'a EngineRequest,
    ) -> BoxFuture<'a, Result<Vec<RawResult>, EngineError>> {
        Box::pin(async move {
            let start = req.page.saturating_sub(1) * 10;
            let search_query = format!("all:{}", req.query);

            let resp = client
                .get("https://export.arxiv.org/api/query")
                .query(&[
                    ("search_query", search_query.as_str()),
                    ("start", &start.to_string()),
                    ("max_results", "10"),
                ])
                .header(reqwest::header::USER_AGENT, &req.user_agent)
                .send()
                .await
                .map_err(|e| map_reqwest_err("arxiv", e))?;

            check_http_status(resp.status(), "arxiv")?;
            let body = resp.text().await.map_err(|e| map_reqwest_err("arxiv", e))?;
            Ok(parse_arxiv_atom(&body))
        })
    }
}

pub(crate) fn parse_arxiv_atom(xml: &str) -> Vec<RawResult> {
    let entries = extract_xml_blocks(xml, "entry");
    let mut out = Vec::with_capacity(entries.len());

    for (idx, entry) in entries.into_iter().enumerate() {
        let Some(title) = extract_xml_tag(entry, "title") else {
            continue;
        };
        let Some(url) = extract_xml_tag(entry, "id") else {
            continue;
        };
        let summary = extract_xml_tag(entry, "summary").unwrap_or_default();
        let published = extract_xml_tag(entry, "published");
        let authors = extract_all_xml_tags(entry, "name");

        let snippet = if authors.is_empty() {
            truncate_chars(&summary, 280)
        } else {
            let author_line = authors
                .iter()
                .take(4)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{author_line}] {}", truncate_chars(&summary, 220))
        };

        let mut metadata = BTreeMap::new();
        if !authors.is_empty() {
            metadata.insert("authors".into(), serde_json::json!(authors));
        }

        out.push(RawResult {
            title,
            url,
            snippet,
            engine: "arxiv".to_string(),
            rank: idx + 1,
            category: Category::Science,
            published,
            thumbnail: None,
            metadata,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_arxiv_atom_fixture() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
          <entry>
            <id>https://arxiv.org/abs/1706.03762</id>
            <published>2017-06-12T17:57:34Z</published>
            <title>Attention Is All You Need</title>
            <summary>The dominant sequence transduction models are based on complex recurrent networks.</summary>
            <author><name>Ashish Vaswani</name></author>
            <author><name>Noam Shazeer</name></author>
          </entry>
        </feed>"#;
        let res = parse_arxiv_atom(xml);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].title, "Attention Is All You Need");
        assert_eq!(res[0].url, "https://arxiv.org/abs/1706.03762");
        assert!(res[0].snippet.contains("Ashish Vaswani"));
    }
}
