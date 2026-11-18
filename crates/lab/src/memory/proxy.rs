//! Lab-local OpenAI-compatible metering proxy in front of OpenCode Go (`Design 1`).
//!
//! Every call of a participant or the judge goes through this loopback server. The proxy
//! overwrites the model and `reasoning_effort`, sets the Lab User-Agent and a stable
//! `x-opencode-session` per `(participant, conversation)`, aborts the upstream request when the
//! client gives up, and appends exactly one ledger line per call. Header values and API keys are
//! never written.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use monoagens_reason::secrets::SecretString;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::LabError;
use crate::sha256_hex;

/// List price for `deepseek-v4.1-flash` on OpenCode Go, USD per million tokens (`R-58`).
pub const PRICE_UNCACHED_INPUT_PER_M: f64 = 0.15;
/// Cached input read price, USD per million tokens.
pub const PRICE_CACHED_INPUT_PER_M: f64 = 0.003;
/// Output (including reasoning) price, USD per million tokens.
pub const PRICE_OUTPUT_PER_M: f64 = 0.60;

/// Upstream OpenCode Go chat-completions endpoint (`R-58`).
pub const OPENCODE_GO_CHAT_COMPLETIONS_URL: &str =
    "https://opencode.ai/inference/go/openai/v1/chat/completions";

/// Model and effort used by every memory-suite call and the judge.
pub const MEMORY_MODEL: &str = "deepseek-v4.1-flash";
/// Reasoning effort used by every memory-suite call and the judge.
pub const MEMORY_REASONING_EFFORT: &str = "high";
/// Recorded answer-format instruction version.
pub const ANSWER_FORMAT_V1: &str = "answer_format_v1";

/// Attribution carried from a Lab client to the proxy in `x-lab-*` headers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallAttribution {
    pub participant: String,
    pub dataset: String,
    pub conversation: String,
    pub phase: String,
}

impl CallAttribution {
    /// Builds attribution for one call.
    #[must_use]
    pub fn new(
        participant: impl Into<String>,
        dataset: impl Into<String>,
        conversation: impl Into<String>,
        phase: impl Into<String>,
    ) -> Self {
        Self {
            participant: participant.into(),
            dataset: dataset.into(),
            conversation: conversation.into(),
            phase: phase.into(),
        }
    }

    /// Per-request headers sent by a Lab client to the proxy.
    #[must_use]
    pub fn headers(&self) -> Vec<(String, String)> {
        vec![
            ("x-lab-participant".to_owned(), self.participant.clone()),
            ("x-lab-dataset".to_owned(), self.dataset.clone()),
            ("x-lab-conversation".to_owned(), self.conversation.clone()),
            ("x-lab-phase".to_owned(), self.phase.clone()),
        ]
    }
}

/// One ledger line, written for every proxied call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub participant: String,
    pub dataset: String,
    pub conversation: String,
    pub phase: String,
    #[serde(default)]
    pub purpose: String,
    pub model: String,
    pub reasoning_effort: String,
    /// Input tokens reported by the provider (`prompt_tokens`); `None` when unavailable.
    pub prompt_tokens: Option<u64>,
    pub uncached_input_tokens: u64,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: u64,
    pub reasoning_tokens: Option<u64>,
    pub latency_ms: u64,
    pub list_cost_usd: f64,
    pub allowance_cost_usd: f64,
    pub status: String,
    /// Provider `finish_reason` (for example `length` when the output cap was hit).
    #[serde(default)]
    pub finish_reason: Option<String>,
    pub http_status: Option<u16>,
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Token and cost fields parsed from one successful chat-completions response.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedUsage {
    pub prompt_tokens: Option<u64>,
    pub uncached_input_tokens: u64,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: u64,
    pub reasoning_tokens: Option<u64>,
    pub reported_list_cost_usd: Option<f64>,
    pub reported_allowance_usd: Option<f64>,
}

/// Peak-hour multiplier at `now` (2× Mon–Fri 01:00–04:00 and 06:00–10:00 UTC; else 1×).
#[must_use]
pub fn peak_multiplier_at(now: SystemTime) -> f64 {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let day = secs / 86_400;
    let hour = (secs % 86_400) / 3_600;
    // 1970-01-01 was a Thursday; Monday = 0 after this shift.
    let weekday = (day + 3) % 7;
    let weekday_peak = weekday < 5 && ((1..4).contains(&hour) || (6..10).contains(&hour));
    if weekday_peak { 2.0 } else { 1.0 }
}

