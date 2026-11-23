use std::collections::BTreeSet;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use body_cli::{CliBody, TerminalOutputSink};
use monoagens_body_kit::{EmittedCue, PersistentBodyRuntime};
use monoagens_core::{
    ContextPlan, DecisionOutcome, DecisionReason, DecisionTrace, DoorDecision, Doors, EventDetail,
    EventKind, EventRecord, Input, LogSeq,
};
use monoagens_gateway::{
    Clock, DoorController, GatewayHandle, LifecycleCueEvent, LifecycleCueKind, ValveDoorController,
};
use monoagens_protocol::{
    Action, ActionId, ActorId, ApprovalResolveParams, ApprovalResolveResult, Audience, BodyId,
    CapabilityDescriptor, OriginActor, Percept, PlaceId, Receipt, Taint, Time, Waist2Error,
};
use monoagens_reason::ModelProvider;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};

use crate::ChatArgs;
use crate::common::{
    BootGatewayParams, BootedGateway, boot_gateway_runtime, load_ready_gateway_env,
    owner_actor_from_cfg, write_json_pretty,
};

struct SharedChatSink {
    rendered: Arc<Mutex<Vec<(String, String)>>>,
}

impl TerminalOutputSink for SharedChatSink {
    fn render_bubble(&mut self, pid: &str, text: &str) -> Result<(), String> {
        self.rendered
            .lock()
            .expect("rendered lock")
            .push((pid.to_owned(), text.to_owned()));
        Ok(())
    }

    fn render_cue(&mut self, _cue: &EmittedCue) -> Result<(), String> {
        Ok(())
    }
}

/// Executes `monoagens chat` with an optional injected [`ModelProvider`] (used by tests to drive
/// `body-cli` + `Valve` + `GatewayRuntime` deterministically without network).
pub fn run_chat_with_provider(
    config_path: Option<&Path>,
    json_mode: bool,
    args: ChatArgs,
    provider_override: Option<Arc<dyn ModelProvider>>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8, String> {
    cmd_chat(config_path, json_mode, args, provider_override, out, err)
}

/// Interactive CLI chat session state for room/group/speaker simulation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplSessionState {
    pub owner_actor: ActorId,
    pub current_actor: ActorId,
    pub place: PlaceId,
    pub transport_place: PlaceId,
    pub audience: Audience,
    pub is_group: bool,
    pub show_brain: bool,
}

impl ReplSessionState {
    #[must_use]
    pub fn new(owner_actor: ActorId, place: PlaceId, show_brain: bool) -> Self {
        let audience = Audience::new([owner_actor.clone(), ActorId::new("soul")]);
        Self {
            current_actor: owner_actor.clone(),
            owner_actor,
            transport_place: place.clone(),
            place,
            audience,
            is_group: false,
            show_brain,
        }
    }

    #[must_use]
    pub fn prompt_label(&self) -> String {
        let mode = if self.is_group { "grup" } else { "dm" };
        format!(
            "[{mode}:{} | {}]> ",
            self.place.as_str(),
            self.current_actor.as_str()
        )
    }
}

/// Result of parsing a single REPL input line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplAction {
    Message(String),
    Notice(String),
    Quit,
}

const REPL_HELP_TEXT: &str = "\
Perintah chat interaktif MonoAgens:
  /dm                         Pindah ke DM pribadi dengan Mono (otomatis menyapa Mono)
  /room <nama_ruang>          Pindah ruang DM (mis. /room dm:catatan)
  /group <ruang> <a,b,c>      Masuk grup simulasi (mis. /group room:ta owner,budi,sari)
                              Di grup, Mono hanya otomatis disapa jika kamu menyebut @mono
  /as <nama_pengirim>         Ganti pengirim pesan (mis. /as budi atau /as owner)
  /brain                      Tampilkan / sembunyikan status otak [otak: ...]
  /status                     Lihat ruangan, pengirim, audiens, & status otak saat ini
  /help                       Tampilkan bantuan ini
  /quit atau /exit            Keluar dari chat";

