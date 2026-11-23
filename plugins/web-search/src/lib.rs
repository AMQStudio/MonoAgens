//! Waist-2 (`monoagens.body.v1`) Web Search and SSRF-guarded Fetch capability plugin (`plugin-web-search`).
//!
//! Declares capabilities `web.fetch` and `web.search` under place `cap:web-search`, enforcing
//! fail-closed SSRF validation across initial URLs, resolved IP addresses, and every redirect hop.

#![forbid(unsafe_code)]

pub mod config;
pub mod fetch;
pub mod html;
pub mod providers;
pub mod ssrf;
pub mod transport;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use monoagens_body_kit::{
    HonestCueThresholds, capability_schema_hash, make_refused_receipt, make_terminal_receipt,
};
use monoagens_body_sdk::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult, ActionId,
    ApprovalResolveParams, ApprovalResolveResult, Audience, BodyDiscoverParams, BodyDiscoverResult,
    BodyHandler, BodyId, BodyShutdownParams, BodyShutdownResult, CURRENT_PROTOCOL_VERSION,
    CapabilityDescriptor, CapabilityFlags, CapabilityId, MetaValue, PerceptEmitParams,
    PerceptEmitResult, PlaceId, ReceiptOutcome, TerminationReason, Time, Waist2Error,
    sort_capabilities,
};
use serde::Deserialize;
use serde_json::json;

pub use config::{
    EnvSource, MemoryEnv, ProcessEnv, WebSearchConfig, resolve_state_dir_from_env,
};
pub use fetch::{FetchError, FetchErrorKind, FetchOutput, fetch_page};
pub use html::{ExtractedPage, extract_html_text};
pub use providers::{
    ProviderAttempt, SearchChainError, SearchOutput, SearchResultItem, search_web,
};
pub use ssrf::{DnsResolver, MockDnsResolver, SsrfError, SystemDnsResolver, validate_and_resolve};
pub use transport::{
    HttpMethod, HttpRequest, HttpResponse, LiveWebTransport, MockWebTransport, WebTransport,
};

/// Default Waist-2 body identifier for `plugin-web-search`.
pub const DEFAULT_BODY_ID: &str = "plugin-web-search";

/// Default capability place owned by `plugin-web-search`.
pub const DEFAULT_PLACE: &str = "cap:web-search";

#[derive(Debug, Deserialize)]
struct WebSearchPayload {
    query: String,
    #[serde(default)]
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct WebFetchPayload {
    url: String,
    #[serde(default)]
    max_chars: Option<usize>,
}

/// Waist-2 capability plugin implementing `web.search` and `web.fetch`.
pub struct WebSearchBody<
    T: WebTransport = LiveWebTransport,
    D: DnsResolver = SystemDnsResolver,
    E: EnvSource = ProcessEnv,
> {
    body_id: BodyId,
    place: PlaceId,
    config: WebSearchConfig,
    transport: T,
    resolver: D,
    env: E,
    held_actions: HashSet<ActionId>,
    thresholds: HonestCueThresholds,
    clock_now_ms: u64,
    shutdown_flag: Option<Arc<AtomicBool>>,
}

impl WebSearchBody<LiveWebTransport, SystemDnsResolver, ProcessEnv> {
    /// Constructs a live `plugin-web-search` body powered by `LiveWebTransport`,
    /// `SystemDnsResolver`, and `ProcessEnv`.
    pub fn with_live(config: WebSearchConfig) -> Result<Self, String> {
        let transport = LiveWebTransport::new(config.timeout_secs)?;
        Ok(Self::new(
            BodyId::new(DEFAULT_BODY_ID),
            PlaceId::new(DEFAULT_PLACE),
            config,
            transport,
            SystemDnsResolver,
            ProcessEnv,
        ))
    }
}

impl<T: WebTransport, D: DnsResolver, E: EnvSource> WebSearchBody<T, D, E> {
    /// Constructs a configurable [`WebSearchBody`] with injectable transport, DNS resolver, and env.
    #[must_use]
    pub fn new(
        body_id: BodyId,
        place: PlaceId,
        config: WebSearchConfig,
        transport: T,
        resolver: D,
        env: E,
    ) -> Self {
        Self {
            body_id,
            place,
            config,
            transport,
            resolver,
            env,
            held_actions: HashSet::new(),
            thresholds: HonestCueThresholds::default(),
            clock_now_ms: 0,
            shutdown_flag: None,
        }
    }

