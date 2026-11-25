use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use crate::engine::{
    check_http_status, map_reqwest_err, BoxFuture, EngineError, EngineRequest, SearchEngine,
};
use crate::models::{Category, RawResult};
use crate::text::clean_text;

pub struct WikimediaEngine;

const CATS: &[Category] = &[Category::Images];
const BANGS: &[&str] = &["!img", "!images"];

impl SearchEngine for WikimediaEngine {
    fn name(&self) -> &'static str {
        "wikimedia"
    }

    fn description(&self) -> &'static str {
        "Open-license images via Wikimedia Commons API"
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
            let offset = req.page.saturating_sub(1) * 12;
            let resp = client
                .get("https://commons.wikimedia.org/w/api.php")
                .query(&[
                    ("action", "query"),
                    ("generator", "search"),
                    ("gsrnamespace", "6"),
                    ("gsrsearch", req.query.as_str()),
                    ("gsrlimit", "12"),
                    ("gsroffset", &offset.to_string()),
                    ("prop", "imageinfo"),
                    ("iiprop", "url|size|mime"),
                    ("iiurlwidth", "640"),
                    ("format", "json"),
                ])
                .header(reqwest::header::USER_AGENT, &req.user_agent)
                .send()
                .await
                .map_err(|e| map_reqwest_err("wikimedia", e))?;

            check_http_status(resp.status(), "wikimedia")?;
            let body = resp
                .text()
                .await
                .map_err(|e| map_reqwest_err("wikimedia", e))?;
            parse_wikimedia_json(&body)
        })
    }
}

#[derive(Deserialize)]
struct WmResponse {
    query: Option<WmQuery>,
}

#[derive(Deserialize)]
struct WmQuery {
    #[serde(default)]
    pages: HashMap<String, WmPage>,
}

#[derive(Deserialize)]
struct WmPage {
    #[serde(default)]
    title: String,
    #[serde(default)]
    index: usize,
    #[serde(default)]
    imageinfo: Vec<WmImageInfo>,
}

#[derive(Deserialize)]
struct WmImageInfo {
    #[serde(default)]
    url: String,
    #[serde(default)]
    thumburl: Option<String>,
    #[serde(default)]
    descriptionurl: Option<String>,
    #[serde(default)]
    width: u64,
    #[serde(default)]
    height: u64,
    #[serde(default)]
    mime: String,
}

pub(crate) fn parse_wikimedia_json(body: &str) -> Result<Vec<RawResult>, EngineError> {
    let parsed: WmResponse =
        serde_json::from_str(body).map_err(|e| EngineError::Parse(format!("wikimedia: {e}")))?;
    let mut pages: Vec<WmPage> = parsed
        .query
        .map(|q| q.pages.into_values().collect())
        .unwrap_or_default();

    pages.sort_by(|a, b| a.index.cmp(&b.index).then_with(|| a.title.cmp(&b.title)));

    let mut out = Vec::new();
    for (idx, page) in pages.into_iter().enumerate() {
        let Some(info) = page.imageinfo.into_iter().next() else {
            continue;
        };
        if info.url.is_empty() {
            continue;
        }
        let title = clean_text(page.title.strip_prefix("File:").unwrap_or(&page.title));
        let page_url = info.descriptionurl.unwrap_or_else(|| info.url.clone());
        let thumb = info.thumburl.unwrap_or_else(|| info.url.clone());

        let mut metadata = BTreeMap::new();
        metadata.insert("image_url".into(), serde_json::json!(info.url));
        metadata.insert("width".into(), serde_json::json!(info.width));
        metadata.insert("height".into(), serde_json::json!(info.height));
        metadata.insert("mime".into(), serde_json::json!(info.mime));

        out.push(RawResult {
            title,
            url: page_url,
            snippet: format!("{}x{} ({})", info.width, info.height, info.mime),
            engine: "wikimedia".to_string(),
            rank: idx + 1,
            category: Category::Images,
            published: None,
            thumbnail: Some(thumb),
            metadata,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wikimedia_json_fixture() {
        let json = r#"{
            "query": {
                "pages": {
                    "99": {
                        "title": "File:Ferris the crab.svg",
                        "index": 1,
                        "imageinfo": [{
                            "url": "https://upload.wikimedia.org/ferris.svg",
                            "thumburl": "https://upload.wikimedia.org/thumb/ferris.png",
                            "descriptionurl": "https://commons.wikimedia.org/wiki/File:Ferris",
                            "width": 800,
                            "height": 600,
                            "mime": "image/svg+xml"
                        }]
                    }
                }
            }
        }"#;
        let res = parse_wikimedia_json(json).expect("valid wikimedia json");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].title, "Ferris the crab.svg");
        assert_eq!(
            res[0].thumbnail.as_deref(),
            Some("https://upload.wikimedia.org/thumb/ferris.png")
        );
    }
}