/// Parses a single user input line, mutating `state` for slash commands.
#[must_use]
pub fn parse_repl_command(raw: &str, state: &mut ReplSessionState) -> ReplAction {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ReplAction::Notice(String::new());
    }
    if !trimmed.starts_with('/') {
        return ReplAction::Message(trimmed.to_owned());
    }

    let mut parts = trimmed.split_whitespace();
    let cmd = parts.next().unwrap_or("");
    match cmd {
        "/quit" | "/exit" | "/q" => ReplAction::Quit,
        "/help" | "/h" => ReplAction::Notice(REPL_HELP_TEXT.to_owned()),
        "/brain" => {
            state.show_brain = !state.show_brain;
            ReplAction::Notice(format!(
                "Status otak [otak: ...]: {}",
                if state.show_brain { "AKTIF" } else { "MATI" }
            ))
        }
        "/dm" => {
            state.place = state.transport_place.clone();
            state.current_actor = state.owner_actor.clone();
            state.audience = Audience::new([state.owner_actor.clone(), ActorId::new("soul")]);
            state.is_group = false;
            ReplAction::Notice(format!("Pindah ke DM pribadi ({}).", state.place.as_str()))
        }
        "/room" => {
            let Some(room) = parts.next() else {
                return ReplAction::Notice("Pakai: /room <nama_ruang>".to_owned());
            };
            state.place = PlaceId::new(room);
            state.audience = Audience::new([state.current_actor.clone(), ActorId::new("soul")]);
            state.is_group = false;
            ReplAction::Notice(format!("Pindah ke ruang DM '{}'.", state.place.as_str()))
        }
        "/group" => {
            let tokens: Vec<&str> = parts.collect();
            if tokens.is_empty() {
                return ReplAction::Notice(
                    "Pakai: /group <nama_ruang> <anggota1,anggota2,...>".to_owned(),
                );
            }
            let (room_name, members_raw) =
                if tokens.len() >= 2 && tokens.last().is_some_and(|t| t.contains(',')) {
                    (
                        tokens[..tokens.len() - 1].join("_"),
                        *tokens.last().expect("checked"),
                    )
                } else if tokens.len() >= 2 {
                    (tokens[0].to_owned(), tokens[1])
                } else {
                    (tokens[0].to_owned(), "")
                };
            let mut members: Vec<ActorId> = members_raw
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|m| {
                    if m.eq_ignore_ascii_case("owner") {
                        state.owner_actor.clone()
                    } else {
                        ActorId::new(m)
                    }
                })
                .collect();
            if members.is_empty() {
                members.push(state.owner_actor.clone());
                members.push(ActorId::new("budi"));
                members.push(ActorId::new("sari"));
            }
            if !members.contains(&state.current_actor) {
                members.push(state.current_actor.clone());
            }
            members.push(ActorId::new("soul"));
            state.place = PlaceId::new(room_name);
            state.audience = Audience::new(members);
            state.is_group = true;
            let names: Vec<&str> = state.audience.members.iter().map(ActorId::as_str).collect();
            ReplAction::Notice(format!(
                "Masuk grup '{}' (audiens: [{}]). Sebut @mono untuk menyapa Mono langsung.",
                state.place.as_str(),
                names.join(", ")
            ))
        }
        "/as" => {
            let Some(who) = parts.next() else {
                return ReplAction::Notice("Pakai: /as <nama_pengirim>".to_owned());
            };
            state.current_actor = if who.eq_ignore_ascii_case("owner") {
                state.owner_actor.clone()
            } else {
                ActorId::new(who)
            };
            if !state.audience.members.contains(&state.current_actor) {
                let mut members = state.audience.members.clone();
                members.insert(state.current_actor.clone());
                state.audience = Audience::new(members);
            }
            ReplAction::Notice(format!(
                "Sekarang berbicara sebagai '{}'.",
                state.current_actor.as_str()
            ))
        }
        "/status" => {
            let names: Vec<&str> = state.audience.members.iter().map(ActorId::as_str).collect();
            ReplAction::Notice(format!(
                "ruang={} | mode={} | pengirim={} | audiens=[{}] | otak={}",
                state.place.as_str(),
                if state.is_group { "grup" } else { "dm" },
                state.current_actor.as_str(),
                names.join(", "),
                if state.show_brain { "aktif" } else { "mati" },
            ))
        }
        other => ReplAction::Notice(format!(
            "Perintah tidak dikenal '{other}'. Ketik /help untuk daftar perintah."
        )),
    }
}

/// Constructs a canonical Waist-2 percept from the active REPL session state.
///
/// In DM mode, messages are automatically addressed to `soul`. In group mode, `soul` is added
/// to `addressees`/`named` only when the text explicitly mentions `@mono` or `@soul`
/// (case-insensitive), allowing natural testing of `Silent` / `Wait` / `Go` in group rooms.
pub fn build_repl_percept(
    state: &ReplSessionState,
    text: &str,
    now: Time,
    turn_idx: usize,
) -> Result<Percept, String> {
    let helper_body = CliBody::new(
        BodyId::new("body-cli"),
        state.transport_place.clone(),
        state.current_actor.clone(),
        4096,
        SharedChatSink {
            rendered: Arc::new(Mutex::new(Vec::new())),
        },
    );
    let (percept_params, _) =
        helper_body.build_user_percept(text, None, None, now, format!("cli-chat-{turn_idx}"));
    let mut percept = percept_params
        .into_percept(monoagens_protocol::CURRENT_PROTOCOL_VERSION)
        .map_err(|e| e.to_string())?;
    percept.place = state.transport_place.clone();
    percept.actor = state.current_actor.clone();
    percept.origin_actor = Some(OriginActor::new(state.current_actor.clone()));
    percept.audience = state.audience.clone();
    percept.taint = if state.current_actor == state.owner_actor {
        Taint::Trusted
    } else {
        Taint::Untrusted
    };

    let lower = text.to_ascii_lowercase();
    let mentions_soul = lower.contains("@mono") || lower.contains("@soul");
    if state.is_group {
        percept.addressees.clear();
        percept.named.clear();
        if mentions_soul {
            percept.addressees.push(ActorId::new("soul"));
            percept.named.push(ActorId::new("soul"));
        }
    } else if percept.addressees.is_empty() {
        percept.addressees.push(ActorId::new("soul"));
    }
    Ok(percept)
}