    /// Attaches a shutdown flag flipped when `body/shutdown` arrives.
    #[must_use]
    pub fn with_shutdown_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.shutdown_flag = Some(flag);
        self
    }

    /// Sets the virtual/clock timestamp used for receipts.
    pub fn set_now_ms(&mut self, now_ms: u64) {
        self.clock_now_ms = now_ms;
    }

    /// Returns the active [`WebSearchConfig`].
    #[must_use]
    pub fn config(&self) -> &WebSearchConfig {
        &self.config
    }

    /// Returns the owned [`PlaceId`].
    #[must_use]
    pub fn place(&self) -> &PlaceId {
        &self.place
    }

    /// Borrows the underlying transport.
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    fn capability_descriptors(&self) -> Vec<CapabilityDescriptor> {
        let search_schema = json!({
            "capability": "web.search",
            "input": {
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": { "type": "string" },
                    "max_results": { "type": "integer", "minimum": 1, "maximum": 10 }
                }
            },
            "place": self.place.as_str()
        });
        let fetch_schema = json!({
            "capability": "web.fetch",
            "input": {
                "type": "object",
                "required": ["url"],
                "properties": {
                    "url": { "type": "string" },
                    "max_chars": { "type": "integer", "minimum": 200 }
                }
            },
            "place": self.place.as_str()
        });

        let mut caps = vec![
            CapabilityDescriptor {
                id: CapabilityId::new("web.fetch"),
                description: "Fetch a public http/https web page through the SSRF guard and extract normalized text".to_owned(),
                places: vec![self.place.clone()],
                flags: CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 500,
                capability_schema_hash: capability_schema_hash(&fetch_schema),
                sensed_effect: None,
            },
            CapabilityDescriptor {
                id: CapabilityId::new("web.search"),
                description: "Search the web via configured provider fallback chain (searxng -> tavily -> brave -> duckduckgo)".to_owned(),
                places: vec![self.place.clone()],
                flags: CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 500,
                capability_schema_hash: capability_schema_hash(&search_schema),
                sensed_effect: None,
            },
        ];
        sort_capabilities(&mut caps);
        caps
    }

    fn execute_web_search(&self, params: ActionExecuteParams) -> ActionExecuteResult {
        let payload: WebSearchPayload = match serde_json::from_slice(params.payload.as_bytes()) {
            Ok(p) => p,
            Err(e) => {
                return refused_result(
                    params.action_id,
                    params.capability_id,
                    "ERR_INVALID_PAYLOAD",
                    format!("invalid web.search payload: {e}"),
                    params.h_act,
                    self.clock_now_ms,
                );
            }
        };

        if payload.query.trim().is_empty() {
            return refused_result(
                params.action_id,
                params.capability_id,
                "ERR_INVALID_PAYLOAD",
                "web.search requires a non-empty 'query' field",
                params.h_act,
                self.clock_now_ms,
            );
        }

        if let Some(max_res) = payload.max_results
            && !(1..=config::MAX_SEARCH_RESULTS).contains(&max_res)
        {
            return refused_result(
                params.action_id,
                params.capability_id,
                "ERR_INVALID_PAYLOAD",
                format!(
                    "web.search 'max_results' must be within 1..={}",
                    config::MAX_SEARCH_RESULTS
                ),
                params.h_act,
                self.clock_now_ms,
            );
        }

        match search_web(
            &payload.query,
            payload.max_results,
            &self.config,
            &self.env,
            &self.transport,
        ) {
            Ok(output) => {
                let count = output.results.len() as u64;
                let detail = serde_json::to_string(&output).unwrap_or_else(|_| "{}".to_owned());
                done_result(
                    params.action_id,
                    params.capability_id,
                    count.max(1),
                    detail,
                    params.h_act,
                    self.clock_now_ms,
                )
            }
            Err(err) => failed_result(
                params.action_id,
                params.capability_id,
                "no_provider_answered",
                serde_json::to_string(&err).unwrap_or_else(|_| err.error.clone()),
                params.h_act,
                self.clock_now_ms,
            ),
        }
    }

    fn execute_web_fetch(&self, params: ActionExecuteParams) -> ActionExecuteResult {
        let payload: WebFetchPayload = match serde_json::from_slice(params.payload.as_bytes()) {
            Ok(p) => p,
            Err(e) => {
                return refused_result(
                    params.action_id,
                    params.capability_id,
                    "ERR_INVALID_PAYLOAD",
                    format!("invalid web.fetch payload: {e}"),
                    params.h_act,
                    self.clock_now_ms,
                );
            }
        };

        if payload.url.trim().is_empty() {
            return refused_result(
                params.action_id,
                params.capability_id,
                "ERR_INVALID_PAYLOAD",
                "web.fetch requires a non-empty 'url' field",
                params.h_act,
                self.clock_now_ms,
            );
        }

        if let Some(max_chars) = payload.max_chars
            && max_chars < config::MIN_MAX_CHARS
        {
            return refused_result(
                params.action_id,
                params.capability_id,
                "ERR_INVALID_PAYLOAD",
                format!(
                    "web.fetch 'max_chars' must be at least {}",
                    config::MIN_MAX_CHARS
                ),
                params.h_act,
                self.clock_now_ms,
            );
        }

        match fetch_page(
            &payload.url,
            payload.max_chars,
            &self.config,
            &self.transport,
            &self.resolver,
        ) {
            Ok(output) => {
                let detail = serde_json::to_string(&output).unwrap_or_else(|_| "{}".to_owned());
                done_result(
                    params.action_id,
                    params.capability_id,
                    1,
                    detail,
                    params.h_act,
                    self.clock_now_ms,
                )
            }
            Err(err) => match err.kind {
                FetchErrorKind::Blocked => refused_result(
                    params.action_id,
                    params.capability_id,
                    &err.reason_code,
                    err.message,
                    params.h_act,
                    self.clock_now_ms,
                ),
                FetchErrorKind::Upstream => failed_result(
                    params.action_id,
                    params.capability_id,
                    &err.reason_code,
                    json!({ "error": err.message, "reason": err.reason_code }).to_string(),
                    params.h_act,
                    self.clock_now_ms,
                ),
            },
        }
    }
}