/// List-price cost in USD for one call, applying the peak-hour multiplier.
#[must_use]
pub fn list_cost_usd(uncached_input: u64, cached_input: u64, output: u64, multiplier: f64) -> f64 {
    (uncached_input as f64 * PRICE_UNCACHED_INPUT_PER_M
        + cached_input as f64 * PRICE_CACHED_INPUT_PER_M
        + output as f64 * PRICE_OUTPUT_PER_M)
        / 1_000_000.0
        * multiplier
}

/// Derives the stable `x-opencode-session` value for a participant and conversation.
/// The value itself is never written to any file.
#[must_use]
pub fn session_id_for_attribution(participant: &str, conversation: &str) -> String {
    let digest = sha256_hex(format!("{participant}\u{1f}{conversation}").as_bytes());
    format!("ses_{}", &digest[..32])
}

/// Extracts metering fields from an OpenAI-compatible response body.
#[must_use]
pub fn parse_usage(response: &serde_json::Value, _multiplier: f64) -> Option<ParsedUsage> {
    let usage = response.get("usage")?;
    let opencode_tokens = usage.get("opencode").and_then(|o| o.get("tokens"));
    let prompt_tokens = usage
        .get("prompt_tokens")
        .and_then(serde_json::Value::as_u64);
    let cached = opencode_tokens
        .and_then(|t| t.get("cache_read"))
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            usage
                .get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .and_then(serde_json::Value::as_u64)
        });
    // OpenCode's `tokens.input` excludes cache reads; OpenAI's `prompt_tokens` includes them.
    let uncached = match opencode_tokens
        .and_then(|t| t.get("input"))
        .and_then(serde_json::Value::as_u64)
    {
        Some(v) => v,
        None => prompt_tokens
            .unwrap_or(0)
            .saturating_sub(cached.unwrap_or(0)),
    };
    let output = opencode_tokens
        .and_then(|t| t.get("output"))
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            usage
                .get("completion_tokens")
                .and_then(serde_json::Value::as_u64)
        })
        .unwrap_or(0);
    let reasoning = opencode_tokens
        .and_then(|t| t.get("reasoning"))
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            usage
                .get("completion_tokens_details")
                .and_then(|d| d.get("reasoning_tokens"))
                .and_then(serde_json::Value::as_u64)
        });
    let reported_list = usage
        .get("opencode")
        .and_then(|o| o.get("list_cost"))
        .and_then(serde_json::Value::as_f64);
    let reported_allowance = usage
        .get("opencode")
        .and_then(|o| o.get("allowance"))
        .and_then(serde_json::Value::as_f64);
    Some(ParsedUsage {
        prompt_tokens,
        uncached_input_tokens: uncached,
        cached_input_tokens: cached,
        output_tokens: output,
        reasoning_tokens: reasoning,
        reported_list_cost_usd: reported_list,
        reported_allowance_usd: reported_allowance,
    })
}

/// Extracts the provider `finish_reason` from a chat-completions response.
#[must_use]
pub fn parse_finish_reason(response: &serde_json::Value) -> Option<String> {
    response
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("finish_reason"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// Aggregated ledger totals, used to reconcile run records with what the provider billed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LedgerTotals {
    pub calls: u64,
    pub errors: u64,
    pub uncached_input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cached_unknown_calls: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub list_cost_usd: f64,
    pub allowance_cost_usd: f64,
    pub latencies_ms: Vec<u64>,
}

impl LedgerTotals {
    /// Adds one ledger line into the totals.
    pub fn add(&mut self, e: &LedgerEntry) {
        self.calls += 1;
        if e.status != "ok" {
            self.errors += 1;
        }
        self.uncached_input_tokens += e.uncached_input_tokens;
        match e.cached_input_tokens {
            Some(c) => self.cached_input_tokens += c,
            None => self.cached_unknown_calls += 1,
        }
        self.output_tokens += e.output_tokens;
        self.reasoning_tokens += e.reasoning_tokens.unwrap_or(0);
        self.list_cost_usd += e.list_cost_usd;
        self.allowance_cost_usd += e.allowance_cost_usd;
        self.latencies_ms.push(e.latency_ms);
    }
}

/// Reads a JSONL ledger and computes totals, optionally filtered by participant.
pub fn read_ledger_totals(
    path: &Path,
    participant: Option<&str>,
) -> Result<LedgerTotals, LabError> {
    let mut totals = LedgerTotals::default();
    let file = File::open(path)?;
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<LedgerEntry>(&line) else {
            continue;
        };
        if participant.is_some_and(|p| entry.participant != p) {
            continue;
        }
        totals.add(&entry);
    }
    Ok(totals)
}

