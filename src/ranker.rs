use std::collections::{BTreeMap, HashMap};

use crate::models::{Category, RawResult, SearchResult};
use crate::text::canonicalize_url;

struct Group {
    title: String,
    url: String,
    snippet: String,
    engines: Vec<String>,
    category: Category,
    published: Option<String>,
    thumbnail: Option<String>,
    metadata: BTreeMap<String, serde_json::Value>,
    rrf_score: f64,
}

/// Deduplicates raw engine results by canonical URL and scores them using
/// weighted Reciprocal Rank Fusion (RRF), multi-engine consensus, and term matching.
pub fn rank_and_dedupe(
    query: &str,
    raw: Vec<RawResult>,
    weights: &HashMap<String, f64>,
    limit: usize,
) -> Vec<SearchResult> {
    if raw.is_empty() || limit == 0 {
        return Vec::new();
    }

    let query_tokens: Vec<String> = query
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase())
        .filter(|t| t.len() >= 2)
        .collect();

    let mut groups: HashMap<String, Group> = HashMap::new();

    for item in raw {
        if item.url.trim().is_empty() || item.title.trim().is_empty() {
            continue;
        }
        let canon = canonicalize_url(&item.url);
        let weight = weights.get(&item.engine).copied().unwrap_or(1.0);
        let contribution = weight / (60.0 + item.rank as f64);

        if let Some(existing) = groups.get_mut(&canon) {
            existing.rrf_score += contribution;
            if !existing.engines.contains(&item.engine) {
                existing.engines.push(item.engine);
            }
            if item.snippet.len() > existing.snippet.len() {
                existing.snippet = item.snippet;
            }
            if existing.published.is_none() {
                existing.published = item.published;
            }
            if existing.thumbnail.is_none() {
                existing.thumbnail = item.thumbnail;
            }
            for (k, v) in item.metadata {
                existing.metadata.entry(k).or_insert(v);
            }
        } else {
            groups.insert(
                canon,
                Group {
                    title: item.title,
                    url: item.url,
                    snippet: item.snippet,
                    engines: vec![item.engine],
                    category: item.category,
                    published: item.published,
                    thumbnail: item.thumbnail,
                    metadata: item.metadata,
                    rrf_score: contribution,
                },
            );
        }
    }

    let mut scored: Vec<SearchResult> = groups
        .into_values()
        .map(|g| {
            let consensus_bonus = 0.015 * (g.engines.len().saturating_sub(1) as f64);
            let title_lower = g.title.to_ascii_lowercase();
            let snippet_lower = g.snippet.to_ascii_lowercase();

            let mut term_bonus = 0.0;
            if !query_tokens.is_empty() {
                let title_hits = query_tokens
                    .iter()
                    .filter(|t| title_lower.contains(t.as_str()))
                    .count();
                let snippet_hits = query_tokens
                    .iter()
                    .filter(|t| snippet_lower.contains(t.as_str()))
                    .count();
                let denom = query_tokens.len() as f64;
                term_bonus = 0.012 * (title_hits as f64 / denom)
                    + 0.005 * (snippet_hits as f64 / denom);
            }

            let raw_score = g.rrf_score + consensus_bonus + term_bonus;
            let rounded_score = (raw_score * 10_000.0).round() / 10_000.0;

            SearchResult {
                title: g.title,
                url: g.url,
                snippet: g.snippet,
                engines: g.engines,
                score: rounded_score,
                category: g.category,
                published: g.published,
                thumbnail: g.thumbnail,
                metadata: g.metadata,
            }
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.url.cmp(&b.url))
    });
    scored.truncate(limit);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_item(title: &str, url: &str, engine: &str, rank: usize) -> RawResult {
        RawResult {
            title: title.to_string(),
            url: url.to_string(),
            snippet: "Details about rust programming".to_string(),
            engine: engine.to_string(),
            rank,
            category: Category::General,
            published: None,
            thumbnail: None,
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn dedupes_urls_and_boosts_consensus() {
        let mut weights = HashMap::new();
        weights.insert("duckduckgo".into(), 1.3);
        weights.insert("wikipedia".into(), 1.15);

        let raw = vec![
            raw_item("Only One", "https://example.org/only", "duckduckgo", 1),
            raw_item(
                "Rust Language",
                "https://www.rust-lang.org/?utm_source=ddg",
                "duckduckgo",
                3,
            ),
            raw_item(
                "Rust Language",
                "https://rust-lang.org/",
                "wikipedia",
                2,
            ),
        ];

        let ranked = rank_and_dedupe("rust language", raw, &weights, 10);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].title, "Rust Language");
        assert_eq!(
            ranked[0].engines,
            vec!["duckduckgo".to_string(), "wikipedia".to_string()]
        );
    }
}