impl<T: WebTransport, D: DnsResolver, E: EnvSource> BodyHandler for WebSearchBody<T, D, E> {
    fn discover(&mut self, params: BodyDiscoverParams) -> Result<BodyDiscoverResult, Waist2Error> {
        let capabilities = self.capability_descriptors();
        let places = vec![self.place.clone()];
        Ok(BodyDiscoverResult {
            body_id: params.body_id.unwrap_or_else(|| self.body_id.clone()),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            capabilities,
            places,
        })
    }

    fn execute(&mut self, params: ActionExecuteParams) -> Result<ActionExecuteResult, Waist2Error> {
        if params.audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }

        let cap = params.capability_id.as_str();
        if cap != "web.search" && cap != "web.fetch" {
            return Err(Waist2Error::unknown_capability(&params.capability_id));
        }

        let place_ok = !params.places.is_empty() && params.places.iter().all(|p| p == &self.place);
        if !place_ok {
            return Ok(refused_result(
                params.action_id,
                params.capability_id,
                "ERR_UNKNOWN_PLACE",
                "action targets a place not owned by plugin-web-search",
                params.h_act,
                self.clock_now_ms,
            ));
        }

        if params.h_act.is_some() && params.payload.as_bytes() == b"hold" {
            self.held_actions.insert(params.action_id.clone());
            let receipt = make_terminal_receipt(
                params.action_id,
                params.capability_id,
                ReceiptOutcome::Held,
                Some("PendingApproval".to_owned()),
                Some("awaiting owner approval before web operation".to_owned()),
                Some(TerminationReason::Normal),
                Some(0.0),
                Some(0),
                params.h_act,
                Time::from_millis(self.clock_now_ms),
            );
            return Ok(ActionExecuteResult {
                accepted: true,
                receipt: Some(receipt),
            });
        }

        let is_delivery_probe = params.payload.as_bytes() == b"delivery-timing-probe"
            || params.headers.meta.get("org.monoagens.delivery_probe")
                == Some(&MetaValue::Bool(true));
        if is_delivery_probe {
            let detail = json!({
                "d_art_ms": self.thresholds.d_art_ms,
                "working_cue_ms": self.thresholds.working_cue_ms,
                "progress_note_ms": self.thresholds.progress_note_ms,
                "probe": true,
            })
            .to_string();
            return Ok(done_result(
                params.action_id,
                params.capability_id,
                1,
                detail,
                params.h_act,
                self.clock_now_ms,
            ));
        }

