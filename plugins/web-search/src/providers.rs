//! Search provider chain (`searxng` -> `tavily` -> `brave` -> `duckduckgo` HTML lite)
//! for `plugin-web-search`.
//!
//! API keys (`TAVILY_API_KEY`, `BRAVE_API_KEY`) are read strictly from [`EnvSource`] and are
//! never logged or persisted to disk.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::config::{
    ENV_BRAVE_API_KEY, ENV_SEARXNG_URL, ENV_TAVILY_API_KEY, EnvSource, MAX_SEARCH_RESULTS,
    WebSearchConfig,
};
use crate::html::{collapse_whitespace, decode_html_entities};
use crate::transport::{HttpRequest, WebTransport};

/// One normalized search hit returned by `web.search`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResultItem {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Structured output returned in `Receipt.detail` by `web.search`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchOutput {
    pub provider: String,
    pub query: String,
    pub results: Vec<SearchResultItem>,
}

/// Diagnostic record for one skipped or failed provider in the chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAttempt {
    pub provider: String,
    pub reason: String,
}

/// Structured error returned when no provider in the chain succeeded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchChainError {
    pub error: String,
    pub query: String,
    pub tried: Vec<ProviderAttempt>,
}

enum ProviderStepError {
    Skipped(String),
    Failed(String),
}

/// Executes `web.search` across the configured provider chain (`searxng` -> `tavily` -> `brave` -> `duckduckgo`).
pub fn search_web(
    query: &str,
    max_results_override: Option<usize>,
    config: &WebSearchConfig,
    env: &dyn EnvSource,
    transport: &dyn WebTransport,
) -> Result<SearchOutput, SearchChainError> {
    let clean_query = query.trim();
    let limit = max_results_override
        .unwrap_or(config.default_max_results)
        .clamp(1, MAX_SEARCH_RESULTS);

    let mut tried = Vec::new();
    for provider in &config.providers {
        let name = provider.trim().to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        match call_provider(&name, clean_query, limit, config, env, transport) {
            Ok(mut items) => {
                items.truncate(limit);
                if !items.is_empty() {
                    return Ok(SearchOutput {
                        provider: name,
                        query: clean_query.to_owned(),
                        results: items,
                    });
                }
                tried.push(ProviderAttempt {
                    provider: name,
                    reason: "empty".to_owned(),
                });
            }
            Err(ProviderStepError::Skipped(reason) | ProviderStepError::Failed(reason)) => {
                tried.push(ProviderAttempt {
                    provider: name,
                    reason,
                });
            }
        }
    }

    Err(SearchChainError {
        error: "no provider answered".to_owned(),
        query: clean_query.to_owned(),
        tried,
    })
}

fn call_provider(
    name: &str,
    query: &str,
    limit: usize,
    config: &WebSearchConfig,
    env: &dyn EnvSource,
    transport: &dyn WebTransport,
) -> Result<Vec<SearchResultItem>, ProviderStepError> {
    match name {
        "searxng" => search_searxng(query, limit, config, env, transport),
        "tavily" => search_tavily(query, limit, env, transport),
        "brave" => search_brave(query, limit, env, transport),
        "duckduckgo" | "ddgs" => search_duckduckgo(query, limit, transport),
        _ => Err(ProviderStepError::Skipped("unknown_provider".to_owned())),
    }
}

fn search_searxng(
    query: &str,
    limit: usize,
    config: &WebSearchConfig,
    env: &dyn EnvSource,
    transport: &dyn WebTransport,
) -> Result<Vec<SearchResultItem>, ProviderStepError> {
    let base = if !config.searxng_url.trim().is_empty() {
        config.searxng_url.trim().to_owned()
    } else if let Some(from_env) = env.get_nonempty(ENV_SEARXNG_URL) {
        from_env
    } else {
        return Err(ProviderStepError::Skipped("missing_url".to_owned()));
    };

    let mut url = Url::parse(&format!("{}/search", base.trim_end_matches('/')))
        .map_err(|_| ProviderStepError::Failed("invalid_searxng_url".to_owned()))?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("format", "json");

    let resp = send_checked(HttpRequest::get(url.as_str()), transport)?;
    let val: Value = serde_json::from_slice(&resp.body)
        .map_err(|_| ProviderStepError::Failed("invalid_json".to_owned()))?;

    let mut out = Vec::new();
    if let Some(arr) = val.get("results").and_then(Value::as_array) {
        for item in arr.iter().take(limit) {
            let title = item.get("title").and_then(Value::as_str).unwrap_or("");
            let link = item.get("url").and_then(Value::as_str).unwrap_or("");
            let snippet = item.get("content").and_then(Value::as_str).unwrap_or("");
            push_if_valid(&mut out, title, link, snippet);
        }
    }
    Ok(out)
}