/// Configuration for [`MeteringProxy`].
#[derive(Clone)]
pub struct MeteringProxyConfig {
    pub upstream_url: String,
    pub api_key: Option<SecretString>,
    pub model: String,
    pub reasoning_effort: String,
    pub user_agent: String,
    pub ledger_path: PathBuf,
    pub max_retries: u32,
}

impl std::fmt::Debug for MeteringProxyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeteringProxyConfig")
            .field("upstream_url", &self.upstream_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("model", &self.model)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("user_agent", &self.user_agent)
            .field("ledger_path", &self.ledger_path)
            .field("max_retries", &self.max_retries)
            .finish()
    }
}

impl MeteringProxyConfig {
    /// Standard phase 0 configuration: OpenCode Go, DeepSeek v4.1 Flash, effort `high`.
    #[must_use]
    pub fn phase0(api_key: Option<SecretString>, ledger_path: PathBuf) -> Self {
        Self {
            upstream_url: OPENCODE_GO_CHAT_COMPLETIONS_URL.to_owned(),
            api_key,
            model: MEMORY_MODEL.to_owned(),
            reasoning_effort: MEMORY_REASONING_EFFORT.to_owned(),
            user_agent: format!("monoagens-lab/{}", env!("CARGO_PKG_VERSION")),
            ledger_path,
            max_retries: 3,
        }
    }
}

struct Shared {
    cfg: MeteringProxyConfig,
    ledger: Mutex<File>,
    client: reqwest::Client,
}

/// Running loopback metering proxy. Dropping it stops the accept loop.
pub struct MeteringProxy {
    addr: SocketAddr,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MeteringProxy {
    /// Binds a loopback port and starts serving.
    pub fn start(cfg: MeteringProxyConfig) -> Result<Self, LabError> {
        if let Some(parent) = cfg.ledger_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&cfg.ledger_path)?;
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| LabError::Protocol(format!("proxy HTTP client: {e}")))?;
        let shared = Arc::new(Shared {
            cfg,
            ledger: Mutex::new(file),
            client,
        });
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let shared = Arc::clone(&shared);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                else {
                    return;
                };
                runtime.block_on(async move {
                    let Ok(listener) = TcpListener::from_std(listener) else {
                        return;
                    };
                    while !stop.load(Ordering::Relaxed) {
                        match tokio::time::timeout(Duration::from_millis(50), listener.accept())
                            .await
                        {
                            Ok(Ok((stream, _))) => {
                                let shared = Arc::clone(&shared);
                                tokio::spawn(async move {
                                    let _ = handle_connection(stream, &shared).await;
                                });
                            }
                            Ok(Err(_)) => break,
                            Err(_) => continue,
                        }
                    }
                });
            })
        };
        Ok(Self {
            addr,
            shared,
            stop,
            handle: Some(handle),
        })
    }

    /// Loopback base URL, e.g. `http://127.0.0.1:41234`.
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// OpenAI-compatible chat-completions endpoint served by this proxy.
    #[must_use]
    pub fn chat_completions_url(&self) -> String {
        format!("{}/v1/chat/completions", self.base_url())
    }

    /// Path of the JSONL ledger.
    #[must_use]
    pub fn ledger_path(&self) -> &Path {
        &self.shared.cfg.ledger_path
    }

    /// Aggregates ledger lines, optionally for one participant.
    pub fn totals(&self, participant: Option<&str>) -> Result<LedgerTotals, LabError> {
        read_ledger_totals(&self.shared.cfg.ledger_path, participant)
    }
}

impl Drop for MeteringProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = std::net::TcpStream::connect(self.addr);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct HttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