        if matches!(
            params.payload.as_bytes(),
            b"idempotency-probe" | b"crash-recovery-probe" | b"owned-place-action"
        ) {
            return Ok(done_result(
                params.action_id,
                params.capability_id,
                1,
                json!({ "probe": true }).to_string(),
                params.h_act,
                self.clock_now_ms,
            ));
        }

        match cap {
            "web.search" => Ok(self.execute_web_search(params)),
            "web.fetch" => Ok(self.execute_web_fetch(params)),
            _ => Err(Waist2Error::unknown_capability(&params.capability_id)),
        }
    }

    fn cancel(&mut self, params: ActionCancelParams) -> Result<ActionCancelResult, Waist2Error> {
        let receipt = make_terminal_receipt(
            params.action_id,
            CapabilityId::new("web.search"),
            ReceiptOutcome::Failed,
            Some("canceled_by_core".to_owned()),
            params.reason,
            Some(TerminationReason::CanceledByCore),
            Some(0.0),
            Some(0),
            None,
            Time::from_millis(self.clock_now_ms),
        );
        Ok(ActionCancelResult {
            cancelled: true,
            receipt: Some(receipt),
        })
    }

    fn resolve_approval(
        &mut self,
        params: ApprovalResolveParams,
    ) -> Result<ApprovalResolveResult, Waist2Error> {
        if self.held_actions.remove(&params.action_id) {
            return Ok(ApprovalResolveResult { applied: true });
        }
        Ok(ApprovalResolveResult { applied: false })
    }

    fn on_percept(&mut self, params: PerceptEmitParams) -> Result<PerceptEmitResult, Waist2Error> {
        if params.audience.as_ref().is_none_or(Audience::is_empty) {
            return Err(Waist2Error::undeclared_audience());
        }
        if params.place != self.place {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "percept place '{}' is not owned by plugin-web-search",
                params.place
            )));
        }
        Ok(PerceptEmitResult { accepted: true })
    }

    fn on_shutdown(
        &mut self,
        _params: BodyShutdownParams,
    ) -> Result<BodyShutdownResult, Waist2Error> {
        if let Some(ref flag) = self.shutdown_flag {
            flag.store(true, Ordering::SeqCst);
        }
        Ok(BodyShutdownResult { accepted: true })
    }
}

fn refused_result(
    action_id: ActionId,
    capability_id: CapabilityId,
    reason_code: &str,
    detail: impl Into<String>,
    h_act: Option<monoagens_body_sdk::HAct>,
    now_ms: u64,
) -> ActionExecuteResult {
    ActionExecuteResult {
        accepted: false,
        receipt: Some(make_refused_receipt(
            action_id,
            capability_id,
            reason_code,
            detail,
            h_act,
            Time::from_millis(now_ms),
        )),
    }
}

fn done_result(
    action_id: ActionId,
    capability_id: CapabilityId,
    delivered_units: u64,
    detail: String,
    h_act: Option<monoagens_body_sdk::HAct>,
    now_ms: u64,
) -> ActionExecuteResult {
    ActionExecuteResult {
        accepted: true,
        receipt: Some(make_terminal_receipt(
            action_id,
            capability_id,
            ReceiptOutcome::Done,
            None,
            Some(detail),
            Some(TerminationReason::Normal),
            Some(1.0),
            Some(delivered_units),
            h_act,
            Time::from_millis(now_ms),
        )),
    }
}

fn failed_result(
    action_id: ActionId,
    capability_id: CapabilityId,
    reason_code: &str,
    detail: String,
    h_act: Option<monoagens_body_sdk::HAct>,
    now_ms: u64,
) -> ActionExecuteResult {
    ActionExecuteResult {
        accepted: true,
        receipt: Some(make_terminal_receipt(
            action_id,
            capability_id,
            ReceiptOutcome::Failed,
            Some(reason_code.to_owned()),
            Some(detail),
            Some(TerminationReason::Normal),
            Some(0.0),
            Some(0),
            h_act,
            Time::from_millis(now_ms),
        )),
    }
}
