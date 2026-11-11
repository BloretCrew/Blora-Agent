// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Web search over the public HTTP APIs of SearXNG, Brave, Tavily, and Exa.

use std::time::Duration;

use blora_types::{BloraError, Result};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchBackend {
    Searxng,
    Brave,
    Tavily,
    Exa,
}

impl SearchBackend {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Searxng => "searxng",
            Self::Brave => "brave",
            Self::Tavily => "tavily",
            Self::Exa => "exa",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "searxng" => Some(Self::Searxng),
            "brave" => Some(Self::Brave),
            "tavily" => Some(Self::Tavily),
            "exa" => Some(Self::Exa),
            _ => None,
        }
    }
}

/// Backend plus credentials, normally read from the environment.
#[derive(Clone, Debug)]
pub struct SearchConfig {
    pub backend: SearchBackend,
    /// API key, or the SearXNG base URL for [`SearchBackend::Searxng`].
    pub credential: String,
    /// Override of the vendor endpoint (tests, self-hosted proxies).
    pub endpoint: Option<String>,
}

const SETUP_HINT: &str = "web_search is not configured. Set one of: SEARXNG_URL (self-hosted SearXNG with JSON output enabled), BRAVE_SEARCH_API_KEY, TAVILY_API_KEY, or EXA_API_KEY. BLORA_SEARCH_BACKEND picks one when several are set. web_fetch still works for known URLs.";