async fn read_http_request(stream: &mut TcpStream) -> std::io::Result<HttpRequest> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let header_end;
    loop {
        let n = tokio::time::timeout(Duration::from_secs(60), stream.read(&mut tmp))
            .await
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "request read timeout")
            })??;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed before headers",
            ));
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
        if buf.len() > 64 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request headers too large",
            ));
        }
    }
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_owned(), value.trim().to_owned()));
        }
    }
    let content_length = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buf[header_end..].to_vec();
    while body.len() < content_length {
        let n = tokio::time::timeout(Duration::from_secs(60), stream.read(&mut tmp))
            .await
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "request body timeout")
            })??;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn write_http_response(
    stream: &mut TcpStream,
    status: u16,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        504 => "Gateway Timeout",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

async fn handle_connection(mut stream: TcpStream, shared: &Shared) -> std::io::Result<()> {
    let req = match read_http_request(&mut stream).await {
        Ok(req) => req,
        Err(_) => return Ok(()),
    };
    if req.method == "GET" && req.path == "/health" {
        return write_http_response(&mut stream, 200, b"{\"status\":\"ok\"}").await;
    }
    if req.method != "POST" || !req.path.starts_with("/v1/chat/completions") {
        return write_http_response(&mut stream, 404, b"{\"error\":\"not found\"}").await;
    }
    forward_chat_completion(&mut stream, shared, &req).await
}

/// Resolves when the client closes its side of the connection, which cancels the upstream call.
async fn client_disconnected(stream: &mut TcpStream) {
    let mut buf = [0u8; 1];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(_) => continue,
        }
    }
}

