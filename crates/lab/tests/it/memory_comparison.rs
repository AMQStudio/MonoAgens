//! `lab-memory-comparison` Groups 1–3: proxy metering, judge prompts, scoring, pilot and report.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use monoagens_lab::datasets::framework::{
    ExternalItem, GoldBook, GoldEntry, HistoryMode, JudgeExecutionConfig,
};
use monoagens_lab::datasets::make_text_percept;
use monoagens_lab::memory::judge::JudgeRunner;
use monoagens_lab::memory::{
    CallAttribution, DatasetArtifact, LOCOMO_ANSWER_FORMAT_V1, LONGMEMEVAL_ANSWER_FORMAT_V1,
    MemoryItemScore, MemoryPhase, MemoryRunFile, MemorySuiteOptions, MeteringProxy,
    MeteringProxyConfig, generate_memory_report, list_cost_usd, locomo_judge_prompt,
    longmemeval_judge_prompt, parse_locomo_verdict, parse_longmemeval_verdict, parse_usage,
    score_locomo, score_longmemeval, select_pilot_locomo_ids, select_pilot_longmemeval_ids,
};
use monoagens_lab::record::{
    DatasetRunProvenance, MemoryUsageTotals, MetricScoreInterval, PROMPT_SOURCE_CORE_RENDER,
    PurposeCost, RunRecord, UsageBreakdown,
};
use monoagens_lab::{LabError, RunMetricsSummary, ScenarioSet};
use monoagens_protocol::{ActorId, Audience, PlaceId, VisibleScope};
use serde_json::json;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Loopback mock upstream
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Captured {
    headers: Vec<(String, String)>,
    body: serde_json::Value,
}

