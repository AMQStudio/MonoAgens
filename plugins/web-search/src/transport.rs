//! Injectable HTTP transport abstraction for `plugin-web-search`.
//!
//! [`LiveWebTransport`] wraps `reqwest::blocking::Client` with `Policy::none()` so every
//! redirect hop is manually inspected by [`crate::ssrf`]. [`MockWebTransport`] supports
//! deterministic 100% offline unit and integration tests.

use std::collections::{BTreeMap, VecDeque};
use std::io::Read;
use std::sync::Mutex;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde_json::Value;

/// Maximum response body read per HTTP call (2 MiB).
pub const MAX_RESPONSE_BYTES: usize = 2_000_000;

/// HTTP method used by [`HttpRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

/// Outbound HTTP request dispatched through [`WebTransport`].
#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub json_body: Option<Value>,
}

impl HttpRequest {
    #[must_use]
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: HttpMethod::Get,
            url: url.into(),
            headers: BTreeMap::new(),
            json_body: None,
        }
    }

    #[must_use]
    pub fn post_json(url: impl Into<String>, body: Value) -> Self {
        Self {
            method: HttpMethod::Post,
            url: url.into(),
            headers: BTreeMap::new(),
            json_body: Some(body),
        }
    }
}

/// Inbound HTTP response returned by [`WebTransport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Looks up a response header case-insensitively.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        let needle = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_ascii_lowercase() == needle)
            .map(|(_, v)| v.as_str())
    }
}

/// Transport abstraction allowing search providers and `web.fetch` to run offline in tests.
pub trait WebTransport: Send + Sync {
    fn send_request(&self, req: HttpRequest) -> Result<HttpResponse, String>;
}

/// Live HTTP transport powered by `reqwest::blocking::Client` with automatic redirects disabled.
#[derive(Clone)]
pub struct LiveWebTransport {
    client: Client,
}

impl LiveWebTransport {
    /// Builds a [`LiveWebTransport`] with `Policy::none()` and the configured timeout.
    pub fn new(timeout_secs: u64) -> Result<Self, String> {
        let client = Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(timeout_secs.max(1)))
            .user_agent("monoagens-web-search/0.1")
            .build()
            .map_err(|e| format!("failed to build HTTP client: {e}"))?;
        Ok(Self { client })
    }
}

impl WebTransport for LiveWebTransport {
    fn send_request(&self, req: HttpRequest) -> Result<HttpResponse, String> {
        let mut builder = match req.method {
            HttpMethod::Get => self.client.get(&req.url),
            HttpMethod::Post => self.client.post(&req.url),
        };
        for (k, v) in &req.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        if let Some(ref json) = req.json_body {
            builder = builder.json(json);
        }

        let mut resp = builder
            .send()
            .map_err(|e| format!("http request failed: {}", redact_error(&e)))?;

        let status = resp.status().as_u16();
        let mut headers = BTreeMap::new();
        for (name, val) in resp.headers() {
            if let Ok(s) = val.to_str() {
                headers.insert(name.as_str().to_ascii_lowercase(), s.to_owned());
            }
        }

        let mut body = Vec::new();
        let mut limited = (&mut resp).take((MAX_RESPONSE_BYTES + 1) as u64);
        limited
            .read_to_end(&mut body)
            .map_err(|e| format!("failed reading HTTP response body: {e}"))?;
        if body.len() > MAX_RESPONSE_BYTES {
            body.truncate(MAX_RESPONSE_BYTES);
        }

        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

fn redact_error(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        return "timeout".to_owned();
    }
    if err.is_connect() {
        return "connect_error".to_owned();
    }
    "transport_error".to_owned()
}

/// Offline mock [`WebTransport`] recording requests and replying from queued or URL-matched stubs.
#[derive(Debug, Default)]
pub struct MockWebTransport {
    recorded: Mutex<Vec<HttpRequest>>,
    routes: Mutex<BTreeMap<String, VecDeque<Result<HttpResponse, String>>>>,
    fallback_queue: Mutex<VecDeque<Result<HttpResponse, String>>>,
}

impl MockWebTransport {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a response for requests whose URL starts with or equals `url_prefix`.
    pub fn push_route(&self, url_prefix: impl Into<String>, resp: Result<HttpResponse, String>) {
        self.routes
            .lock()
            .expect("mock routes lock")
            .entry(url_prefix.into())
            .or_default()
            .push_back(resp);
    }

    /// Pushes a response onto the FIFO fallback queue when no prefix route matches.
    pub fn push_response(&self, resp: Result<HttpResponse, String>) {
        self.fallback_queue
            .lock()
            .expect("mock queue lock")
            .push_back(resp);
    }

    /// Returns a snapshot of all recorded [`HttpRequest`]s.
    #[must_use]
    pub fn recorded_requests(&self) -> Vec<HttpRequest> {
        self.recorded.lock().expect("mock recorded lock").clone()
    }
}

impl WebTransport for MockWebTransport {
    fn send_request(&self, req: HttpRequest) -> Result<HttpResponse, String> {
        let url = req.url.clone();
        self.recorded
            .lock()
            .expect("mock recorded lock")
            .push(req);

        {
            let mut routes = self.routes.lock().expect("mock routes lock");
            // Prefer longest matching prefix.
            let matched_key = routes
                .keys()
                .filter(|prefix| url.starts_with(prefix.as_str()))
                .max_by_key(|prefix| prefix.len())
                .cloned();
            if let Some(key) = matched_key
                && let Some(queue) = routes.get_mut(&key)
                && let Some(next) = queue.pop_front()
            {
                return next;
            }
        }

        if let Some(next) = self
            .fallback_queue
            .lock()
            .expect("mock queue lock")
            .pop_front()
        {
            return next;
        }

        Err(format!("MockWebTransport: unexpected request to '{url}'"))
    }
}
