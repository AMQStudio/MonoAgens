//! SSRF-checked page fetching (`web.fetch`) with manual bounded redirect inspection.

use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::{MIN_MAX_CHARS, WebSearchConfig};
use crate::html::{extract_html_text, normalize_plain_text};
use crate::ssrf::{DnsResolver, SsrfError, validate_and_resolve};
use crate::transport::{HttpRequest, WebTransport};

const TEXT_CONTENT_PREFIXES: &[&str] = &[
    "text/",
    "application/xhtml+xml",
    "application/json",
    "application/xml",
];

/// Successful output returned in `Receipt.detail` by `web.fetch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchOutput {
    pub url: String,
    pub status: u16,
    pub title: String,
    pub text: String,
    pub truncated: bool,
}

/// Classification of `web.fetch` errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchErrorKind {
    /// Refused before issuing an outbound request (or on a redirect hop) due to SSRF/policy.
    Blocked,
    /// Upstream HTTP/network/content-type/redirect-limit failure.
    Upstream,
}

/// Typed failure returned by [`fetch_page`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchError {
    pub kind: FetchErrorKind,
    pub reason_code: String,
    pub message: String,
}

impl FetchError {
    fn blocked(err: SsrfError) -> Self {
        Self {
            kind: FetchErrorKind::Blocked,
            reason_code: "ERR_SSRF_BLOCKED".to_owned(),
            message: err.to_string(),
        }
    }

    fn upstream(reason_code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: FetchErrorKind::Upstream,
            reason_code: reason_code.into(),
            message: message.into(),
        }
    }
}

/// Fetches `raw_url`, validating every hop (initial URL + up to `config.max_redirects`
/// `Location` redirects) through [`validate_and_resolve`] before issuing any request.
pub fn fetch_page(
    raw_url: &str,
    max_chars_override: Option<usize>,
    config: &WebSearchConfig,
    transport: &dyn WebTransport,
    resolver: &dyn DnsResolver,
) -> Result<FetchOutput, FetchError> {
    let max_chars = max_chars_override
        .unwrap_or(config.default_max_chars)
        .max(MIN_MAX_CHARS);

    let mut current_url = raw_url.trim().to_owned();

    for hop in 0..=config.max_redirects {
        let validated = validate_and_resolve(&current_url, &config.allow_ports, resolver)
            .map_err(FetchError::blocked)?;

        let resp = transport
            .send_request(HttpRequest::get(validated.url.as_str()))
            .map_err(|e| FetchError::upstream("web_fetch_transport_error", e))?;

        if is_redirect(resp.status) {
            let Some(location) = resp.header("location").filter(|s| !s.trim().is_empty()) else {
                return Err(FetchError::upstream(
                    "web_fetch_redirect_missing_location",
                    format!("server answered {} without Location header", resp.status),
                ));
            };
            if hop == config.max_redirects {
                return Err(FetchError::upstream(
                    "web_fetch_too_many_redirects",
                    format!("too many redirects (max {})", config.max_redirects),
                ));
            }
            let next_url = validated
                .url
                .join(location.trim())
                .map_err(|e| FetchError::blocked(SsrfError::InvalidUrl(e.to_string())))?;
            current_url = next_url.to_string();
            continue;
        }

        if resp.status >= 400 {
            return Err(FetchError::upstream(
                format!("http_{}", resp.status),
                format!("server returned HTTP {}", resp.status),
            ));
        }

        let content_type = resp
            .header("content-type")
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();

        if !content_type.is_empty()
            && !TEXT_CONTENT_PREFIXES
                .iter()
                .any(|prefix| content_type.starts_with(prefix))
        {
            return Err(FetchError::upstream(
                "web_fetch_unsupported_content_type",
                format!("not a text page: {content_type}"),
            ));
        }

        let body_text = String::from_utf8_lossy(&resp.body);
        let is_html = content_type.is_empty()
            || content_type.starts_with("text/html")
            || content_type.starts_with("application/xhtml+xml");

        let (title, text, truncated) = if is_html {
            let extracted = extract_html_text(&body_text, max_chars);
            (extracted.title, extracted.text, extracted.truncated)
        } else {
            let (norm, trunc) = normalize_plain_text(&body_text, max_chars);
            (String::new(), norm, trunc)
        };

        return Ok(FetchOutput {
            url: canonical_url_string(&validated.url),
            status: resp.status,
            title,
            text,
            truncated,
        });
    }

    Err(FetchError::upstream(
        "web_fetch_too_many_redirects",
        format!("too many redirects (max {})", config.max_redirects),
    ))
}

const fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn canonical_url_string(url: &Url) -> String {
    url.as_str().to_owned()
}