async fn forward_chat_completion(
    stream: &mut TcpStream,
    shared: &Shared,
    req: &HttpRequest,
) -> std::io::Result<()> {
    let attribution = CallAttribution::new(
        req.header("x-lab-participant").unwrap_or("unknown"),
        req.header("x-lab-dataset").unwrap_or("unknown"),
        req.header("x-lab-conversation").unwrap_or("unknown"),
        req.header("x-lab-phase").unwrap_or("unknown"),
    );
    let purpose = req.header("x-lab-purpose").unwrap_or("unknown").to_owned();
    let started = Instant::now();
    let mut payload: serde_json::Value = match serde_json::from_slice(&req.body) {
        Ok(v) => v,
        Err(err) => {
            let body = format!("{{\"error\":\"invalid request JSON: {err}\"}}");
            write_ledger(
                shared,
                &aborted_entry(&attribution, &purpose, started, 400, err.to_string()),
            );
            return write_http_response(stream, 400, body.as_bytes()).await;
        }
    };
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(
            "model".to_owned(),
            serde_json::Value::String(shared.cfg.model.clone()),
        );
        obj.insert(
            "reasoning_effort".to_owned(),
            serde_json::Value::String(shared.cfg.reasoning_effort.clone()),
        );
        obj.insert("stream".to_owned(), serde_json::Value::Bool(false));
    }

    let session = session_id_for_attribution(&attribution.participant, &attribution.conversation);
    let mut attempts: u32 = 0;
    let (status, raw_body) = loop {
        attempts += 1;
        let mut builder = shared
            .client
            .post(&shared.cfg.upstream_url)
            .header("content-type", "application/json")
            .header("user-agent", &shared.cfg.user_agent)
            .header("x-opencode-session", &session)
            .json(&payload);
        if let Some(key) = &shared.cfg.api_key {
            builder = builder.bearer_auth(key.expose());
        }
        let send = builder.send();
        tokio::pin!(send);
        let outcome = tokio::select! {
            res = &mut send => Some(res),
            _ = client_disconnected(stream) => None,
        };
        let Some(outcome) = outcome else {
            // The client gave up: cancel the upstream request and record an aborted call.
            write_ledger(
                shared,
                &aborted_entry(
                    &attribution,
                    &purpose,
                    started,
                    499,
                    "client disconnected; upstream aborted".to_owned(),
                ),
            );
            return Ok(());
        };
        match outcome {
            Ok(resp) => {
                let code = resp.status().as_u16();
                let retry_after = resp
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.trim().parse::<u64>().ok());
                let body = resp.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
                let transient = code == 429 || code == 408 || (500..=599).contains(&code);
                if transient && attempts <= shared.cfg.max_retries {
                    let delay = retry_after
                        .map(|s| s.saturating_mul(1_000))
                        .unwrap_or_else(|| (500_u64 << (attempts - 1)).min(30_000));
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    continue;
                }
                break (code, body);
            }
            Err(err) => {
                let transient = err.is_timeout() || err.is_connect() || err.is_request();
                if transient && attempts <= shared.cfg.max_retries {
                    tokio::time::sleep(Duration::from_millis(
                        (500_u64 << (attempts - 1)).min(30_000),
                    ))
                    .await;
                    continue;
                }
                write_ledger(
                    shared,
                    &aborted_entry(&attribution, &purpose, started, 502, err.to_string()),
                );
                return write_http_response(
                    stream,
                    502,
                    format!("{{\"error\":\"upstream error: {err}\"}}").as_bytes(),
                )
                .await;
            }
        }
    };

    let parsed = serde_json::from_slice::<serde_json::Value>(&raw_body).ok();
    let finish_reason = parsed.as_ref().and_then(parse_finish_reason);
    let entry = match parsed
        .as_ref()
        .filter(|_| (200..300).contains(&status))
        .and_then(|v| parse_usage(v, peak_multiplier_at(SystemTime::now())))
    {
        Some(usage) => {
            let multiplier = peak_multiplier_at(SystemTime::now());
            let list = usage.reported_list_cost_usd.unwrap_or_else(|| {
                list_cost_usd(
                    usage.uncached_input_tokens,
                    usage.cached_input_tokens.unwrap_or(0),
                    usage.output_tokens,
                    multiplier,
                )
            });
            LedgerEntry {
                participant: attribution.participant.clone(),
                dataset: attribution.dataset.clone(),
                conversation: attribution.conversation.clone(),
                phase: attribution.phase.clone(),
                purpose: purpose.clone(),
                model: shared.cfg.model.clone(),
                reasoning_effort: shared.cfg.reasoning_effort.clone(),
                prompt_tokens: usage.prompt_tokens,
                uncached_input_tokens: usage.uncached_input_tokens,
                cached_input_tokens: usage.cached_input_tokens,
                output_tokens: usage.output_tokens,
                reasoning_tokens: usage.reasoning_tokens,
                latency_ms: started.elapsed().as_millis() as u64,
                list_cost_usd: list,
                allowance_cost_usd: usage.reported_allowance_usd.unwrap_or(list),
                status: "ok".to_owned(),
                finish_reason,
                http_status: Some(status),
                attempts,
                error: None,
            }
        }
        None => LedgerEntry {
            participant: attribution.participant.clone(),
            dataset: attribution.dataset.clone(),
            conversation: attribution.conversation.clone(),
            phase: attribution.phase.clone(),
            purpose: purpose.clone(),
            model: shared.cfg.model.clone(),
            reasoning_effort: shared.cfg.reasoning_effort.clone(),
            prompt_tokens: None,
            uncached_input_tokens: 0,
            cached_input_tokens: None,
            output_tokens: 0,
            reasoning_tokens: None,
            latency_ms: started.elapsed().as_millis() as u64,
            list_cost_usd: 0.0,
            allowance_cost_usd: 0.0,
            status: "error".to_owned(),
            finish_reason,
            http_status: Some(status),
            attempts,
            error: Some(truncate_error(&raw_body)),
        },
    };
    write_ledger(shared, &entry);
    write_http_response(stream, status, &raw_body).await
}

fn truncate_error(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    text.chars().take(300).collect()
}

fn aborted_entry(
    attribution: &CallAttribution,
    purpose: &str,
    started: Instant,
    status: u16,
    error: String,
) -> LedgerEntry {
    LedgerEntry {
        participant: attribution.participant.clone(),
        dataset: attribution.dataset.clone(),
        conversation: attribution.conversation.clone(),
        phase: attribution.phase.clone(),
        purpose: purpose.to_owned(),
        model: MEMORY_MODEL.to_owned(),
        reasoning_effort: MEMORY_REASONING_EFFORT.to_owned(),
        prompt_tokens: None,
        uncached_input_tokens: 0,
        cached_input_tokens: None,
        output_tokens: 0,
        reasoning_tokens: None,
        latency_ms: started.elapsed().as_millis() as u64,
        list_cost_usd: 0.0,
        allowance_cost_usd: 0.0,
        status: "aborted".to_owned(),
        finish_reason: None,
        http_status: Some(status),
        attempts: 1,
        error: Some(error),
    }
}

fn write_ledger(shared: &Shared, entry: &LedgerEntry) {
    if let Ok(mut file) = shared.ledger.lock()
        && let Ok(line) = serde_json::to_string(entry)
    {
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}
