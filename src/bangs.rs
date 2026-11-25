use crate::models::Category;

/// Parsed result of checking leading or trailing `!bang` shortcuts in a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBang {
    pub effective_query: String,
    pub category_override: Option<Category>,
    pub engines_override: Vec<String>,
}

struct BangSpec {
    trigger: &'static str,
    category: Option<Category>,
    engine: Option<&'static str>,
}

const BANGS: &[BangSpec] = &[
    BangSpec {
        trigger: "!g",
        category: None,
        engine: Some("duckduckgo"),
    },
    BangSpec {
        trigger: "!ddg",
        category: None,
        engine: Some("duckduckgo"),
    },
    BangSpec {
        trigger: "!web",
        category: Some(Category::General),
        engine: None,
    },
    BangSpec {
        trigger: "!w",
        category: Some(Category::Wiki),
        engine: Some("wikipedia"),
    },
    BangSpec {
        trigger: "!wiki",
        category: Some(Category::Wiki),
        engine: Some("wikipedia"),
    },
    BangSpec {
        trigger: "!wiby",
        category: Some(Category::General),
        engine: Some("wiby"),
    },
    BangSpec {
        trigger: "!hn",
        category: Some(Category::News),
        engine: Some("hackernews"),
    },
    BangSpec {
        trigger: "!gh",
        category: Some(Category::Code),
        engine: Some("github"),
    },
    BangSpec {
        trigger: "!github",
        category: Some(Category::Code),
        engine: Some("github"),
    },
    BangSpec {
        trigger: "!so",
        category: Some(Category::Code),
        engine: Some("stackexchange"),
    },
    BangSpec {
        trigger: "!arxiv",
        category: Some(Category::Science),
        engine: Some("arxiv"),
    },
    BangSpec {
        trigger: "!paper",
        category: Some(Category::Science),
        engine: Some("arxiv"),
    },
    BangSpec {
        trigger: "!crates",
        category: Some(Category::Packages),
        engine: Some("crates"),
    },
    BangSpec {
        trigger: "!crate",
        category: Some(Category::Packages),
        engine: Some("crates"),
    },
    BangSpec {
        trigger: "!rs",
        category: Some(Category::Packages),
        engine: Some("crates"),
    },
    BangSpec {
        trigger: "!npm",
        category: Some(Category::Packages),
        engine: Some("npm"),
    },
    BangSpec {
        trigger: "!js",
        category: Some(Category::Packages),
        engine: Some("npm"),
    },
    BangSpec {
        trigger: "!img",
        category: Some(Category::Images),
        engine: Some("wikimedia"),
    },
    BangSpec {
        trigger: "!images",
        category: Some(Category::Images),
        engine: Some("wikimedia"),
    },
    BangSpec {
        trigger: "!code",
        category: Some(Category::Code),
        engine: None,
    },
    BangSpec {
        trigger: "!news",
        category: Some(Category::News),
        engine: None,
    },
    BangSpec {
        trigger: "!sci",
        category: Some(Category::Science),
        engine: None,
    },
    BangSpec {
        trigger: "!science",
        category: Some(Category::Science),
        engine: None,
    },
    BangSpec {
        trigger: "!pkg",
        category: Some(Category::Packages),
        engine: None,
    },
    BangSpec {
        trigger: "!packages",
        category: Some(Category::Packages),
        engine: None,
    },
    BangSpec {
        trigger: "!all",
        category: Some(Category::All),
        engine: None,
    },
];

/// Parses a single leading or trailing known `!bang` token out of `raw_query`.
pub fn parse_bangs(raw_query: &str) -> ParsedBang {
    let tokens: Vec<&str> = raw_query.split_whitespace().collect();
    if tokens.is_empty() {
        return ParsedBang {
            effective_query: String::new(),
            category_override: None,
            engines_override: Vec::new(),
        };
    }

    if let Some(spec) = find_bang(tokens[0]) {
        return ParsedBang {
            effective_query: tokens[1..].join(" "),
            category_override: spec.category,
            engines_override: spec.engine.map(|e| vec![e.to_string()]).unwrap_or_default(),
        };
    }

    if tokens.len() > 1 {
        if let Some(spec) = find_bang(tokens[tokens.len() - 1]) {
            return ParsedBang {
                effective_query: tokens[..tokens.len() - 1].join(" "),
                category_override: spec.category,
                engines_override: spec.engine.map(|e| vec![e.to_string()]).unwrap_or_default(),
            };
        }
    }

    ParsedBang {
        effective_query: tokens.join(" "),
        category_override: None,
        engines_override: Vec::new(),
    }
}

fn find_bang(token: &str) -> Option<&'static BangSpec> {
    if !token.starts_with('!') {
        return None;
    }
    let lower = token.to_ascii_lowercase();
    BANGS.iter().find(|spec| spec.trigger == lower)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_leading_and_trailing_bangs() {
        let lead = parse_bangs("!w rust programming");
        assert_eq!(lead.effective_query, "rust programming");
        assert_eq!(lead.category_override, Some(Category::Wiki));
        assert_eq!(lead.engines_override, vec!["wikipedia".to_string()]);

        let trail = parse_bangs("attention is all you need !ARXIV");
        assert_eq!(trail.effective_query, "attention is all you need");
        assert_eq!(trail.category_override, Some(Category::Science));
        assert_eq!(trail.engines_override, vec!["arxiv".to_string()]);
    }

    #[test]
    fn preserves_unknown_bang_and_empty_query() {
        let unknown = parse_bangs("!unknown foo bar");
        assert_eq!(unknown.effective_query, "!unknown foo bar");
        assert_eq!(unknown.category_override, None);
        assert!(unknown.engines_override.is_empty());

        let empty = parse_bangs("   ");
        assert_eq!(empty.effective_query, "");
    }
}
