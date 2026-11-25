use std::time::Duration;

use crate::models::SuggestResponse;
use crate::text::sanitize_lang;

/// Fetches query autocomplete suggestions from DuckDuckGo, falling back to Wikipedia OpenSearch.
pub async fn fetch_suggestions(
    client: &reqwest::Client,
    query: &str,
    lang: &str,
    user_agent: &str,
) -> SuggestResponse {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return SuggestResponse {
            query: String::new(),
            suggestions: Vec::new(),
            source: "none".to_string(),
        };
    }

    if let Some(items) = try_ddg_suggest(client, trimmed, user_agent).await {
        return SuggestResponse {
            query: trimmed.to_string(),
            suggestions: items,
            source: "duckduckgo".to_string(),
        };
    }

    let safe_lang = sanitize_lang(lang);
    if let Some(items) = try_wiki_suggest(client, trimmed, &safe_lang, user_agent).await {
        return SuggestResponse {
            query: trimmed.to_string(),
            suggestions: items,
            source: "wikipedia".to_string(),
        };
    }

    SuggestResponse {
        query: trimmed.to_string(),
        suggestions: Vec::new(),
        source: "none".to_string(),
    }
}

async fn try_ddg_suggest(
    client: &reqwest::Client,
    query: &str,
    user_agent: &str,
) -> Option<Vec<String>> {
    let resp = tokio::time::timeout(
        Duration::from_millis(2500),
        client
            .get("https://duckduckgo.com/ac/")
            .query(&[("q", query), ("type", "list")])
            .header(reqwest::header::USER_AGENT, user_agent)
            .send(),
    )
    .await
    .ok()?
    .ok()?;

    if !resp.status().is_success() {
        return None;
    }
    let body = resp.text().await.ok()?;
    parse_opensearch_list(&body)
}

async fn try_wiki_suggest(
    client: &reqwest::Client,
    query: &str,
    lang: &str,
    user_agent: &str,
) -> Option<Vec<String>> {
    let url = format!("https://{lang}.wikipedia.org/w/api.php");
    let resp = tokio::time::timeout(
        Duration::from_millis(2500),
        client
            .get(&url)
            .query(&[
                ("action", "opensearch"),
                ("search", query),
                ("limit", "8"),
                ("format", "json"),
            ])
            .header(reqwest::header::USER_AGENT, user_agent)
            .send(),
    )
    .await
    .ok()?
    .ok()?;

    if !resp.status().is_success() {
        return None;
    }
    let body = resp.text().await.ok()?;
    parse_opensearch_list(&body)
}

pub(crate) fn parse_opensearch_list(body: &str) -> Option<Vec<String>> {
    let val: serde_json::Value = serde_json::from_str(body).ok()?;
    let arr = val.as_array()?;
    let list = arr.get(1)?.as_array()?;
    let suggestions: Vec<String> = list
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .take(10)
        .collect();
    if suggestions.is_empty() {
        None
    } else {
        Some(suggestions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_opensearch_array() {
        let json = r#"["rust",["rust programming","rust language","rustup"]]"#;
        let items = parse_opensearch_list(json).expect("valid array");
        assert_eq!(items, vec!["rust programming", "rust language", "rustup"]);
    }
}
