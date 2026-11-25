use url::Url;

/// Strips `<...>` HTML/XML tags from a string while preserving spacing.
pub fn strip_html_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => {
                in_tag = true;
                if !out.ends_with(' ') && !out.is_empty() {
                    out.push(' ');
                }
            }
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Decodes common named, decimal, and hexadecimal HTML entities.
pub fn decode_html_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp_idx) = rest.find('&') {
        out.push_str(&rest[..amp_idx]);
        let after = &rest[amp_idx + 1..];
        if let Some(semi_idx) = after.find(';') {
            if semi_idx > 0 && semi_idx <= 10 {
                let entity = &after[..semi_idx];
                if let Some(decoded) = decode_single_entity(entity) {
                    out.push_str(&decoded);
                    rest = &after[semi_idx + 1..];
                    continue;
                }
            }
        }
        out.push('&');
        rest = after;
    }
    out.push_str(rest);
    out
}

fn decode_single_entity(entity: &str) -> Option<String> {
    match entity {
        "amp" => Some("&".to_string()),
        "lt" => Some("<".to_string()),
        "gt" => Some(">".to_string()),
        "quot" => Some("\"".to_string()),
        "apos" | "#39" | "#x27" | "#X27" => Some("'".to_string()),
        "nbsp" => Some(" ".to_string()),
        "ndash" => Some("-".to_string()),
        "mdash" => Some("--".to_string()),
        "hellip" => Some("...".to_string()),
        _ if entity.starts_with("#x") || entity.starts_with("#X") => {
            u32::from_str_radix(&entity[2..], 16)
                .ok()
                .and_then(char::from_u32)
                .map(|c| c.to_string())
        }
        _ if entity.starts_with('#') => entity[1..]
            .parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .map(|c| c.to_string()),
        _ => None,
    }
}

/// Strips HTML tags, decodes HTML entities, and collapses consecutive whitespace.
pub fn clean_text(input: &str) -> String {
    let stripped = strip_html_tags(input);
    let decoded = decode_html_entities(&stripped);
    let mut out = String::with_capacity(decoded.len());
    for word in decoded.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Truncates a string to at most `max_chars` Unicode scalar values, appending `...` when cut.
pub fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    if max_chars <= 3 {
        return input.chars().take(max_chars).collect();
    }
    let mut s: String = input.chars().take(max_chars - 3).collect();
    s.push_str("...");
    s
}

/// Normalizes URLs for cross-engine deduplication (lowercases host, strips `www.`,
/// removes fragments and tracking parameters, normalizes trailing slashes).
pub fn canonicalize_url(raw: &str) -> String {
    let trimmed = raw.trim();
    let Ok(mut url) = Url::parse(trimmed) else {
        return trimmed.to_string();
    };

    let _ = url.set_fragment(None);

    if let Some(host) = url.host_str().map(str::to_ascii_lowercase) {
        let stripped = host.strip_prefix("www.").unwrap_or(&host).to_string();
        let _ = url.set_host(Some(&stripped));
    }

    let kept_pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| !is_tracking_param(k))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    if kept_pairs.is_empty() {
        url.set_query(None);
    } else {
        let mut ser = url.query_pairs_mut();
        ser.clear();
        for (k, v) in kept_pairs {
            ser.append_pair(&k, &v);
        }
    }

    let path = url.path().to_string();
    if path.len() > 1 && path.ends_with('/') {
        let trimmed_path = path.trim_end_matches('/');
        url.set_path(trimmed_path);
    }

    url.to_string()
}

fn is_tracking_param(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.starts_with("utm_")
        || matches!(
            k.as_str(),
            "fbclid" | "gclid" | "dclid" | "msclkid" | "mc_cid" | "mc_eid" | "igshid" | "ref"
        )
}