impl SearchConfig {
    /// Pick a backend from the environment. `BLORA_SEARCH_BACKEND` forces one;
    /// otherwise the first configured backend wins in the order SearXNG, Brave,
    /// Tavily, Exa.
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|name| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        })
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let credential = |backend: SearchBackend| -> Option<String> {
            get(match backend {
                SearchBackend::Searxng => "SEARXNG_URL",
                SearchBackend::Brave => "BRAVE_SEARCH_API_KEY",
                SearchBackend::Tavily => "TAVILY_API_KEY",
                SearchBackend::Exa => "EXA_API_KEY",
            })
        };
        let endpoint = get("BLORA_SEARCH_ENDPOINT");
        if let Some(forced) = get("BLORA_SEARCH_BACKEND") {
            let backend = SearchBackend::parse(&forced).ok_or_else(|| {
                BloraError::Other(format!(
                    "unknown BLORA_SEARCH_BACKEND {forced}; use searxng, brave, tavily, or exa"
                ))
            })?;
            let credential = credential(backend).ok_or_else(|| {
                BloraError::Other(format!(
                    "BLORA_SEARCH_BACKEND={} but its credential is missing. {SETUP_HINT}",
                    backend.as_str()
                ))
            })?;
            return Ok(Self {
                backend,
                credential,
                endpoint,
            });
        }
        for backend in [
            SearchBackend::Searxng,
            SearchBackend::Brave,
            SearchBackend::Tavily,
            SearchBackend::Exa,
        ] {
            if let Some(credential) = credential(backend) {
                return Ok(Self {
                    backend,
                    credential,
                    endpoint,
                });
            }
        }
        Err(BloraError::Other(SETUP_HINT.to_owned()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Run one query and return up to `limit` hits.
pub fn search(config: &SearchConfig, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
    let query = query.trim();
    if query.is_empty() {
        return Err(BloraError::Other("web_search needs a query".to_owned()));
    }
    let limit = limit.clamp(1, 20);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("BloraAgent/", env!("CARGO_PKG_VERSION")))
        .build();
    let endpoint = |default: &str| {
        config
            .endpoint
            .clone()
            .unwrap_or_else(|| default.to_owned())
    };
    let limit_text = limit.to_string();
    let result = match config.backend {
        SearchBackend::Searxng => {
            let base = config
                .endpoint
                .clone()
                .unwrap_or_else(|| config.credential.clone());
            agent
                .get(&format!("{}/search", base.trim_end_matches('/')))
                .query("q", query)
                .query("format", "json")
                .set("Accept", "application/json")
                .call()
        }
        SearchBackend::Brave => agent
            .get(&endpoint("https://api.search.brave.com/res/v1/web/search"))
            .query("q", query)
            .query("count", &limit_text)
            .set("Accept", "application/json")
            .set("X-Subscription-Token", &config.credential)
            .call(),
        SearchBackend::Tavily => agent
            .post(&endpoint("https://api.tavily.com/search"))
            .set("Authorization", &format!("Bearer {}", config.credential))
            .send_json(json!({"query": query, "max_results": limit})),
        SearchBackend::Exa => agent
            .post(&endpoint("https://api.exa.ai/search"))
            .set("x-api-key", &config.credential)
            .send_json(json!({
                "query": query,
                "numResults": limit,
                "contents": {"text": {"maxCharacters": 400}}
            })),
    };
    let response = result.map_err(|err| match err {
        ureq::Error::Status(code, response) => {
            let body: String = response
                .into_string()
                .unwrap_or_default()
                .chars()
                .take(300)
                .collect();
            BloraError::Other(format!(
                "{} search failed with HTTP {code}: {}",
                config.backend.as_str(),
                body.trim()
            ))
        }
        ureq::Error::Transport(transport) => BloraError::Other(format!(
            "{} search failed: {transport}",
            config.backend.as_str()
        )),
    })?;
    let body: Value = response
        .into_json()
        .map_err(|err| BloraError::Other(format!("search response was not JSON: {err}")))?;
    let mut hits = parse_hits(config.backend, &body);
    hits.truncate(limit);
    Ok(hits)
}

fn parse_hits(backend: SearchBackend, body: &Value) -> Vec<SearchHit> {
    let (list, snippet_keys): (Option<&Vec<Value>>, &[&str]) = match backend {
        SearchBackend::Searxng => (body["results"].as_array(), &["content"]),
        SearchBackend::Brave => (body["web"]["results"].as_array(), &["description"]),
        SearchBackend::Tavily => (body["results"].as_array(), &["content"]),
        SearchBackend::Exa => (body["results"].as_array(), &["summary", "text"]),
    };
    list.into_iter()
        .flatten()
        .filter_map(|item| {
            let url = item["url"].as_str()?.trim().to_owned();
            if url.is_empty() {
                return None;
            }
            let title = item["title"].as_str().unwrap_or("").trim().to_owned();
            let snippet = snippet_keys
                .iter()
                .find_map(|key| item[*key].as_str())
                .unwrap_or("");
            let snippet = crate::html::decode_entities(&strip_tags(snippet));
            let snippet: String = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
            let snippet: String = snippet.chars().take(400).collect();
            Some(SearchHit {
                title: if title.is_empty() { url.clone() } else { title },
                url,
                snippet,
            })
        })
        .collect()
}

fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Render hits for the model: numbered, with URL and snippet.
#[must_use]
pub fn format_hits(backend: SearchBackend, query: &str, hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return format!("No results for \"{query}\" ({}).", backend.as_str());
    }
    let mut out = format!(
        "Results for \"{query}\" via {} (untrusted web content; verify before relying on it):\n",
        backend.as_str()
    );
    for (index, hit) in hits.iter().enumerate() {
        out.push_str(&format!("\n{}. {}\n   {}\n", index + 1, hit.title, hit.url));
        if !hit.snippet.is_empty() {
            out.push_str(&format!("   {}\n", hit.snippet));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::tests::{http, serve};

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn selects_backend_from_environment() {
        assert!(
            SearchConfig::from_lookup(lookup(&[]))
                .unwrap_err()
                .to_string()
                .contains("SEARXNG_URL")
        );
        let config =
            SearchConfig::from_lookup(lookup(&[("TAVILY_API_KEY", "t"), ("EXA_API_KEY", "e")]))
                .unwrap();
        assert_eq!(config.backend, SearchBackend::Tavily);
        let forced = SearchConfig::from_lookup(lookup(&[
            ("TAVILY_API_KEY", "t"),
            ("EXA_API_KEY", "e"),
            ("BLORA_SEARCH_BACKEND", "exa"),
        ]))
        .unwrap();
        assert_eq!(forced.backend, SearchBackend::Exa);
        assert!(SearchConfig::from_lookup(lookup(&[("BLORA_SEARCH_BACKEND", "brave")])).is_err());
    }

    #[test]
    fn parses_each_vendor_shape() {
        let brave = json!({"web": {"results": [{"title": "B", "url": "https://b", "description": "<strong>hi</strong> &amp; bye"}]}});
        assert_eq!(
            parse_hits(SearchBackend::Brave, &brave)[0].snippet,
            "hi & bye"
        );
        let tavily = json!({"results": [{"title": "T", "url": "https://t", "content": "c"}, {"title": "no url"}]});
        assert_eq!(parse_hits(SearchBackend::Tavily, &tavily).len(), 1);
        let exa = json!({"results": [{"title": "", "url": "https://e", "text": "body"}]});
        let hit = &parse_hits(SearchBackend::Exa, &exa)[0];
        assert_eq!(hit.title, "https://e");
        assert_eq!(hit.snippet, "body");
    }

    #[test]
    fn queries_searxng_over_http() {
        let base = serve(1, |request| {
            assert!(
                request.starts_with("GET /search?q=rust+agents&format=json"),
                "{request}"
            );
            http(
                "200 OK",
                "application/json",
                r#"{"results":[{"title":"Rust","url":"https://rust-lang.org","content":"A language"}]}"#,
            )
        });
        let config = SearchConfig {
            backend: SearchBackend::Searxng,
            credential: base,
            endpoint: None,
        };
        let hits = search(&config, "rust agents", 5).unwrap();
        assert_eq!(hits[0].url, "https://rust-lang.org");
        let text = format_hits(config.backend, "rust agents", &hits);
        assert!(text.contains("1. Rust\n   https://rust-lang.org\n   A language"));
    }

    #[test]
    fn reports_vendor_errors() {
        let base = serve(1, |_| {
            http(
                "401 Unauthorized",
                "application/json",
                "{\"error\":\"bad key\"}",
            )
        });
        let config = SearchConfig {
            backend: SearchBackend::Tavily,
            credential: "x".to_owned(),
            endpoint: Some(base),
        };
        let err = search(&config, "q", 3).unwrap_err().to_string();
        assert!(err.contains("HTTP 401") && err.contains("bad key"), "{err}");
    }
}