/// Formats a compact human-readable one-line summary of a turn's [`DecisionTrace`].
#[must_use]
pub fn format_decision_trace_summary(trace: &DecisionTrace) -> String {
    let outcome = match trace.outcome {
        DecisionOutcome::Go => "Go",
        DecisionOutcome::Silent => "Silent",
        DecisionOutcome::Note => "Note",
        DecisionOutcome::DoorBlocked => "DoorBlocked",
        DecisionOutcome::Processed => "Processed",
    };
    let tier = trace.neural.compute_tier.map_or("—", |t| match t {
        monoagens_core::ComputeTier::Fast => "Fast",
        monoagens_core::ComputeTier::Standard => "Standard",
        monoagens_core::ComputeTier::Deep => "Deep",
    });
    let mut summary = format!(
        "[otak: {outcome} | reason={:?} | tier={tier} | p_go={:.2} | p_wait={:.2}",
        trace.reason, trace.neural.gate_probs[3], trace.neural.gate_probs[2]
    );
    if trace.neural.wait_delta_ms > 0.0 && trace.reason == DecisionReason::WaitDeferral {
        summary.push_str(&format!(" ({:.0}ms)", trace.neural.wait_delta_ms));
    }
    if let Some(p_corr) = trace.neural.p_user_correction {
        summary.push_str(&format!(" | p_corr={p_corr:.2}"));
    }
    if trace.neural.is_neutral_fallback {
        summary.push_str(" | neutral");
    }
    summary.push(']');
    summary
}

fn find_latest_decision_since(
    events: &[EventRecord],
    since_len: usize,
) -> Option<(DecisionOutcome, Option<DecisionTrace>)> {
    events.get(since_len..)?.iter().rev().find_map(|ev| {
        if let Some(EventDetail::Decision { outcome, trace, .. }) = &ev.detail {
            Some((*outcome, trace.as_deref().cloned()))
        } else {
            None
        }
    })
}

struct CliInteractiveDoorController {
    inner: ValveDoorController,
    dynamic_actors: Arc<Mutex<BTreeSet<ActorId>>>,
}

impl CliInteractiveDoorController {
    fn sync_dynamic_actors(&mut self) {
        let guard = self.dynamic_actors.lock().expect("dynamic actors lock");
        let known = &mut self.inner.valve_mut().snapshot_mut().known_actors;
        for actor in guard.iter() {
            known.insert(actor.clone());
        }
    }
}

impl Doors for CliInteractiveDoorController {
    fn inbound(&mut self, percept: &mut Percept) -> DoorDecision {
        self.sync_dynamic_actors();
        self.inner.inbound(percept)
    }

    fn context(&mut self, plan: &mut ContextPlan) -> DoorDecision {
        self.sync_dynamic_actors();
        self.inner.context(plan)
    }

    fn action(&mut self, action: &mut Action) -> DoorDecision {
        self.inner.action(action)
    }
}

impl DoorController for CliInteractiveDoorController {
    fn reload_valve(&mut self, config: &monoagens_gateway::GatewayConfig) -> bool {
        self.inner.reload_valve(config)
    }

    fn sync_runtime_state(
        &mut self,
        log_seq: LogSeq,
        now: Time,
        daily_tokens_used: u64,
        capabilities: &[CapabilityDescriptor],
    ) {
        self.inner
            .sync_runtime_state(log_seq, now, daily_tokens_used, capabilities);
    }

    fn sync_state_epoch(&mut self, state_epoch: u64) {
        self.inner.sync_state_epoch(state_epoch);
    }

    fn replay_from_store_events(&mut self, events: &[EventRecord]) {
        self.inner.replay_from_store_events(events);
    }

    fn sync_item_audiences(&mut self, events: &[EventRecord]) {
        self.inner.sync_item_audiences(events);
    }