/// Extracts the destination URL from DuckDuckGo HTML redirect links (`//duckduckgo.com/l/?uddg=...`)
/// while filtering out DuckDuckGo ad redirects (`y.js`).
pub fn unwrap_ddg_url(href: &str) -> Option<String> {
    let trimmed = href.trim();
    if trimmed.is_empty() || trimmed.contains("/y.js") {
        return None;
    }

    let full = if trimmed.starts_with("//") {
        format!("https:{trimmed}")
    } else if trimmed.starts_with('/') {
        format!("https://duckduckgo.com{trimmed}")
    } else {
        trimmed.to_string()
    };

    let parsed = Url::parse(&full).ok()?;
    if parsed
        .host_str()
        .is_some_and(|h| h.ends_with("duckduckgo.com"))
        && parsed.path().starts_with("/l/")
    {
        for (k, v) in parsed.query_pairs() {
            if k == "uddg" && (v.starts_with("http://") || v.starts_with("https://")) {
                return Some(v.into_owned());
            }
        }
        return None;
    }

    if matches!(parsed.scheme(), "http" | "https") {
        Some(parsed.to_string())
    } else {
        None
    }
}

/// Validates and normalizes an ISO language code for safe insertion into search URLs.
pub fn sanitize_lang(lang: &str) -> String {
    let clean: String = lang
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphabetic() || *c == '-')
        .take(8)
        .collect();
    if clean.is_empty() {
        "en".to_string()
    } else {
        clean.to_ascii_lowercase()
    }
}

/// Extracts all `<tag>...</tag>` inner blocks from an XML payload.
pub fn extract_xml_blocks<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut blocks = Vec::new();
    let mut pos = 0;

    while let Some(rel_start) = xml[pos..].find(&open) {
        let tag_start = pos + rel_start;
        let after_open = tag_start + open.len();
        let Some(gt_rel) = xml[after_open..].find('>') else {
            break;
        };
        let content_start = after_open + gt_rel + 1;
        let Some(close_rel) = xml[content_start..].find(&close) else {
            break;
        };
        let content_end = content_start + close_rel;
        blocks.push(&xml[content_start..content_end]);
        pos = content_end + close.len();
    }

    blocks
}

/// Extracts and cleans the first `<tag>...</tag>` text value inside an XML block.
pub fn extract_xml_tag(block: &str, tag: &str) -> Option<String> {
    extract_xml_blocks(block, tag)
        .into_iter()
        .next()
        .map(clean_text)
        .filter(|s| !s.is_empty())
}

/// Extracts and cleans all `<tag>...</tag>` text values inside an XML block.
pub fn extract_all_xml_tags(block: &str, tag: &str) -> Vec<String> {
    extract_xml_blocks(block, tag)
        .into_iter()
        .map(clean_text)
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_html_and_entities() {
        let raw = "<span class=\"searchmatch\">Rust</span> &amp; Cargo&#39;s &#x27;fast&#x27;   build";
        assert_eq!(clean_text(raw), "Rust & Cargo's 'fast' build");
    }

    #[test]
    fn canonicalizes_urls_for_dedupe() {
        assert_eq!(
            canonicalize_url("https://www.Example.COM/docs/?utm_source=x&q=rust#top"),
            "https://example.com/docs?q=rust"
        );
        assert_eq!(
            canonicalize_url("https://rust-lang.org/learn/"),
            "https://rust-lang.org/learn"
        );
    }

    #[test]
    fn unwraps_ddg_redirect_and_rejects_ads() {
        let href = "//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org%2F&rut=abc";
        assert_eq!(
            unwrap_ddg_url(href),
            Some("https://rust-lang.org/".to_string())
        );
        assert_eq!(unwrap_ddg_url("https://duckduckgo.com/y.js?ad=1"), None);
    }

    #[test]
    fn extracts_xml_blocks_and_tags() {
        let xml = "<feed><entry><title>Paper 1</title><author><name>Alice</name></author><author><name>Bob</name></author></entry></feed>";
        let entries = extract_xml_blocks(xml, "entry");
        assert_eq!(entries.len(), 1);
        assert_eq!(extract_xml_tag(entries[0], "title").as_deref(), Some("Paper 1"));
        assert_eq!(
            extract_all_xml_tags(entries[0], "name"),
            vec!["Alice".to_string(), "Bob".to_string()]
        );
    }
}