fn search_tavily(
    query: &str,
    limit: usize,
    env: &dyn EnvSource,
    transport: &dyn WebTransport,
) -> Result<Vec<SearchResultItem>, ProviderStepError> {
    let Some(key) = env.get_nonempty(ENV_TAVILY_API_KEY) else {
        return Err(ProviderStepError::Skipped("missing_key".to_owned()));
    };

    let mut req = HttpRequest::post_json(
        "https://api.tavily.com/search",
        json!({
            "query": query,
            "max_results": limit,
        }),
    );
    req.headers = BTreeMap::from([("Authorization".to_owned(), format!("Bearer {key}"))]);

    let resp = send_checked(req, transport)?;
    let val: Value = serde_json::from_slice(&resp.body)
        .map_err(|_| ProviderStepError::Failed("invalid_json".to_owned()))?;

    let mut out = Vec::new();
    if let Some(arr) = val.get("results").and_then(Value::as_array) {
        for item in arr.iter().take(limit) {
            let title = item.get("title").and_then(Value::as_str).unwrap_or("");
            let link = item.get("url").and_then(Value::as_str).unwrap_or("");
            let snippet = item.get("content").and_then(Value::as_str).unwrap_or("");
            push_if_valid(&mut out, title, link, snippet);
        }
    }
    Ok(out)
}

fn search_brave(
    query: &str,
    limit: usize,
    env: &dyn EnvSource,
    transport: &dyn WebTransport,
) -> Result<Vec<SearchResultItem>, ProviderStepError> {
    let Some(key) = env.get_nonempty(ENV_BRAVE_API_KEY) else {
        return Err(ProviderStepError::Skipped("missing_key".to_owned()));
    };

    let mut url = Url::parse("https://api.search.brave.com/res/v1/web/search")
        .map_err(|_| ProviderStepError::Failed("invalid_url".to_owned()))?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("count", &limit.to_string());

    let mut req = HttpRequest::get(url.as_str());
    req.headers = BTreeMap::from([
        ("X-Subscription-Token".to_owned(), key),
        ("Accept".to_owned(), "application/json".to_owned()),
    ]);

    let resp = send_checked(req, transport)?;
    let val: Value = serde_json::from_slice(&resp.body)
        .map_err(|_| ProviderStepError::Failed("invalid_json".to_owned()))?;

    let mut out = Vec::new();
    if let Some(arr) = val
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(Value::as_array)
    {
        for item in arr.iter().take(limit) {
            let title = item.get("title").and_then(Value::as_str).unwrap_or("");
            let link = item.get("url").and_then(Value::as_str).unwrap_or("");
            let snippet = item
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            push_if_valid(&mut out, title, link, snippet);
        }
    }
    Ok(out)
}

fn search_duckduckgo(
    query: &str,
    limit: usize,
    transport: &dyn WebTransport,
) -> Result<Vec<SearchResultItem>, ProviderStepError> {
    let mut url = Url::parse("https://html.duckduckgo.com/html/")
        .map_err(|_| ProviderStepError::Failed("invalid_url".to_owned()))?;
    url.query_pairs_mut().append_pair("q", query);

    let mut req = HttpRequest::get(url.as_str());
    req.headers = BTreeMap::from([(
        "Accept".to_owned(),
        "text/html,application/xhtml+xml".to_owned(),
    )]);

    let resp = send_checked(req, transport)?;
    let html = String::from_utf8_lossy(&resp.body);
    Ok(parse_duckduckgo_html(&html, limit))
}

fn send_checked(
    req: HttpRequest,
    transport: &dyn WebTransport,
) -> Result<crate::transport::HttpResponse, ProviderStepError> {
    let resp = transport
        .send_request(req)
        .map_err(|_| ProviderStepError::Failed("transport_error".to_owned()))?;
    if resp.status >= 400 {
        return Err(ProviderStepError::Failed(format!("http_{}", resp.status)));
    }
    Ok(resp)
}

fn push_if_valid(out: &mut Vec<SearchResultItem>, title: &str, url: &str, snippet: &str) {
    let clean_url = url.trim();
    if clean_url.is_empty() {
        return;
    }
    out.push(SearchResultItem {
        title: collapse_whitespace(&decode_html_entities(title)),
        url: clean_url.to_owned(),
        snippet: collapse_whitespace(&decode_html_entities(snippet)),
    });
}