    fn resolve_approval(
        &mut self,
        params: &ApprovalResolveParams,
    ) -> Option<Result<ApprovalResolveResult, Waist2Error>> {
        self.inner.resolve_approval(params)
    }

    fn record_in_band_rejection_audit(&mut self, params: &ApprovalResolveParams, detail: &str) {
        self.inner.record_in_band_rejection_audit(params, detail);
    }

    fn take_pending_approval_events(&mut self) -> Vec<EventRecord> {
        self.inner.take_pending_approval_events()
    }

    fn take_approved_action(&mut self, action_id: &ActionId) -> Option<Action> {
        self.inner.take_approved_action(action_id)
    }

    fn take_pending_denial_receipt(&mut self, action_id: &ActionId) -> Option<Receipt> {
        self.inner.take_pending_denial_receipt(action_id)
    }
}

fn register_session_actors(
    dynamic_actors: &Arc<Mutex<BTreeSet<ActorId>>>,
    session: &ReplSessionState,
) {
    let mut guard = dynamic_actors.lock().expect("dynamic actors lock");
    if session.current_actor.as_str() != "soul" {
        guard.insert(session.current_actor.clone());
    }
    for member in &session.audience.members {
        if member.as_str() != "soul" {
            guard.insert(member.clone());
        }
    }
}

async fn enrich_and_submit_percept(
    handle: &GatewayHandle,
    mut percept: Percept,
) -> Result<monoagens_gateway::TurnOutcome, monoagens_gateway::GatewayError> {
    if let Some(pq) = handle.pre_queue_service() {
        let _ = pq.enrich_percept_async(&mut percept).await;
    }
    handle.submit_and_wait(Input::Percept(Box::new(percept))).await
}

#[allow(clippy::too_many_arguments)]
fn drain_async_chat_outputs(
    cue_rx: &mut tokio::sync::mpsc::UnboundedReceiver<LifecycleCueEvent>,
    rendered_bubbles: &Arc<Mutex<Vec<(String, String)>>>,
    rendered_cursor: &mut usize,
    emitted_outputs: &mut Vec<String>,
    out: &mut dyn Write,
    err: &mut dyn Write,
    leading_newline: bool,
) -> bool {
    let mut printed_any = false;
    while let Ok(cue) = cue_rx.try_recv() {
        let cue_label = match cue.kind {
            LifecycleCueKind::WorkingCue => "working_cue",
            LifecycleCueKind::ProgressNote => "progress_note",
        };
        if leading_newline && !printed_any {
            let _ = writeln!(err);
        }
        let _ = writeln!(err, "[{cue_label}] {}", cue.detail);
        let _ = err.flush();
        printed_any = true;
    }
    let new_items = {
        let guard = rendered_bubbles.lock().expect("rendered lock");
        if guard.len() > *rendered_cursor {
            let slice = guard[*rendered_cursor..].to_vec();
            *rendered_cursor = guard.len();
            slice
        } else {
            Vec::new()
        }
    };
    if !new_items.is_empty() {
        for (_pid, text) in new_items {
            if leading_newline && !printed_any {
                let _ = writeln!(out);
            }
            let _ = writeln!(out, "Mono> {text}");
            let _ = out.flush();
            emitted_outputs.push(text);
            printed_any = true;
        }
    }
    printed_any
}