impl Captured {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

struct MockUpstream {
    addr: std::net::SocketAddr,
    captured: Arc<Mutex<Vec<Captured>>>,
    stop: Arc<AtomicUsize>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MockUpstream {
    fn start(
        responder: impl Fn(&Captured, usize) -> (u16, String) + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicUsize::new(0));
        let captured_thread = Arc::clone(&captured);
        let stop_thread = Arc::clone(&stop);
        let responder = Arc::new(responder);
        let handle = std::thread::spawn(move || {
            let mut seq = 0usize;
            while stop_thread.load(Ordering::Relaxed) == 0 {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                        if let Some(req) = read_request(&mut stream) {
                            captured_thread.lock().unwrap().push(req.clone());
                            let (status, body) = responder(&req, seq);
                            seq += 1;
                            write_response(&mut stream, status, &body);
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            addr,
            captured,
            stop,
            handle: Some(handle),
        }
    }

    fn url(&self) -> String {
        format!("http://{}/v1/chat/completions", self.addr)
    }

    fn captured(&self) -> Vec<Captured> {
        self.captured.lock().unwrap().clone()
    }
}

impl Drop for MockUpstream {
    fn drop(&mut self) {
        self.stop.store(1, Ordering::Relaxed);
        let _ = TcpStream::connect(self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Captured> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let header_end;
    loop {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let _request_line = lines.next();
    let mut headers = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_owned(), v.trim().to_owned()));
        }
    }
    let len = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buf[header_end..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    Some(Captured {
        headers,
        body: serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
    })
}

fn write_response(stream: &mut TcpStream, status: u16, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
}

fn thinking_or_answer(captured: &Captured) -> (u16, String) {
    let text = captured.body.to_string();
    if captured.body.get("response_format").is_some() {
        let answered =
            text.contains(LOCOMO_ANSWER_FORMAT_V1) || text.contains(LONGMEMEVAL_ANSWER_FORMAT_V1);
        let act = if answered {
            r#"{"acts":[{"act":"act","capability_id":"a.speak","places":["p"],"people":["soul"],"payload":[97,110,115,119,101,114],"motivation":5,"confidence_0_20":19,"used_ids":[]}]}"#
        } else {
            r#"{"acts":[{"act":"think","thought":"silent","motivation":1,"confidence_0_20":10,"used_ids":[]}]}"#
        };
        (
            200,
            json!({
                "id": "chatcmpl-mock",
                "model": "deepseek-v4.1-flash",
                "choices": [{"message": {"content": act}}],
                "usage": {
                    "prompt_tokens": 9200,
                    "completion_tokens": 300,
                    "prompt_tokens_details": {"cached_tokens": 7000},
                    "opencode": {"tokens": {"input": 1200, "output": 300, "reasoning": 120, "cache_read": 8000}}
                }
            })
            .to_string(),
        )
    } else {
        (
            200,
            json!({
                "id": "judge-mock",
                "model": "deepseek-v4.1-flash",
                "choices": [{"message": {"content": "{\"label\": \"CORRECT\"}"}}],
                "usage": {
                    "prompt_tokens": 500,
                    "completion_tokens": 10,
                    "prompt_tokens_details": {"cached_tokens": 100}
                }
            })
            .to_string(),
        )
    }
}

// ---------------------------------------------------------------------------
// Task 1.1 — proxy headers, effort, usage shapes, ledger attribution, secret scan
// ---------------------------------------------------------------------------

#[test]
fn metering_proxy_sets_wire_headers_parses_usage_and_never_writes_secrets() {
    let upstream = MockUpstream::start(|_captured, seq| {
        let usage = if seq == 0 {
            json!({
                "prompt_tokens": 9200,
                "completion_tokens": 300,
                "opencode": {"tokens": {"input": 1200, "output": 300, "reasoning": 120, "cache_read": 8000}}
            })
        } else {
            json!({
                "prompt_tokens": 5000,
                "completion_tokens": 40,
                "prompt_tokens_details": {"cached_tokens": 3000}
            })
        };
        (
            200,
            json!({
                "model": "deepseek-v4.1-flash",
                "choices": [{"message": {"content": "ok"}, "finish_reason": "length"}],
                "usage": usage
            })
            .to_string(),
        )
    });
    let tmp = TempDir::new().unwrap();
    let ledger = tmp.path().join("ledger.jsonl");
    let key = "sk-test-SECRET-KEY";
    let config = MeteringProxyConfig {
        upstream_url: upstream.url(),
        api_key: Some(monoagens_reason::secrets::SecretString::new(key)),
        model: "deepseek-v4.1-flash".to_owned(),
        reasoning_effort: "high".to_owned(),
        user_agent: "monoagens-lab-test/1.0".to_owned(),
        ledger_path: ledger.clone(),
        max_retries: 1,
    };
    let proxy = MeteringProxy::start(config).unwrap();
    let client = reqwest::blocking::Client::new();
    for (conversation, requested_effort) in [("conv-a", "low"), ("conv-b", "max")] {
        let attribution = CallAttribution::new("monoagens", "locomo", conversation, "answer");
        let mut req = client
            .post(proxy.chat_completions_url())
            .header("content-type", "application/json")
            .json(&json!({
                "model": "wrong-model",
                "reasoning_effort": requested_effort,
                "messages": [{"role": "user", "content": "hello"}]
            }));
        for (name, value) in attribution.headers() {
            req = req.header(name, value);
        }
        req = req.header("x-lab-purpose", "classify");
        let resp = req.send().unwrap();
        assert!(resp.status().is_success());
    }

    let captured = upstream.captured();
    assert_eq!(captured.len(), 2);
    for req in &captured {
        assert_eq!(
            req.body.get("model").and_then(|v| v.as_str()),
            Some("deepseek-v4.1-flash")
        );
        assert_eq!(
            req.body.get("reasoning_effort").and_then(|v| v.as_str()),
            Some("high")
        );
        assert_eq!(req.header("user-agent"), Some("monoagens-lab-test/1.0"));
        assert!(
            req.header("x-opencode-session")
                .unwrap()
                .starts_with("ses_")
        );
        assert_eq!(
            req.header("authorization"),
            Some(format!("Bearer {key}").as_str())
        );
        assert!(req.header("x-lab-participant").is_none());
        assert!(req.header("x-lab-dataset").is_none());
    }
    assert_ne!(
        captured[0].header("x-opencode-session"),
        captured[1].header("x-opencode-session")
    );

    let raw = fs::read_to_string(&ledger).unwrap();
    let lines: Vec<serde_json::Value> = raw
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["participant"], "monoagens");
    assert_eq!(lines[0]["dataset"], "locomo");
    assert_eq!(lines[0]["conversation"], "conv-a");
    assert_eq!(lines[0]["phase"], "answer");
    assert_eq!(lines[0]["uncached_input_tokens"], 1200);
    assert_eq!(lines[0]["cached_input_tokens"], 8000);
    assert_eq!(lines[0]["output_tokens"], 300);
    assert_eq!(lines[0]["reasoning_tokens"], 120);
    assert_eq!(lines[1]["cached_input_tokens"], 3000);
    assert_eq!(lines[1]["uncached_input_tokens"], 2000);
    assert_eq!(lines[0]["status"], "ok");
    assert_eq!(lines[0]["purpose"], "classify");
    assert_eq!(lines[0]["finish_reason"], "length");

    // Secret scan: the API key, Authorization value and the session value never hit disk.
    let session = monoagens_lab::memory::session_id_for_attribution("monoagens", "conv-a");
    for entry in walk(tmp.path()) {
        let text = fs::read_to_string(&entry).unwrap_or_default();
        assert!(
            !text.contains(key),
            "API key leaked into {}",
            entry.display()
        );
        assert!(
            !text.contains("Bearer "),
            "authorization leaked into {}",
            entry.display()
        );
        assert!(
            !text.contains(&session),
            "session value leaked into {}",
            entry.display()
        );
    }
}

#[test]
fn proxy_aborts_upstream_when_client_gives_up() {
    let upstream = MockUpstream::start(|_captured, _seq| {
        std::thread::sleep(Duration::from_secs(2));
        (
            200,
            json!({"choices":[{"message":{"content":"late"},"finish_reason":"stop"}],
                   "usage":{"prompt_tokens":10,"completion_tokens":5}})
            .to_string(),
        )
    });
    let tmp = TempDir::new().unwrap();
    let proxy = MeteringProxy::start(MeteringProxyConfig {
        upstream_url: upstream.url(),
        api_key: Some(monoagens_reason::secrets::SecretString::new("sk-test")),
        model: "deepseek-v4.1-flash".to_owned(),
        reasoning_effort: "high".to_owned(),
        user_agent: "monoagens-lab-test/1.0".to_owned(),
        ledger_path: tmp.path().join("ledger.jsonl"),
        max_retries: 0,
    })
    .unwrap();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    let result = client
        .post(proxy.chat_completions_url())
        .header("content-type", "application/json")
        .header("x-lab-participant", "monoagens")
        .header("x-lab-dataset", "locomo")
        .header("x-lab-conversation", "conv-a")
        .header("x-lab-phase", "answer")
        .header("x-lab-purpose", "reason")
        .json(&json!({"model":"x","messages":[{"role":"user","content":"hi"}]}))
        .send();
    assert!(result.is_err(), "client must time out");
    std::thread::sleep(Duration::from_millis(600));
    let raw = fs::read_to_string(proxy.ledger_path()).unwrap();
    let lines: Vec<serde_json::Value> = raw
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["status"], "aborted");
    assert_eq!(lines[0]["purpose"], "reason");
    assert_eq!(lines[0]["output_tokens"], 0);
}

#[test]
fn proxy_usage_parser_and_cost_match_list_price() {
    let response = json!({
        "usage": {
            "prompt_tokens": 9200,
            "completion_tokens": 300,
            "opencode": {"tokens": {"input": 1200, "output": 300, "reasoning": 120, "cache_read": 8000}}
        }
    });
    let usage = parse_usage(&response, 1.0).unwrap();
    assert_eq!(usage.uncached_input_tokens, 1200);
    assert_eq!(usage.cached_input_tokens, Some(8000));
    assert_eq!(usage.output_tokens, 300);
    assert_eq!(usage.reasoning_tokens, Some(120));
    let expected = (1200.0 * 0.15 + 8000.0 * 0.003 + 300.0 * 0.60) / 1_000_000.0;
    assert!((list_cost_usd(1200, 8000, 300, 1.0) - expected).abs() < 1e-12);

    let fallback = json!({
        "usage": {
            "prompt_tokens": 5000,
            "completion_tokens": 40,
            "prompt_tokens_details": {"cached_tokens": 3000}
        }
    });
    let usage = parse_usage(&fallback, 1.0).unwrap();
    assert_eq!(usage.uncached_input_tokens, 2000);
    assert_eq!(usage.cached_input_tokens, Some(3000));
    assert_eq!(usage.output_tokens, 40);
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Task 1.2 — MonoAgens and judge through the proxy; ledger totals equal the record
// ---------------------------------------------------------------------------

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn suite_options(tmp: &TempDir, proxy_url: String, phase: MemoryPhase) -> MemorySuiteOptions {
    MemorySuiteOptions {
        datasets_dir: workspace_root().join("lab/datasets"),
        manifest_path: Some(workspace_root().join("lab/datasets.manifest.toml")),
        run_dir: tmp.path().join("run"),
        prior: monoagens_store::load_prior_or_default(None).unwrap().prior,
        seed: 1,
        phase,
        live: true,
        max_cost_usd: Some(100.0),
        provider_endpoint: Some(proxy_url.clone()),
        judge_endpoint: Some(proxy_url),
        probe_provider: None,
        judge: None,
        embedder: None,
        write_report: false,
        only_datasets: Some(vec!["locomo".to_owned()]),
        max_items_per_dataset: Some(1),
        reuse_from: Vec::new(),
    }
}

#[test]
fn ledger_totals_equal_run_record_totals() {
    let upstream = MockUpstream::start(|captured, _seq| thinking_or_answer(captured));
    let tmp = TempDir::new().unwrap();
    let proxy = MeteringProxy::start(MeteringProxyConfig {
        upstream_url: upstream.url(),
        api_key: Some(monoagens_reason::secrets::SecretString::new("sk-test")),
        model: "deepseek-v4.1-flash".to_owned(),
        reasoning_effort: "high".to_owned(),
        user_agent: "monoagens-lab-test/1.0".to_owned(),
        ledger_path: tmp.path().join("run/ledger.jsonl"),
        max_retries: 1,
    })
    .unwrap();
    let opts = suite_options(&tmp, proxy.chat_completions_url(), MemoryPhase::Pilot);
    let outcome = monoagens_lab::memory::run_memory_suite(&opts).unwrap();
    assert_eq!(outcome.datasets.len(), 1);
    let record: RunRecord = serde_json::from_str(
        &fs::read_to_string(
            tmp.path()
                .join("run")
                .join(&outcome.datasets[0].record_rel_path),
        )
        .unwrap(),
    )
    .unwrap();
    let usage = record
        .dataset_provenance
        .as_ref()
        .and_then(|p| p.memory_usage.as_ref())
        .unwrap();
    let entries: Vec<serde_json::Value> = fs::read_to_string(proxy.ledger_path())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let phase = |p: &str| -> Vec<&serde_json::Value> {
        entries
            .iter()
            .filter(|e| e["participant"] == "monoagens" && e["phase"] == p)
            .collect()
    };
    assert_eq!(usage.ingest.calls as usize, phase("ingest").len());
    assert_eq!(usage.answer.calls as usize, phase("answer").len());
    assert!(usage.answer.calls > 0);
    let ledger_answer_tokens: u64 = phase("answer")
        .iter()
        .map(|e| e["output_tokens"].as_u64().unwrap())
        .sum();
    assert_eq!(usage.answer.output_tokens, ledger_answer_tokens);
    let judge_calls: u64 = entries
        .iter()
        .filter(|e| e["participant"] == "judge")
        .count() as u64;
    assert_eq!(usage.judge.calls, judge_calls);
    for entry in &entries {
        assert_eq!(entry["model"], "deepseek-v4.1-flash");
        assert_eq!(entry["reasoning_effort"], "high");
    }
}

#[test]
fn rerun_resumes_without_repeating_completed_conversations() {
    let upstream = MockUpstream::start(|captured, _seq| thinking_or_answer(captured));
    let tmp = TempDir::new().unwrap();
    let proxy = MeteringProxy::start(MeteringProxyConfig {
        upstream_url: upstream.url(),
        api_key: Some(monoagens_reason::secrets::SecretString::new("sk-test")),
        model: "deepseek-v4.1-flash".to_owned(),
        reasoning_effort: "high".to_owned(),
        user_agent: "monoagens-lab-test/1.0".to_owned(),
        ledger_path: tmp.path().join("run/ledger.jsonl"),
        max_retries: 1,
    })
    .unwrap();
    let opts = suite_options(&tmp, proxy.chat_completions_url(), MemoryPhase::Pilot);
    let first = monoagens_lab::memory::run_memory_suite(&opts).unwrap();
    let item_id = first.datasets[0].n_items;
    assert!(item_id > 0);
    let monoagens_lines = |entries: &[serde_json::Value]| {
        entries
            .iter()
            .filter(|e| e["participant"] == "monoagens")
            .count()
    };
    let lines_first = monoagens_lines(
        &fs::read_to_string(proxy.ledger_path())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect::<Vec<_>>(),
    );
    let _second = monoagens_lab::memory::run_memory_suite(&opts).unwrap();
    let lines_second = monoagens_lines(
        &fs::read_to_string(proxy.ledger_path())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect::<Vec<_>>(),
    );
    assert_eq!(
        lines_first, lines_second,
        "resume must not repeat a completed conversation"
    );
    let judge_lines = |entries: &[serde_json::Value]| {
        entries
            .iter()
            .filter(|e| e["participant"] == "judge")
            .count()
    };
    let all_second: Vec<serde_json::Value> = fs::read_to_string(proxy.ledger_path())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        judge_lines(&all_second) % 199,
        0,
        "judge calls must not be multiplied by a resumed scoring pass"
    );
    // Interrupt one conversation and resume: exactly that item runs again.
    let record: RunRecord = serde_json::from_str(
        &fs::read_to_string(
            tmp.path()
                .join("run")
                .join(&first.datasets[0].record_rel_path),
        )
        .unwrap(),
    )
    .unwrap();
    let chosen = &record.dataset_provenance.as_ref().unwrap().chosen_item_ids[0];
    fs::remove_file(
        tmp.path()
            .join("run/items/locomo")
            .join(format!("{chosen}.json")),
    )
    .unwrap();
    let _third = monoagens_lab::memory::run_memory_suite(&opts).unwrap();
    let lines_third = monoagens_lines(
        &fs::read_to_string(proxy.ledger_path())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect::<Vec<_>>(),
    );
    assert!(
        lines_third > lines_second,
        "the interrupted item must run again"
    );
}

// ---------------------------------------------------------------------------
// Task 2.2 — judge prompts (verbatim) and unparsable-verdict retry
// ---------------------------------------------------------------------------

#[test]
fn judge_prompts_are_verbatim_and_type_specific() {
    let locomo = locomo_judge_prompt("Where did Ava go?", "Paris", "Paris");
    assert!(
        locomo.contains("Your task is to label an answer to a question as ’CORRECT’ or ’WRONG’.")
    );
    assert!(locomo.contains("Question: Where did Ava go?"));
    assert!(locomo.contains("Gold answer: Paris"));
    assert!(locomo.contains("Generated answer: Paris"));
    assert!(locomo.contains(
        "Just return the label CORRECT or WRONG in a json format with the key as \"label\"."
    ));
    assert!(parse_locomo_verdict(r#"{"label":"CORRECT"}"#) == Some(true));
    assert!(parse_locomo_verdict(r#"{"label":"WRONG"}"#) == Some(false));
    assert!(parse_locomo_verdict("I think CORRECT.") == Some(true));

    let standard = longmemeval_judge_prompt("single-session-user", "Q?", "gold", "response", false);
    assert!(standard.contains("Please answer yes if the response contains the correct answer."));
    assert!(standard.contains("Question: Q?"));
    assert!(standard.contains("Correct Answer: gold"));
    assert!(standard.contains("Model Response: response"));
    let temporal =
        longmemeval_judge_prompt("temporal-reasoning", "Q?", "18 days", "19 days", false);
    assert!(temporal.contains("do not penalize off-by-one errors"));
    let update = longmemeval_judge_prompt("knowledge-update", "Q?", "new", "new", false);
    assert!(update.contains("previous information along with an updated answer"));
    let preference = longmemeval_judge_prompt(
        "single-session-preference",
        "Q?",
        "rubric",
        "response",
        true,
    );
    assert!(preference.contains("unanswerable question"));
    let abs = longmemeval_judge_prompt("single-session-user", "Q?", "explanation", "no", true);
    assert!(
        abs.contains("Correctly identifies".to_lowercase().as_str())
            || abs.contains("correctly identifies the question as unanswerable")
    );
    assert!(parse_longmemeval_verdict("Yes") == Some(true));
    assert!(parse_longmemeval_verdict("no") == Some(false));
    assert!(parse_longmemeval_verdict("maybe").is_none());
}

struct MockJudge {
    responses: Mutex<Vec<Result<String, LabError>>>,
    calls: AtomicUsize,
}

impl MockJudge {
    fn new(responses: Vec<Result<String, LabError>>) -> Self {
        Self {
            responses: Mutex::new(responses),
            calls: AtomicUsize::new(0),
        }
    }
}

impl JudgeRunner for MockJudge {
    fn judge(&self, _prompt: &str, _attribution: &CallAttribution) -> Result<String, LabError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let mut guard = self.responses.lock().unwrap();
        if guard.is_empty() {
            Ok("yes".to_owned())
        } else {
            guard.remove(0)
        }
    }

    fn calls(&self) -> u64 {
        self.calls.load(Ordering::Relaxed) as u64
    }
}

fn gold_entry(cat: &str, gold: &str) -> GoldEntry {
    GoldEntry {
        item_id: "item".to_owned(),
        probe_id: "q".to_owned(),
        cluster_id: "cluster".to_owned(),
        gold_answer: gold.to_owned(),
        accepted_answers: vec![gold.to_owned()],
        gold_evidence: Vec::new(),
        privacy_conflict: false,
        category: Some(cat.to_owned()),
        metadata: BTreeMap::new(),
    }
}

fn prediction(answer: Option<&str>) -> monoagens_lab::datasets::ExternalPrediction {
    monoagens_lab::datasets::ExternalPrediction {
        item_id: "item".to_owned(),
        probe_id: "q".to_owned(),
        cluster_id: "cluster".to_owned(),
        spoke: answer.is_some(),
        response_text: answer.map(str::to_owned),
        action_turn_ms: Some(1),
        probe_gate_wakes: 1,
        history_gate_wakes: 1,
        tokens_in: 1,
        tokens_out: 1,
        cost_usd: 0.0,
    }
}

fn fixture_gold() -> GoldBook {
    GoldBook {
        dataset_id: "locomo".to_owned(),
        entries: vec![gold_entry("cat_4", "Paris")],
    }
}

fn fixture_questions() -> BTreeMap<(String, String), String> {
    BTreeMap::from([(
        ("item".to_owned(), "q".to_owned()),
        "Where did Ava go?".to_owned(),
    )])
}

fn judge_config<'a>() -> JudgeExecutionConfig<'a> {
    JudgeExecutionConfig {
        live: true,
        judge_model: Some("deepseek-v4.1-flash"),
        judge_prompt_version: Some("test"),
        provider: None,
        datasets_dir: None,
        manifest: None,
        recorder: None,
    }
}

#[test]
fn unparsable_verdict_retries_once_then_records_judge_failure() {
    let judge = MockJudge::new(vec![Ok("maybe".to_owned()), Ok("CORRECT".to_owned())]);
    let score = score_locomo(
        &[prediction(Some("Paris"))],
        &fixture_gold(),
        &fixture_questions(),
        &BTreeMap::new(),
        Some(&judge),
        &judge_config(),
        Path::new("."),
        1,
    )
    .unwrap();
    assert_eq!(judge.calls(), 2);
    assert_eq!(score.items[0].judge_correct, Some(true));

    let failing = MockJudge::new(vec![Ok("???".to_owned()), Ok("still no label".to_owned())]);
    let score = score_locomo(
        &[prediction(Some("Paris"))],
        &fixture_gold(),
        &fixture_questions(),
        &BTreeMap::new(),
        Some(&failing),
        &judge_config(),
        Path::new("."),
        1,
    )
    .unwrap();
    assert_eq!(failing.calls(), 2);
    assert!(score.items[0].judge_failed);
    assert_eq!(score.items[0].judge_correct, None);
    assert_eq!(score.judge_failures, 1);
    let judge_metric = score
        .metrics
        .iter()
        .find(|m| m.name == "j_deepseek")
        .unwrap();
    assert_eq!(judge_metric.value, None);
}

// ---------------------------------------------------------------------------
// Task 2.3 — known answers, silent answer, abstention
// ---------------------------------------------------------------------------

#[test]
fn locomo_scoring_f1_refusal_and_silence() {
    let gold = GoldBook {
        dataset_id: "locomo".to_owned(),
        entries: vec![
            gold_entry("cat_4", "Paris"),
            GoldEntry {
                probe_id: "q5".to_owned(),
                ..gold_entry("cat_5", "No information available")
            },
        ],
    };
    let mut cat5 = prediction(Some("No information available"));
    cat5.probe_id = "q5".to_owned();
    let judge = MockJudge::new(vec![
        Ok(r#"{"label": "CORRECT"}"#.to_owned()),
        Ok(r#"{"label": "WRONG"}"#.to_owned()),
    ]);
    let score = score_locomo(
        &[prediction(Some("Paris")), cat5],
        &gold,
        &fixture_questions(),
        &BTreeMap::new(),
        Some(&judge),
        &judge_config(),
        Path::new("."),
        1,
    )
    .unwrap();
    assert_eq!(score.n_items, 2);
    assert!(
        (score
            .metrics
            .iter()
            .find(|m| m.name == "qa_f1")
            .unwrap()
            .value
            .unwrap()
            - 1.0)
            .abs()
            < 1e-12
    );
    assert_eq!(
        score
            .metrics
            .iter()
            .find(|m| m.name == "cat5_refusal_acc")
            .unwrap()
            .value,
        Some(1.0)
    );
    assert_eq!(
        score
            .metrics
            .iter()
            .find(|m| m.name == "j_deepseek")
            .unwrap()
            .value,
        Some(1.0)
    );

    let silent = score_locomo(
        &[prediction(None)],
        &fixture_gold(),
        &fixture_questions(),
        &BTreeMap::new(),
        None,
        &judge_config(),
        Path::new("."),
        1,
    )
    .unwrap();
    assert!(silent.items[0].silent);
    assert!((silent.silence_rate - 1.0).abs() < 1e-12);
    assert_eq!(
        silent
            .metrics
            .iter()
            .find(|m| m.name == "qa_f1")
            .unwrap()
            .value,
        Some(0.0)
    );
}

#[test]
fn longmemeval_scoring_reports_abstention_apart() {
    let gold = GoldBook {
        dataset_id: "longmemeval".to_owned(),
        entries: vec![
            GoldEntry {
                item_id: "item".to_owned(),
                probe_id: "q".to_owned(),
                cluster_id: "cluster".to_owned(),
                gold_answer: "gold".to_owned(),
                accepted_answers: vec!["gold".to_owned()],
                gold_evidence: Vec::new(),
                privacy_conflict: false,
                category: Some("multi-session".to_owned()),
                metadata: BTreeMap::from([("is_abs".to_owned(), "false".to_owned())]),
            },
            GoldEntry {
                item_id: "abs_item".to_owned(),
                probe_id: "q".to_owned(),
                cluster_id: "abs_cluster".to_owned(),
                gold_answer: "not answerable".to_owned(),
                accepted_answers: vec!["not answerable".to_owned()],
                gold_evidence: Vec::new(),
                privacy_conflict: false,
                category: Some("single-session-user".to_owned()),
                metadata: BTreeMap::from([("is_abs".to_owned(), "true".to_owned())]),
            },
        ],
    };
    let mut abs = prediction(Some("I don't know"));
    abs.item_id = "abs_item".to_owned();
    abs.cluster_id = "abs_cluster".to_owned();
    let judge = MockJudge::new(vec![Ok("yes".to_owned()), Ok("yes".to_owned())]);
    let score = score_longmemeval(
        &[prediction(Some("gold")), abs],
        &gold,
        &fixture_questions(),
        &BTreeMap::new(),
        Some(&judge),
        &judge_config(),
        Path::new("."),
        1,
    )
    .unwrap();
    assert_eq!(
        score
            .metrics
            .iter()
            .find(|m| m.name == "acc_j_deepseek")
            .unwrap()
            .value,
        Some(1.0)
    );
    assert_eq!(
        score
            .metrics
            .iter()
            .find(|m| m.name == "abs_acc_j_deepseek")
            .unwrap()
            .value,
        Some(1.0)
    );
    assert_eq!(
        score
            .metrics
            .iter()
            .find(|m| m.name == "acc_j_deepseek_task_avg")
            .unwrap()
            .value,
        Some(1.0)
    );
}

// ---------------------------------------------------------------------------
// Task 2.4 — pilot selection is a subset of the full run and stops
// ---------------------------------------------------------------------------

fn synthetic_item(id: &str, qtype: &str, is_abs: bool) -> ExternalItem {
    let soul = ActorId::new("soul");
    let user = ActorId::new("user");
    let place = PlaceId::new("p");
    let aud = Audience::new([user, soul]);
    ExternalItem {
        item_id: id.to_owned(),
        cluster_id: id.to_owned(),
        strata_key: format!("{qtype}:abs={is_abs}"),
        history: vec![make_text_percept(
            "user",
            "p",
            aud.members.iter().cloned(),
            1,
            "h",
        )],
        checkpoint_group: None,
        probes: vec![monoagens_lab::datasets::ExternalProbe {
            probe_id: "q".to_owned(),
            percept: make_text_percept("user", "p", aud.members.iter().cloned(), 2, "q"),
            isolated_checkpoint: true,
            probe_scope: VisibleScope::new(
                Audience::new([ActorId::new("user"), ActorId::new("soul")]),
                Some(place),
            ),
            expected_capability: None,
        }],
        history_mode: HistoryMode::StaticSilent,
        capabilities: Vec::new(),
    }
}

#[test]
fn pilot_ids_are_a_seeded_stratified_subset_of_the_full_run() {
    let locomo: Vec<ExternalItem> = ["conv-b", "conv-a", "conv-c"]
        .iter()
        .map(|id| synthetic_item(id, "cat_1", false))
        .collect();
    let locomo_ids = select_pilot_locomo_ids(&locomo);
    assert_eq!(locomo_ids, vec!["conv-a".to_owned()]);

    let mut items = Vec::new();
    let mut gold = GoldBook {
        dataset_id: "longmemeval".to_owned(),
        entries: Vec::new(),
    };
    let types = [
        "single-session-user",
        "single-session-assistant",
        "multi-session",
        "temporal-reasoning",
        "knowledge-update",
        "single-session-preference",
    ];
    for (t_idx, qtype) in types.iter().enumerate() {
        for i in 0..40 {
            let id = format!("lme_{t_idx:02}_{i:03}");
            items.push(synthetic_item(&id, qtype, i == 0 && t_idx == 0));
            let mut entry = gold_entry(qtype, "gold");
            entry.item_id = id.clone();
            entry.cluster_id = id.clone();
            gold.entries.push(entry);
        }
    }
    let ids = select_pilot_longmemeval_ids(&items, &gold, 20, 7);
    assert_eq!(ids.len(), 20, "pilot must select exactly 20 items");
    assert_eq!(ids, select_pilot_longmemeval_ids(&items, &gold, 20, 7));
    let all: std::collections::BTreeSet<String> = items.iter().map(|i| i.item_id.clone()).collect();
    assert!(ids.iter().all(|id| all.contains(id)));
    let by_type: BTreeMap<String, usize> = ids
        .iter()
        .map(|id| {
            gold.entries
                .iter()
                .find(|e| &e.item_id == id)
                .unwrap()
                .category
                .clone()
                .unwrap()
        })
        .fold(BTreeMap::new(), |mut acc, t| {
            *acc.entry(t).or_insert(0) += 1;
            acc
        });
    assert_eq!(by_type.len(), 6, "all question types are represented");
}

// ---------------------------------------------------------------------------
// Task 3.1 — report tables equal their record values
// ---------------------------------------------------------------------------

fn usage_breakdown(
    calls: u64,
    uncached: u64,
    cached: u64,
    output: u64,
    cost: f64,
) -> UsageBreakdown {
    UsageBreakdown {
        calls,
        uncached_input_tokens: uncached,
        cached_input_tokens: cached,
        cached_unknown_calls: 0,
        output_tokens: output,
        reasoning_tokens: 0,
        list_cost_usd: cost,
        allowance_cost_usd: cost,
    }
}

fn fixture_record() -> RunRecord {
    let metrics = vec![
        MetricScoreInterval {
            metric: "qa_f1".to_owned(),
            value: Some(0.75),
            ci95_low: Some(0.5),
            ci95_high: Some(0.9),
            n_clusters: Some(10),
            not_scored_reason: None,
        },
        MetricScoreInterval {
            metric: "j_deepseek".to_owned(),
            value: Some(0.625),
            ci95_low: Some(0.5),
            ci95_high: Some(0.75),
            n_clusters: Some(10),
            not_scored_reason: None,
        },
        MetricScoreInterval {
            metric: "qa_f1_cat_4".to_owned(),
            value: Some(0.75),
            ci95_low: Some(0.5),
            ci95_high: Some(0.9),
            n_clusters: Some(10),
            not_scored_reason: None,
        },
        MetricScoreInterval {
            metric: "j_deepseek_cat_4".to_owned(),
            value: Some(0.625),
            ci95_low: Some(0.5),
            ci95_high: Some(0.75),
            n_clusters: Some(10),
            not_scored_reason: None,
        },
        MetricScoreInterval {
            metric: "cat5_refusal_acc".to_owned(),
            value: Some(1.0),
            ci95_low: None,
            ci95_high: None,
            n_clusters: Some(1),
            not_scored_reason: None,
        },
        MetricScoreInterval {
            metric: "silence_rate".to_owned(),
            value: Some(0.25),
            ci95_low: None,
            ci95_high: None,
            n_clusters: None,
            not_scored_reason: None,
        },
    ];
    RunRecord {
        schema_version: "1.0".to_owned(),
        run_id: "fixture".to_owned(),
        core_version: "test".to_owned(),
        prior_content_version: "1".to_owned(),
        prior_format_version: 1,
        prior_digest: "digest".to_owned(),
        open_offset: 0,
        model_id: "deepseek-v4.1-flash".to_owned(),
        embedder_id: "none".to_owned(),
        door_rules: Vec::new(),
        switches: monoagens_core::Switches::all_enabled(),
        learning_switches: BTreeMap::new(),
        seed: 1,
        scenario_id: "locomo".to_owned(),
        scenario_set: ScenarioSet::External,
        sealed: false,
        checks: Vec::new(),
        passed: true,
        metrics: RunMetricsSummary {
            total_events: 4,
            percepts: 4,
            decisions_go: 4,
            decisions_note: 0,
            decisions_silent: 0,
            decisions_door_blocked: 0,
            actions_dispatched: 4,
            receipts_recorded: 0,
            wakes_per_event: 1.0,
            simulated_span_ms: 0,
            wall_elapsed_ms: 0,
            brain_map_metrics: None,
            detailed: None,
        },
        cost_by_purpose: vec![PurposeCost {
            purpose: monoagens_core::ModelPurpose::Reason,
            calls: 7,
            tokens_in: 1300,
            tokens_out: 400,
            cost_usd: 0.25,
        }],
        history_reasoning_mode: Some("live".to_owned()),
        monoagens_mode: Some("full".to_owned()),
        dataset_provenance: Some(DatasetRunProvenance {
            dataset_id: "locomo".to_owned(),
            pinned_revision: "rev".to_owned(),
            manifest_digest: "digest".to_owned(),
            converter_version: "1".to_owned(),
            scorer_version: "1".to_owned(),
            sampling_rule: "pilot".to_owned(),
            chosen_item_ids: vec!["conv-a".to_owned()],
            metric_scores: metrics,
            judge_model: Some("deepseek-v4.1-flash".to_owned()),
            judge_prompt_version: Some("v1".to_owned()),
            judge_calls: 4,
            judge_tokens_in: 400,
            judge_tokens_out: 20,
            history_gate_wakes: 4,
            partial_run: true,
            reasoning_effort: Some("high".to_owned()),
            judge_reasoning_effort: Some("high".to_owned()),
            extra_header_names: Vec::new(),
            provider_endpoint_host: None,
            embedder_backend: None,
            embedder_revision: None,
            prompt_source: Some(PROMPT_SOURCE_CORE_RENDER.to_owned()),
            development_exposure: Some("none".to_owned()),
            memory_usage: Some(MemoryUsageTotals {
                ingest: usage_breakdown(3, 1000, 500, 100, 0.01),
                answer: usage_breakdown(4, 300, 200, 300, 0.02),
                judge: usage_breakdown(4, 400, 100, 20, 0.005),
                latencies_ms: vec![10, 20, 30],
                by_purpose: BTreeMap::new(),
                runaway_by_purpose: BTreeMap::new(),
                max_output_tokens: Some(4096),
            }),
            category_metrics: BTreeMap::new(),
            failed_items: Vec::new(),
            model_latencies_ms: vec![10, 20, 30],
            started_at_utc: Some("2026-11-18T00:00:00Z".to_owned()),
            finished_at_utc: Some("2026-11-18T00:01:00Z".to_owned()),
        }),
    }
}

#[test]
fn memory_report_tables_come_from_run_records() {
    let tmp = TempDir::new().unwrap();
    let run_dir = tmp.path().join("run");
    fs::create_dir_all(run_dir.join("records")).unwrap();
    fs::create_dir_all(run_dir.join("items")).unwrap();
    let record = fixture_record();
    fs::write(
        run_dir.join("records/locomo__seed-1.json"),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
    let items = vec![
        MemoryItemScore {
            item_id: "conv-a".to_owned(),
            probe_id: "qa_0".to_owned(),
            cluster_id: "conv-a".to_owned(),
            category: "cat_4".to_owned(),
            question: "Where?".to_owned(),
            gold_answer: "Paris".to_owned(),
            answer: "London".to_owned(),
            spoke: true,
            silent: false,
            is_abs: false,
            token_f1: Some(0.0),
            judge_correct: Some(false),
            judge_verdict: Some("WRONG".to_owned()),
            judge_failed: false,
            failure_reason: None,
            transcript_rel_path: "transcripts/locomo/conv-a.jsonl".to_owned(),
        },
        MemoryItemScore {
            item_id: "conv-a".to_owned(),
            probe_id: "qa_1".to_owned(),
            cluster_id: "conv-a".to_owned(),
            category: "cat_5".to_owned(),
            question: "Q?".to_owned(),
            gold_answer: "x".to_owned(),
            answer: "No information available".to_owned(),
            spoke: true,
            silent: false,
            is_abs: false,
            token_f1: None,
            judge_correct: Some(true),
            judge_verdict: Some("CORRECT".to_owned()),
            judge_failed: false,
            failure_reason: None,
            transcript_rel_path: "transcripts/locomo/conv-a.jsonl".to_owned(),
        },
    ];
    fs::write(
        run_dir.join("items/locomo__scores.json"),
        serde_json::to_vec_pretty(&items).unwrap(),
    )
    .unwrap();
    let run = MemoryRunFile {
        model: "deepseek-v4.1-flash".to_owned(),
        reasoning_effort: "high".to_owned(),
        judge_model: "deepseek-v4.1-flash".to_owned(),
        judge_prompt_versions: BTreeMap::from([("locomo".to_owned(), "v1".to_owned())]),
        answer_format_version: "answer_format_v1".to_owned(),
        seed: 1,
        phase: "pilot".to_owned(),
        live: true,
        status: "complete".to_owned(),
        development_exposure: "none".to_owned(),
        started_at_utc: "2026-11-18T00:00:00Z".to_owned(),
        finished_at_utc: "2026-11-18T00:01:00Z".to_owned(),
        datasets: vec![DatasetArtifact {
            dataset_id: "locomo".to_owned(),
            record_rel_path: "records/locomo__seed-1.json".to_owned(),
            items_rel_path: "items/locomo__scores.json".to_owned(),
            n_items: 2,
        }],
        pilot: None,
        notes: Vec::new(),
    };
    fs::write(
        run_dir.join("memory_run.json"),
        serde_json::to_vec_pretty(&run).unwrap(),
    )
    .unwrap();
    let generated = generate_memory_report(&run_dir).unwrap();
    let md = fs::read_to_string(&generated.markdown_path).unwrap();
    assert!(md.contains("| cat_4 | 1 | 0.750 | 0.625 |"));
    assert!(md.contains("cat5_refusal_acc") || md.contains("| **Cat 5 refusal** | 1 | 1.000 |"));
    assert!(!md.contains("Mem0"), "no published scores");
    let csv = fs::read_to_string(run_dir.join("report/tables/locomo_categories.csv")).unwrap();
    assert!(csv.contains("cat_4,1,0.750,0.625"), "{csv}");
    let tokens = fs::read_to_string(run_dir.join("report/tables/tokens.csv")).unwrap();
    assert!(tokens.contains("locomo,ingest,3,1000,500,100,0,0.010000"));
    let html = fs::read_to_string(&generated.html_path).unwrap();
    assert!(html.contains("LoCoMo per-category scores"));
}

#[test]
fn registered_capability_yields_dispatched_speak_answer_and_configured_timeout() {
    use monoagens_lab::datasets::framework::execute_external_item_with_embedder;
    use monoagens_lab::reason_seam::model::{
        CallOutcome, MessagePart, ModelCallRecord, ModelCallSuccess, ModelOutput, ModelProvider,
        ModelRequest, TokenUsage,
    };
    use monoagens_protocol::{
        CapabilityDescriptor, CapabilityFlags, CapabilityId, CapabilitySchemaHash,
    };

    struct CapabilityProvider {
        seen_timeout: Arc<AtomicUsize>,
        seen_capability: Arc<AtomicUsize>,
    }
    impl ModelProvider for CapabilityProvider {
        fn provider_name(&self) -> &str {
            "mock-capability"
        }
        fn invoke(
            &self,
            request: &ModelRequest,
        ) -> Result<ModelCallSuccess, monoagens_lab::reason_seam::model::ModelCallFailure> {
            self.seen_timeout
                .store(request.timeout_ms as usize, Ordering::Relaxed);
            assert_eq!(request.max_output_tokens, Some(4096));
            let text: String = request
                .messages
                .iter()
                .flat_map(|m| &m.parts)
                .filter_map(|p| match p {
                    MessagePart::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.contains("a.speak") {
                self.seen_capability.store(1, Ordering::Relaxed);
            }
            let raw = r#"{"acts":[{"act":"act","capability_id":"a.speak","places":["p"],"people":["soul"],"payload":[52,52,49,55],"motivation":5,"confidence_0_20":19,"used_ids":[]}]}"#.to_owned();
            let output = ModelOutput::ThoughtActs(
                monoagens_lab::reason_seam::parse_thought_output(&raw).expect("strict batch"),
            );
            Ok(ModelCallSuccess {
                record: ModelCallRecord {
                    request_id: request.request_id,
                    purpose: request.purpose,
                    provider: "mock-capability".to_owned(),
                    model: request.model.clone(),
                    usage: Some(TokenUsage {
                        input_tokens: 10,
                        output_tokens: 2,
                        cached_tokens: Some(3),
                        reasoning_tokens: Some(1),
                        total_tokens: 12,
                    }),
                    latency_ms: 1,
                    outcome: CallOutcome::Success,
                },
                output,
                raw_text: raw,
            })
        }
    }

    let soul = ActorId::new("soul");
    let user = ActorId::new("user");
    let place = PlaceId::new("p");
    let aud = Audience::new([user.clone(), soul.clone()]);
    let probe = monoagens_lab::datasets::ExternalProbe {
        probe_id: "q".to_owned(),
        percept: make_text_percept(
            "user",
            "p",
            aud.members.iter().cloned(),
            2_000,
            "What is the vault code?",
        ),
        isolated_checkpoint: true,
        probe_scope: VisibleScope::new(aud.clone(), Some(place.clone())),
        expected_capability: None,
    };
    let item = ExternalItem {
        item_id: "cap_item".to_owned(),
        cluster_id: "cap_item".to_owned(),
        strata_key: "test".to_owned(),
        history: vec![make_text_percept(
            "user",
            "p",
            aud.members.iter().cloned(),
            1_000,
            "The vault code is 4417.",
        )],
        checkpoint_group: None,
        probes: vec![probe],
        history_mode: HistoryMode::StaticSilent,
        capabilities: vec![CapabilityDescriptor {
            id: CapabilityId::new("a.speak"),
            description: "Answer the question using remembered evidence.".to_owned(),
            places: vec![place],
            flags: CapabilityFlags {
                communicative: true,
                identity_changing: false,
                irreversible: false,
            },
            typical_latency_ms: 1,
            capability_schema_hash: CapabilitySchemaHash::new("sha256:cap-a.speak"),
            sensed_effect: None,
        }],
    };
    let provider = CapabilityProvider {
        seen_timeout: Arc::new(AtomicUsize::new(0)),
        seen_capability: Arc::new(AtomicUsize::new(0)),
    };
    let prior = monoagens_store::load_prior_or_default(None).unwrap().prior;
    let mut guard = monoagens_lab::CostGuard::default();
    let exec = execute_external_item_with_embedder(
        &item,
        &prior,
        &monoagens_core::Switches::all_enabled(),
        1,
        &provider,
        "mock-capability",
        false,
        &mut guard,
        None,
        None,
        Some(424_242),
        Some(4096),
    )
    .unwrap();
    assert_eq!(provider.seen_timeout.load(Ordering::Relaxed), 424_242);
    assert_eq!(provider.seen_capability.load(Ordering::Relaxed), 1);
    assert_eq!(exec.actions_dispatched, 1);
    assert_eq!(exec.predictions[0].response_text.as_deref(), Some("4417"));
}

#[test]
fn mock_history_prompts_carry_trigger_and_capability() {
    use monoagens_lab::datasets::framework::execute_external_item_with_embedder;
    use monoagens_lab::reason_seam::model::{
        CallOutcome, MessagePart, ModelCallRecord, ModelCallSuccess, ModelOutput, ModelProvider,
        ModelRequest, TokenUsage,
    };
    use monoagens_protocol::{
        CapabilityDescriptor, CapabilityFlags, CapabilityId, CapabilitySchemaHash,
    };

    struct RecordingProvider {
        seen: Mutex<Vec<ModelRequest>>,
    }
    impl ModelProvider for RecordingProvider {
        fn provider_name(&self) -> &str {
            "mock-recording"
        }
        fn invoke(
            &self,
            request: &ModelRequest,
        ) -> Result<ModelCallSuccess, monoagens_lab::reason_seam::model::ModelCallFailure> {
            self.seen.lock().unwrap().push(request.clone());
            let raw = if request.purpose == monoagens_core::ModelPurpose::Classify {
                r#"{"label":"note","rationale":"mock classify","score":0.0}"#.to_owned()
            } else {
                r#"{"acts":[{"act":"think","thought":"mock reason","motivation":1,"confidence_0_20":10,"used_ids":[]}]}"#.to_owned()
            };
            let output = if request.purpose == monoagens_core::ModelPurpose::Classify {
                ModelOutput::Verdict(
                    monoagens_lab::reason_seam::parse_classification_verdict(&raw).unwrap(),
                )
            } else {
                ModelOutput::ThoughtActs(
                    monoagens_lab::reason_seam::parse_thought_output(&raw).unwrap(),
                )
            };
            let usage = Some(TokenUsage {
                input_tokens: 42,
                output_tokens: 4,
                cached_tokens: Some(8),
                reasoning_tokens: Some(2),
                total_tokens: 46,
            });
            Ok(ModelCallSuccess {
                record: ModelCallRecord {
                    request_id: request.request_id,
                    purpose: request.purpose,
                    provider: "mock-recording".to_owned(),
                    model: request.model.clone(),
                    usage,
                    latency_ms: 1,
                    outcome: CallOutcome::Success,
                },
                output,
                raw_text: raw,
            })
        }
    }

    fn item(dataset: &str) -> ExternalItem {
        let soul = ActorId::new("soul");
        let user = ActorId::new("user");
        let place = PlaceId::new(format!("{dataset}_p"));
        let aud = Audience::new([user.clone(), soul.clone()]);
        ExternalItem {
            item_id: format!("{dataset}_item"),
            cluster_id: format!("{dataset}_item"),
            strata_key: "test".to_owned(),
            history: (0..6)
                .map(|i| {
                    make_text_percept(
                        "user",
                        place.as_str(),
                        aud.members.iter().cloned(),
                        1_000 + i,
                        format!("Caroline said utterance number {i} about the adoption interview"),
                    )
                })
                .collect(),
            checkpoint_group: None,
            probes: vec![monoagens_lab::datasets::ExternalProbe {
                probe_id: "q".to_owned(),
                percept: make_text_percept(
                    "user",
                    place.as_str(),
                    aud.members.iter().cloned(),
                    2_000,
                    "When did Caroline pass the adoption interviews?",
                ),
                isolated_checkpoint: true,
                probe_scope: VisibleScope::new(aud, Some(place.clone())),
                expected_capability: None,
            }],
            history_mode: HistoryMode::StaticSilent,
            capabilities: vec![CapabilityDescriptor {
                id: CapabilityId::new("a.speak"),
                description: "Answer using remembered evidence.".to_owned(),
                places: vec![place],
                flags: CapabilityFlags {
                    communicative: true,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 1,
                capability_schema_hash: CapabilitySchemaHash::new("sha256:cap-a.speak"),
                sensed_effect: None,
            }],
        }
    }

    let prior = monoagens_store::load_prior_or_default(None).unwrap().prior;
    for dataset in ["locomo", "longmemeval"] {
        let provider = RecordingProvider {
            seen: Mutex::new(Vec::new()),
        };
        let mut guard = monoagens_lab::CostGuard::default();
        let exec = execute_external_item_with_embedder(
            &item(dataset),
            &prior,
            &monoagens_core::Switches::all_enabled(),
            1,
            &provider,
            "mock-recording",
            true,
            &mut guard,
            None,
            None,
            Some(600_000),
            Some(4096),
        )
        .unwrap();
        let seen = provider.seen.lock().unwrap().clone();
        let text = |r: &ModelRequest| {
            r.messages
                .iter()
                .flat_map(|m| &m.parts)
                .filter_map(|p| match p {
                    MessagePart::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        for request in &seen {
            let prompt = text(request);
            assert!(
                prompt.contains("a.speak"),
                "{dataset} prompt missing a.speak"
            );
            assert_eq!(request.max_output_tokens, Some(4096));
            assert_eq!(request.timeout_ms, 600_000);
        }
        let classify = seen
            .iter()
            .find(|r| r.purpose == monoagens_core::ModelPurpose::Classify)
            .map(text);
        let reason = seen
            .iter()
            .find(|r| r.purpose == monoagens_core::ModelPurpose::Reason)
            .map(text);
        eprintln!(
            "{dataset} calls={} classify={}",
            seen.len(),
            classify.is_some()
        );
        if let Some(prompt) = &classify {
            eprintln!(
                "{dataset} classify trigger: {:?}",
                prompt
                    .split("[TRIGGER]")
                    .nth(1)
                    .unwrap_or("")
                    .split("[/TRIGGER]")
                    .next()
                    .unwrap_or("")
                    .trim()
            );
        }
        if let Some(prompt) = &reason {
            eprintln!(
                "{dataset} reason trigger: {:?}",
                prompt
                    .split("[TRIGGER]")
                    .nth(1)
                    .unwrap_or("")
                    .split("[/TRIGGER]")
                    .next()
                    .unwrap_or("")
                    .trim()
            );
        }
        assert_eq!(exec.percepts_count, 7);
    }
}