/// Parses DuckDuckGo HTML (`/html/` `result__a` + `result__snippet`, or `/lite/` `result-link` +
/// `result-snippet`) search results cleanly without external HTML parser crates.
#[must_use]
pub fn parse_duckduckgo_html(html: &str, limit: usize) -> Vec<SearchResultItem> {
    let mut results = Vec::new();
    let mut pos = 0usize;

    while results.len() < limit {
        let next_a = find_anchor_with_class(&html[pos..], "result__a")
            .or_else(|| find_anchor_with_class(&html[pos..], "result-link"));
        let Some((rel_start, rel_end, href, title)) = next_a else {
            break;
        };

        let after_anchor = pos + rel_end;
        let next_anchor_boundary = find_anchor_with_class(&html[after_anchor..], "result__a")
            .or_else(|| find_anchor_with_class(&html[after_anchor..], "result-link"))
            .map_or(html.len(), |(s, _, _, _)| after_anchor + s);

        let window = &html[after_anchor..next_anchor_boundary];
        let snippet = extract_class_element_text(window, "result__snippet")
            .or_else(|| extract_class_element_text(window, "result-snippet"))
            .unwrap_or_default();

        let unwrapped_url = unwrap_ddg_redirect(&decode_html_entities(&href));
        push_if_valid(&mut results, &title, &unwrapped_url, &snippet);

        pos += rel_start + rel_end.saturating_sub(rel_start).max(1);
    }

    results
}

fn find_anchor_with_class(slice: &str, class_needle: &str) -> Option<(usize, usize, String, String)> {
    let mut scan = 0usize;
    while let Some(rel_lt) = slice[scan..].find("<a") {
        let start = scan + rel_lt;
        let after_a = start + 2;
        if let Some(next_ch) = slice[after_a..].chars().next()
            && !next_ch.is_ascii_whitespace()
        {
            scan = after_a;
            continue;
        }
        let Some(tag_close_rel) = slice[start..].find('>') else {
            break;
        };
        let attrs = &slice[start + 2..start + tag_close_rel];
        let body_start = start + tag_close_rel + 1;

        if has_html_class(attrs, class_needle) {
            let href = extract_attr(attrs, "href").unwrap_or_default();
            if let Some(close_rel) = slice[body_start..].find("</a>") {
                let raw_inner = &slice[body_start..body_start + close_rel];
                let clean_title = strip_inline_tags(raw_inner);
                return Some((start, body_start + close_rel + 4, href, clean_title));
            }
        }
        scan = body_start;
    }
    None
}

fn extract_class_element_text(slice: &str, class_needle: &str) -> Option<String> {
    let mut scan = 0usize;
    while let Some(rel_lt) = slice[scan..].find('<') {
        let start = scan + rel_lt;
        if slice[start..].starts_with("</") {
            scan = start + 2;
            continue;
        }
        let Some(tag_close_rel) = slice[start..].find('>') else {
            break;
        };
        let tag_header = &slice[start + 1..start + tag_close_rel];
        let body_start = start + tag_close_rel + 1;
        let tag_name = tag_header
            .split(|c: char| c.is_ascii_whitespace())
            .next()
            .unwrap_or("");
        if !tag_name.is_empty() && has_html_class(tag_header, class_needle) {
            let end_marker = format!("</{tag_name}>");
            if let Some(close_rel) = slice[body_start..].find(&end_marker) {
                let raw = &slice[body_start..body_start + close_rel];
                return Some(strip_inline_tags(raw));
            }
        }
        scan = body_start;
    }
    None
}

fn has_html_class(attrs: &str, target: &str) -> bool {
    extract_attr(attrs, "class").is_some_and(|classes| {
        classes
            .split_ascii_whitespace()
            .any(|cls| cls == target)
    })
}

fn extract_attr(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let needle = format!("{name}=");
    let mut idx = 0usize;
    while let Some(found) = lower[idx..].find(&needle) {
        let pos = idx + found;
        let boundary_ok = pos == 0
            || attrs.as_bytes()[pos - 1].is_ascii_whitespace();
        if !boundary_ok {
            idx = pos + needle.len();
            continue;
        }
        let val_start = pos + needle.len();
        let rest = attrs[val_start..].trim_start();
        if let Some(stripped) = rest.strip_prefix('"') {
            if let Some(end_q) = stripped.find('"') {
                return Some(stripped[..end_q].to_owned());
            }
        } else if let Some(stripped) = rest.strip_prefix('\'') {
            if let Some(end_q) = stripped.find('\'') {
                return Some(stripped[..end_q].to_owned());
            }
        } else {
            let end = rest
                .find(|c: char| c.is_ascii_whitespace() || c == '>')
                .unwrap_or(rest.len());
            return Some(rest[..end].to_owned());
        }
        break;
    }
    None
}

fn strip_inline_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    collapse_whitespace(&decode_html_entities(&out))
}

fn unwrap_ddg_redirect(href: &str) -> String {
    let normalized = if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.to_owned()
    };
    if let Ok(parsed) = Url::parse(&normalized)
        && parsed
            .host_str()
            .is_some_and(|h| h.ends_with("duckduckgo.com"))
        && let Some((_, target)) = parsed.query_pairs().find(|(k, _)| k == "uddg")
    {
        return target.into_owned();
    }
    href.to_owned()
}