pub(crate) fn cmd_chat(
    config_path: Option<&Path>,
    json_mode: bool,
    args: ChatArgs,
    provider_override: Option<Arc<dyn ModelProvider>>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8, String> {
    let Some(loaded) = load_ready_gateway_env(config_path, provider_override.is_some(), err)?
    else {
        return Ok(1);
    };

    let owner_actor = owner_actor_from_cfg(Some(&loaded.config));
    let known_actors: Vec<ActorId> = args
        .actors
        .iter()
        .map(|a| a.trim())
        .filter(|a| !a.is_empty())
        .map(ActorId::new)
        .collect();

    let dynamic_actors = Arc::new(Mutex::new(BTreeSet::new()));
    {
        let mut guard = dynamic_actors.lock().expect("dynamic actors lock");
        guard.insert(owner_actor);
        for actor in &known_actors {
            guard.insert(actor.clone());
        }
    }

    let start_place = PlaceId::new(args.place.as_deref().unwrap_or("cli:terminal"));
    let is_interactive =
        args.interactive || (args.messages.is_empty() && !json_mode && io::stdin().is_terminal());
    let show_brain = args.show_brain || is_interactive;
    let max_wait_polls: usize = if provider_override.is_some() {
        100
    } else {
        12_000
    }; // 500ms scripted vs 60s live LLM

    let batch_inputs: Vec<String> = if is_interactive {
        Vec::new()
    } else if !args.messages.is_empty() {
        args.messages
    } else {
        let stdin = io::stdin();
        stdin
            .lock()
            .lines()
            .map_while(Result::ok)
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            .collect()
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to start tokio runtime: {e}"))?;

    let rendered_bubbles = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let rendered_clone = Arc::clone(&rendered_bubbles);

    let result: Result<(Vec<String>, Vec<EventRecord>), String> = rt.block_on(async {
        let dynamic_actors_for_doors = Arc::clone(&dynamic_actors);
        let BootedGateway {
            runtime,
            owner_actor,
            cfg,
            clock,
            cues,
            _embed_sidecar,
            ..
        } = boot_gateway_runtime(
            &loaded,
            BootGatewayParams {
                data_dir: args.data_dir,
                model: args.model,
                known_actors,
                provider_override,
                json_mode,
            },
            move |inner| CliInteractiveDoorController {
                inner,
                dynamic_actors: dynamic_actors_for_doors,
            },
            err,
        )
        .await?;

        let mut cue_rx = cues.subscribe();
        let neural_model_label = cfg.soul.model_path.as_ref().map_or_else(
            || "neutral (tanpa model)".to_owned(),
            |p| p.display().to_string(),
        );
        let handle = runtime.handle();

        // Spawn in-process body-cli speaking canonical Waist-2 NDJSON over duplex pipe.
        let (gw_stream, body_stream) = tokio::io::duplex(64 * 1024);
        let (gw_read, gw_write) = tokio::io::split(gw_stream);
        let (body_read, mut body_write) = tokio::io::split(body_stream);

        let body_actor = owner_actor.clone();
        let body_place = start_place.clone();
        let body_handle = tokio::spawn(async move {
            let mut body_rt = PersistentBodyRuntime::new(None);
            let mut cli_body = CliBody::new(
                BodyId::new("body-cli"),
                body_place,
                body_actor,
                4096,
                SharedChatSink {
                    rendered: rendered_clone,
                },
            );
            let mut reader = TokioBufReader::new(body_read);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if let Some(reply) = body_rt.dispatch_line(&line, &mut cli_body)
                            && (body_write.write_all(reply.as_bytes()).await.is_err()
                                || body_write.write_all(b"\n").await.is_err()
                                || body_write.flush().await.is_err())
                        {
                            break;
                        }
                    }
                }
            }
        });

        handle
            .connect_ndjson_body(gw_read, gw_write, Some(BodyId::new("body-cli")))
            .await
            .map_err(|e| format!("failed to register body-cli over Waist 2: {e}"))?;

        let mut session = ReplSessionState::new(owner_actor, start_place, show_brain);
        let mut emitted_outputs = Vec::new();
        let mut rendered_cursor = 0usize;

        if is_interactive {
            let _ = writeln!(out, "=== MonoAgens Interactive Chat (V2.2) ===");
            let _ = writeln!(out, "Otak Sistem 1 : {neural_model_label}");
            let _ = writeln!(
                out,
                "Ketik pesan lalu Enter. Ketik /help untuk perintah ruangan/grup, /quit untuk keluar.\n"
            );
            let _ = out.flush();

            let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<io::Result<String>>();
            std::thread::spawn(move || {
                let stdin = io::stdin();
                let mut locked = stdin.lock();
                loop {
                    let mut buf = String::new();
                    match locked.read_line(&mut buf) {
                        Ok(0) => break,
                        Ok(_) => {
                            if line_tx.send(Ok(buf)).is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            let _ = line_tx.send(Err(e));
                            break;
                        }
                    }
                }
            });

            let mut turn_idx = 0usize;
            let mut async_tick = tokio::time::interval(Duration::from_millis(50));
            async_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            'repl: loop {
                let _ = write!(out, "{}", session.prompt_label());
                let _ = out.flush();

                let raw = loop {
                    tokio::select! {
                        maybe_line = line_rx.recv() => {
                            match maybe_line {
                                Some(Ok(line)) => break line,
                                Some(Err(e)) => return Err(format!("stdin read error: {e}")),
                                None => break 'repl,
                            }
                        }
                        _ = async_tick.tick() => {
                            if drain_async_chat_outputs(
                                &mut cue_rx,
                                &rendered_bubbles,
                                &mut rendered_cursor,
                                &mut emitted_outputs,
                                out,
                                err,
                                true,
                            ) {
                                let _ = write!(out, "{}", session.prompt_label());
                                let _ = out.flush();
                            }
                        }
                    }
                };

                match parse_repl_command(&raw, &mut session) {
                    ReplAction::Quit => break,
                    ReplAction::Notice(msg) => {
                        register_session_actors(&dynamic_actors, &session);
                        if !msg.is_empty() {
                            let _ = writeln!(out, "{msg}");
                            let _ = out.flush();
                        }
                        continue;
                    }
                    ReplAction::Message(msg) => {
                        register_session_actors(&dynamic_actors, &session);
                        let before_events = handle.read_events().await.unwrap_or_default().len();
                        let now = clock.now();
                        let percept = build_repl_percept(&session, &msg, now, turn_idx)?;
                        turn_idx += 1;

                        enrich_and_submit_percept(&handle, percept)
                            .await
                            .map_err(|e| format!("chat turn failed: {e}"))?;

                        let post_events = handle.read_events().await.unwrap_or_default();
                        let decision = find_latest_decision_since(&post_events, before_events);
                        if session.show_brain
                            && let Some((outcome, ref tr_opt)) = decision
                        {
                            if let Some(tr) = tr_opt {
                                let _ = writeln!(err, "{}", format_decision_trace_summary(tr));
                            } else {
                                let _ =
                                    writeln!(err, "[otak: {outcome:?} | ditahan Valve inbound]");
                            }
                            let _ = err.flush();
                        }

                        let wait_armed = decision.as_ref().is_some_and(|(_, tr_opt)| {
                            tr_opt
                                .as_ref()
                                .is_some_and(|tr| tr.reason == DecisionReason::WaitDeferral)
                        });
                        if session.show_brain && wait_armed {
                            let _ = writeln!(
                                err,
                                "[otak] menunggu sebentar; Mono akan bantu otomatis jika belum ada yang menjawab."
                            );
                            let _ = err.flush();
                        }

                        let expects_reply = decision
                            .as_ref()
                            .is_none_or(|(outcome, _)| *outcome == DecisionOutcome::Go);
                        if expects_reply {
                            let mut got_reply = false;
                            let mut failure_msg: Option<String> = None;
                            for poll in 0..max_wait_polls {
                                let prev_out_len = emitted_outputs.len();
                                let _ = drain_async_chat_outputs(
                                    &mut cue_rx,
                                    &rendered_bubbles,
                                    &mut rendered_cursor,
                                    &mut emitted_outputs,
                                    out,
                                    err,
                                    false,
                                );
                                if emitted_outputs.len() > prev_out_len {
                                    got_reply = true;
                                    break;
                                }
                                if poll > 0 && poll % 40 == 0 {
                                    let cur_events = handle.read_events().await.unwrap_or_default();
                                    if let Some(slice) = cur_events.get(before_events..) {
                                        let mut has_action = false;
                                        let mut thought_text: Option<String> = None;
                                        let mut thought_processed = false;
                                        for ev in slice {
                                            if let Some(EventDetail::Failure {
                                                component,
                                                detail,
                                                ..
                                            }) = &ev.detail
                                            {
                                                failure_msg =
                                                    Some(format!("[error {component}] {detail}"));
                                                break;
                                            }
                                            match ev.kind {
                                                EventKind::Action => has_action = true,
                                                EventKind::Thought => {
                                                    let t = String::from_utf8_lossy(
                                                        ev.payload.as_bytes(),
                                                    )
                                                    .trim()
                                                    .to_owned();
                                                    if !t.is_empty() {
                                                        thought_text = Some(t);
                                                    }
                                                }
                                                EventKind::Decision => {
                                                    if let Some(EventDetail::Decision {
                                                        outcome: DecisionOutcome::Processed,
                                                        ..
                                                    }) = &ev.detail
                                                    {
                                                        thought_processed = true;
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                        if failure_msg.is_some() {
                                            break;
                                        }
                                        if thought_processed && !has_action {
                                            if let Some(t) = thought_text {
                                                let _ = writeln!(
                                                    err,
                                                    "[pikiran Mono (tanpa kirim pesan)] {t}"
                                                );
                                            } else {
                                                let _ = writeln!(
                                                    err,
                                                    "[otak] Mono selesai berpikir dan memilih tidak mengirim pesan."
                                                );
                                            }
                                            let _ = err.flush();
                                            got_reply = true;
                                            break;
                                        }
                                    }
                                }
                                tokio::time::sleep(Duration::from_millis(5)).await;
                            }
                            if let Some(msg) = failure_msg {
                                let _ = writeln!(err, "{msg}");
                                let _ = err.flush();
                            } else if !got_reply {
                                let _ = writeln!(
                                    err,
                                    "[info] belum ada balasan dalam batas waktu (cek log/endpoint LLM)."
                                );
                                let _ = err.flush();
                            }
                        }
                    }
                }
            }
        } else {
            for (idx, msg) in batch_inputs.iter().enumerate() {
                let before_events = handle.read_events().await.unwrap_or_default().len();
                let now = clock.now();
                let percept = build_repl_percept(&session, msg, now, idx)?;
                enrich_and_submit_percept(&handle, percept)
                    .await
                    .map_err(|e| format!("chat turn failed: {e}"))?;

                let post_events = handle.read_events().await.unwrap_or_default();
                let decision = find_latest_decision_since(&post_events, before_events);
                if session.show_brain
                    && !json_mode
                    && let Some((outcome, ref tr_opt)) = decision
                {
                    if let Some(tr) = tr_opt {
                        let _ = writeln!(err, "{}", format_decision_trace_summary(tr));
                    } else {
                        let _ = writeln!(err, "[otak: {outcome:?}]");
                    }
                }

                let expects_reply = decision
                    .as_ref()
                    .is_none_or(|(outcome, _)| *outcome == DecisionOutcome::Go);
                if !expects_reply {
                    continue;
                }

                // Drain any asynchronous model/action turn briefly without blocking fast responses (d_art = 0 ms).
                for _ in 0..max_wait_polls {
                    while let Ok(cue) = cue_rx.try_recv() {
                        let cue_label = match cue.kind {
                            LifecycleCueKind::WorkingCue => "working_cue",
                            LifecycleCueKind::ProgressNote => "progress_note",
                        };
                        if !json_mode {
                            let _ = writeln!(err, "[{cue_label}] {}", cue.detail);
                        }
                    }
                    let new_items = {
                        let guard = rendered_bubbles.lock().expect("rendered lock");
                        if guard.len() > rendered_cursor {
                            let slice = guard[rendered_cursor..].to_vec();
                            rendered_cursor = guard.len();
                            slice
                        } else {
                            Vec::new()
                        }
                    };
                    if !new_items.is_empty() {
                        for (_pid, text) in new_items {
                            emitted_outputs.push(text);
                        }
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
        }

        let events = handle.read_events().await.unwrap_or_default();
        runtime.shutdown().await;
        body_handle.abort();
        Ok((emitted_outputs, events))
    });

    let (outputs, events) = result?;
    if json_mode {
        let payload = json!({
            "status": "ok",
            "responses": outputs,
            "events_recorded": events.len(),
        });
        write_json_pretty(out, &payload)?;
    } else if !is_interactive {
        for text in outputs {
            writeln!(out, "{text}").map_err(|e| e.to_string())?;
        }
    }
    Ok(0)
}

#[cfg(test)]
mod repl_tests {
    use super::*;
    use monoagens_core::{ComputeTier, DecisionReason, GateClass, NeuralOutputs};
    use monoagens_gateway::{DiagnosticSink, PreQueueService, SystemClock};
    use monoagens_protocol::Time;
    use monoagens_reason::{HashEmbedder, ProviderSecrets};
    use monoagens_valve::{Valve, ValveConfig, ValveSnapshot};

    #[test]
    fn repl_commands_switch_group_speaker_dm_and_brain_toggle() {
        let mut st = ReplSessionState::new(
            ActorId::new("per_owner"),
            PlaceId::new("cli:terminal"),
            true,
        );
        assert!(!st.is_group);
        assert!(st.show_brain);

        let a1 = parse_repl_command("/group room:ta owner,budi,sari", &mut st);
        assert!(matches!(a1, ReplAction::Notice(ref s) if s.contains("room:ta")));
        assert!(st.is_group);
        assert_eq!(st.place.as_str(), "room:ta");
        assert!(st.audience.members.contains(&ActorId::new("budi")));
        assert!(st.audience.members.contains(&ActorId::new("soul")));

        let a2 = parse_repl_command("/as budi", &mut st);
        assert!(matches!(a2, ReplAction::Notice(ref s) if s.contains("budi")));
        assert_eq!(st.current_actor.as_str(), "budi");

        // Casual group chatter does NOT auto-address soul, while keeping deliverable channel place
        let p_casual =
            build_repl_percept(&st, "besok bimbingan jam berapa?", Time::from_millis(1000), 0)
                .expect("percept");
        assert_eq!(p_casual.place.as_str(), "cli:terminal");
        assert!(p_casual.addressees.is_empty());
        assert!(p_casual.named.is_empty());
        assert_eq!(p_casual.taint, Taint::Untrusted);

        // Mentioning @mono addresses and names soul
        let p_mention = build_repl_percept(&st, "eh @Mono tolong cek dong", Time::from_millis(2000), 1)
            .expect("percept");
        assert_eq!(p_mention.place.as_str(), "cli:terminal");
        assert_eq!(p_mention.addressees, vec![ActorId::new("soul")]);
        assert_eq!(p_mention.named, vec![ActorId::new("soul")]);

        // Back to DM restores trusted owner + auto-address
        let _ = parse_repl_command("/dm", &mut st);
        assert!(!st.is_group);
        assert_eq!(st.current_actor.as_str(), "per_owner");
        let p_dm = build_repl_percept(&st, "halo", Time::from_millis(3000), 2).expect("dm");
        assert_eq!(p_dm.addressees, vec![ActorId::new("soul")]);
        assert_eq!(p_dm.taint, Taint::Trusted);

        let _ = parse_repl_command("/brain", &mut st);
        assert!(!st.show_brain);
        assert_eq!(parse_repl_command("/quit", &mut st), ReplAction::Quit);
    }

    #[test]
    fn interactive_door_controller_admits_dynamic_repl_actors_without_startup_flag() {
        let owner = ActorId::new("per_owner");
        let mut snapshot = ValveSnapshot {
            owner_actor_id: Some(owner.clone()),
            t_now: Time::from_millis(1000),
            ..ValveSnapshot::default()
        };
        snapshot.known_actors.insert(owner.clone());
        let valve = Valve::with_default_policies(ValveConfig::default(), snapshot).expect("valve");
        let dynamic_actors = Arc::new(Mutex::new(BTreeSet::new()));
        let mut door = CliInteractiveDoorController {
            inner: ValveDoorController::new(valve),
            dynamic_actors: Arc::clone(&dynamic_actors),
        };

        let mut st = ReplSessionState::new(owner, PlaceId::new("cli:terminal"), true);
        let _ = parse_repl_command("/group tugas_pameran muqsith,obby,nejad", &mut st);
        let _ = parse_repl_command("/as muqsith", &mut st);

        // Before dynamic registration, unknown actor is blocked by strict Valve policy.
        let mut p_before =
            build_repl_percept(&st, "halo gess", Time::from_millis(1500), 0).expect("percept");
        assert!(matches!(door.inbound(&mut p_before), DoorDecision::Refuse { .. }));

        // After REPL actor sync (no --actors flag needed), inbound admits the simulated group turn.
        register_session_actors(&dynamic_actors, &st);
        let mut p_after =
            build_repl_percept(&st, "halo gess", Time::from_millis(2000), 1).expect("percept");
        assert!(matches!(door.inbound(&mut p_after), DoorDecision::Allow));
    }

    #[test]
    fn decision_trace_summary_renders_tier_probs_and_correction() {
        let trace = DecisionTrace {
            neural: NeuralOutputs {
                gate_logits: [0.0, 0.0, 0.1, 2.0],
                gate_probs: [0.05, 0.05, 0.10, 0.80],
                predicted_class: GateClass::Go,
                wait_delta_ms: 0.0,
                epistemic_uncertainty: 0.02,
                cic_abstain: false,
                h_next: Vec::new(),
                h_actor_next: Vec::new(),
                p_uncertainty: Some(0.1),
                p_verify: Some(0.2),
                p_encode_importance: None,
                p_memory_relevance: None,
                compute_tier_probs: Some([0.1, 0.2, 0.7]),
                compute_tier: Some(ComputeTier::Deep),
                p_user_correction: Some(0.87),
                world_pred: None,
                model_blake3: None,
                feature_schema_version: 2,
                is_neutral_fallback: false,
                online_adaptation_applied: true,
            },
            model_blake3: None,
            feature_schema_version: 2,
            online_adaptation_applied: true,
            commitment_bypass: false,
            self_caused: false,
            draw: 0.2,
            probability: 0.8,
            outcome: DecisionOutcome::Go,
            reason: DecisionReason::NeuralGo,
        };
        let line = format_decision_trace_summary(&trace);
        assert!(line.contains("Go"));
        assert!(line.contains("tier=Deep"));
        assert!(line.contains("p_go=0.80"));
        assert!(line.contains("p_corr=0.87"));
    }

    #[test]
    fn prequeue_service_enriches_repl_percept_embeddings_with_t_d_tag() {
        let embedder = Arc::new(HashEmbedder::with_model_ver("egemma2-256", 256));
        let clock = Arc::new(SystemClock::new());
        let diagnostics = DiagnosticSink::new(&ProviderSecrets::default());
        let pq = PreQueueService::new(embedder, None, None, diagnostics, clock, 2000);
        let st = ReplSessionState::new(
            ActorId::new("per_owner"),
            PlaceId::new("cli:terminal"),
            true,
        );
        let mut percept =
            build_repl_percept(&st, "progres foto lantai 3 sudah 70%", Time::from_millis(1000), 0)
                .expect("percept");
        assert!(percept.embeddings.is_empty());
        let report = pq.enrich_percept_sync(&mut percept);
        assert!(report.embedded_tag.is_some());
        assert_eq!(percept.embeddings.len(), 1);
        assert_eq!(percept.embeddings[0].tag.embed_model_ver, "egemma2-256");
        assert_eq!(percept.embeddings[0].tag.prompt_regime_id, "t_d");
        assert_eq!(percept.embeddings[0].values.len(), 256);
    }
}
