//! Deterministic binary snapshot serialization for Slim Core V2 [`CoreState`], including
//! per-place `64d` recurrent neural state `h_t` and bounded online adaptation state [`PlaceAdaptState`].

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{
    ActionId, ActorFacts, ActorId, AttachmentHandle, Audience, BodyId, BodyStatus,
    CapabilityDescriptor, CapabilityFlags, CapabilityId, CapabilitySchemaHash, ChannelId,
    DeclassToken, DomainId, EmbeddingTag, EventId, ModalitySlot, OpaquePayload, OriginActor,
    OwnerSigned, PlaceId, Receipt, ReceiptOutcome, TaggedEmbedding, TerminationReason, Time,
    VisibleScope,
};

use super::{ActorFactsEntry, CoreState, InFlightAction, PendingApprovalEntry, WakeSnapshotRecord};
use crate::activation::ActivationTrace;
use crate::beliefs::{BeliefBook, BeliefLogEntry, BeliefRecord, BetaCredence};
use crate::consolidation::{
    EpisodeBook, EpisodeRecord, EpisodeState, FailureEpisodeRecord, LessonEntry, PendingSleepRun,
    ProcedureLifecycleState, SkillBook, SuccessEpisodeRecord, TrackedProcedure,
};
use crate::context::{
    AutoRecallTriggerState, CapabilityCatalogEntry, IdentityAnchor, PrincipleDescriptor,
    PrincipleSeverity, ProcedureEntry,
};
use crate::event::{
    BinReader, BinWriter, EventFormatError, EventRecord, LogSeq, decode_core_config,
    decode_expect_kind, decode_opt_sensed_effect, decode_taint, encode_core_config,
    encode_expect_kind, encode_opt_sensed_effect, encode_taint,
};
use crate::expectations::ExpectationBook;
use crate::gate::{
    CandidateMemory, ExpectationCue, FrozenDialogueActHead, OpenWait, PlaceHistoryEntry,
    PromiseSummary,
};
use crate::ledger::{
    ActorCard, CapabilityHealthStats, FalseBeliefFlag, OpenRequestItem, PerceptionLedger,
    SelfCalibrationStats, ToldLedgerItem, WitnessedFactSummary,
};
use crate::neural::PlaceAdaptState;
use crate::recall::{ImportanceInputs, MemoryBasis, MemoryEntry};

const SNAPSHOT_MAGIC: &[u8; 4] = b"MGSN";
const SNAPSHOT_VERSION: u16 = 11;

pub fn encode_snapshot(state: &CoreState) -> Vec<u8> {
    let mut w = BinWriter::new();
    w.raw(SNAPSHOT_MAGIC);
    w.u16(SNAPSHOT_VERSION);
    w.u64(state.last_seq.as_u64());
    w.u64(state.epoch);
    w.u64(state.next_action_seq);
    w.opt_u64(state.last_timestamp_ms.map(Time::as_millis));

    match &state.identity {
        Some(id) => {
            w.u8(1);
            w.bytes(&encode_identity_payload(id));
        }
        None => w.u8(0),
    }

    w.u32(state.body_states.len() as u32);
    for (body_id, status) in &state.body_states {
        w.str(body_id.as_str());
        w.u8(encode_body_status(*status));
    }

    w.u32(state.actor_facts.len() as u32);
    for (actor_id, entry) in &state.actor_facts {
        w.str(actor_id.as_str());
        w.bytes(&encode_actor_facts(entry));
    }

    w.u32(state.in_flight_actions.len() as u32);
    for (action_id, act) in &state.in_flight_actions {
        w.str(action_id.as_str());
        w.str(act.capability_id.as_str());
        w.u32(act.places.len() as u32);
        for place in &act.places {
            w.str(place.as_str());
        }
        w.u64(act.dispatched_at_ms.as_millis());
        w.u64(act.event_id.as_u64());
    }

    w.u32(state.pending_approvals.len() as u32);
    for (action_id, pa) in &state.pending_approvals {
        w.str(action_id.as_str());
        w.str(pa.capability_id.as_str());
        w.opt_str(pa.h_act.as_ref().map(monoagens_protocol::HAct::as_str));
        w.u64(pa.held_at_ms.as_millis());
    }

    w.u32(state.terminal_receipts.len() as u32);
    for (action_id, receipt) in &state.terminal_receipts {
        w.str(action_id.as_str());
        w.str(receipt.capability_id.as_str());
        w.u8(encode_receipt_outcome(receipt.outcome));
        w.opt_str(receipt.reason_code.as_deref());
        w.opt_str(receipt.detail.as_deref());
        match receipt.termination_reason {
            Some(tr) => {
                w.u8(1);
                w.u8(encode_term_reason(tr));
            }
            None => w.u8(0),
        }
        w.opt_f32(receipt.progress);
        w.opt_u64(receipt.delivered_units);
        w.opt_str(receipt.h_act.as_ref().map(monoagens_protocol::HAct::as_str));
        w.u64(receipt.timestamp_ms.as_millis());
    }

    w.u32(state.unconsumed_inputs.len() as u32);
    for (&event_id, rec) in &state.unconsumed_inputs {
        w.u64(event_id.as_u64());
        w.bytes(&rec.encode_bytes());
    }

    w.str(state.soul_actor_id.as_str());

    w.u32(state.place_presence.len() as u32);
    for (place, tr) in &state.place_presence {
        w.str(place.as_str());
        encode_activation_trace(&mut w, tr);
    }

    w.u32(state.actor_exchanges.len() as u32);
    for (actor, tr) in &state.actor_exchanges {
        w.str(actor.as_str());
        encode_activation_trace(&mut w, tr);
    }

    w.u32(state.place_history.len() as u32);
    for (place, entries) in &state.place_history {
        w.str(place.as_str());
        w.u32(entries.len() as u32);
        for entry in entries {
            w.u64(entry.event_id.as_u64());
            w.str(entry.actor.as_str());
            encode_audience(&mut w, &entry.audience);
            w.u64(entry.timestamp_ms.as_millis());
            w.u32(entry.deg_by_actor.len() as u32);
            for (a, &d) in &entry.deg_by_actor {
                w.str(a.as_str());
                w.f64(d);
            }
        }
    }

    w.u32(state.place_participants.len() as u32);
    for (place, set) in &state.place_participants {
        w.str(place.as_str());
        w.u32(set.len() as u32);
        for actor in set {
            w.str(actor.as_str());
        }
    }

    w.u32(state.talk_turns.len() as u32);
    for (place, turns) in &state.talk_turns {
        w.str(place.as_str());
        w.u32(turns.len() as u32);
        for &(ts, is_soul) in turns {
            w.u64(ts.as_millis());
            w.u8(u8::from(is_soul));
        }
    }

    // Per-place 64d recurrent neural state h_t and online adaptation equalizer state.
    w.u32(state.place_memory.len() as u32);
    for (place, h) in &state.place_memory {
        w.str(place.as_str());
        w.u32(h.len() as u32);
        for &v in h {
            w.f32(v);
        }
    }

    w.u32(state.place_adapt.len() as u32);
    for (place, ad) in &state.place_adapt {
        w.str(place.as_str());
        for &v in &ad.logit_offsets {
            w.f32(v);
        }
        for &v in &ad.film_scale_deltas {
            w.f32(v);
        }
        for &v in &ad.film_shift_deltas {
            w.f32(v);
        }
        for &v in &ad.arm_pulls {
            w.u64(v);
        }
        for &v in &ad.arm_reward_sums {
            w.f64(v);
        }
        w.u64(ad.total_pulls);
        w.opt_u64(ad.last_updated_ms.map(Time::as_millis));
    }

    w.u32(state.memory_candidates.len() as u32);
    for mem in &state.memory_candidates {
        w.u64(mem.event_id.as_u64());
        encode_audience(&mut w, &mem.audience);
        encode_activation_trace(&mut w, &mem.activation);
        w.f64(mem.importance);
        encode_tagged_embeddings(&mut w, &mem.embeddings);
    }

    w.u32(state.open_promises.len() as u32);
    for (&eid, p) in &state.open_promises {
        w.u64(eid.as_u64());
        w.u64(p.due_ms.as_millis());
        w.f64(p.urgency_weight);
        w.u8(u8::from(p.is_recurring_monitor));
        w.u8(u8::from(p.active));
    }

    w.u32(state.expectation_cues.len() as u32);
    for cue in &state.expectation_cues {
        w.opt_u64(cue.expectation_id.map(EventId::as_u64));
        encode_audience(&mut w, &cue.audience);
        w.opt_str(cue.counterparty.as_ref().map(ActorId::as_str));
        w.u8(u8::from(cue.answers_event));
        encode_tagged_embeddings(&mut w, &cue.embeddings);
    }

    w.u32(state.event_authors.len() as u32);
    for (&eid, actor) in &state.event_authors {
        w.u64(eid.as_u64());
        w.str(actor.as_str());
    }

    w.u32(state.active_chains.len() as u32);
    for &eid in &state.active_chains {
        w.u64(eid.as_u64());
    }

    match &state.dialogue_act_head {
        Some(head) => {
            w.u8(1);
            w.u32(head.w_gap.len() as u32);
            for &val in &head.w_gap {
                w.f32(val);
            }
            w.f32(head.b_gap);
            match &head.expected_tag {
                Some(tag) => {
                    w.u8(1);
                    w.str(&tag.embed_model_ver);
                    w.u32(tag.dim_d);
                    w.str(&tag.prompt_regime_id);
                }
                None => w.u8(0),
            }
        }
        None => w.u8(0),
    }

    w.f64(state.daily_budget_used);
    w.f64(state.daily_budget_limit);
    w.u64(state.unconsolidated_count as u64);
    w.opt_u64(state.last_sleep_ms.map(Time::as_millis));

    encode_open_waits(&mut w, &state.open_waits);
    encode_memory_entries(&mut w, &state.memory_entries);
    encode_logged_declass_tokens(&mut w, &state.logged_declass_tokens);
    state.expectation_book.encode(&mut w);
    encode_capability_descriptors(&mut w, &state.capability_descriptors);

    encode_context_and_books(&mut w, state);

    match &state.recorded_config {
        Some(cfg) => {
            w.u8(1);
            encode_core_config(&mut w, cfg);
        }
        None => w.u8(0),
    }
    w.u64(state.config_loaded_count);
    w.u32(state.decision_neural_hints.len() as u32);
    for (&eid, &(p_verify, p_uncertainty)) in &state.decision_neural_hints {
        w.u64(eid.as_u64());
        match p_verify {
            Some(v) => {
                w.u8(1);
                w.f32(v);
            }
            None => w.u8(0),
        }
        match p_uncertainty {
            Some(v) => {
                w.u8(1);
                w.f32(v);
            }
            None => w.u8(0),
        }
    }

    w.finish()
}

pub fn decode_snapshot(bytes: &[u8]) -> Result<CoreState, EventFormatError> {
    let mut r = BinReader::new(bytes);
    let magic = r.fixed::<4>()?;
    if &magic != SNAPSHOT_MAGIC {
        return Err(EventFormatError::MalformedRecord(
            "invalid snapshot magic header".to_owned(),
        ));
    }
    let version = r.u16()?;
    if version != 10 && version != SNAPSHOT_VERSION {
        return Err(EventFormatError::MalformedRecord(format!(
            "unsupported snapshot version {version} (supported 10..={SNAPSHOT_VERSION})"
        )));
    }

    let last_seq = LogSeq::new(r.u64()?);
    let epoch = r.u64()?;
    let next_action_seq = r.u64()?;
    let last_timestamp_ms = r.opt_u64()?.map(Time::from_millis);

    let identity = match r.u8()? {
        0 => None,
        1 => Some(decode_identity_payload(&r.bytes()?)?),
        tag => {
            return Err(EventFormatError::MalformedRecord(format!(
                "invalid identity tag {tag}"
            )));
        }
    };

    let b_len = r.u32()? as usize;
    let mut body_states = BTreeMap::new();
    for _ in 0..b_len {
        let id = BodyId::new(r.string()?);
        let st = decode_body_status(r.u8()?)?;
        body_states.insert(id, st);
    }

    let af_len = r.u32()? as usize;
    let mut actor_facts = BTreeMap::new();
    for _ in 0..af_len {
        let id = ActorId::new(r.string()?);
        let entry = decode_actor_facts(&r.bytes()?)?;
        actor_facts.insert(id, entry);
    }

    let inf_len = r.u32()? as usize;
    let mut in_flight_actions = BTreeMap::new();
    for _ in 0..inf_len {
        let action_id = ActionId::new(r.string()?);
        let capability_id = CapabilityId::new(r.string()?);
        let plen = r.u32()? as usize;
        let mut places = Vec::with_capacity(plen.min(128));
        for _ in 0..plen {
            places.push(PlaceId::new(r.string()?));
        }
        let dispatched_at_ms = Time::from_millis(r.u64()?);
        let event_id = EventId::new(r.u64()?);
        in_flight_actions.insert(
            action_id.clone(),
            InFlightAction {
                action_id,
                capability_id,
                places,
                dispatched_at_ms,
                event_id,
            },
        );
    }

    let pa_len = r.u32()? as usize;
    let mut pending_approvals = BTreeMap::new();
    for _ in 0..pa_len {
        let action_id = ActionId::new(r.string()?);
        let capability_id = CapabilityId::new(r.string()?);
        let h_act = r.opt_string()?.map(monoagens_protocol::HAct::new);
        let held_at_ms = Time::from_millis(r.u64()?);
        pending_approvals.insert(
            action_id.clone(),
            PendingApprovalEntry {
                action_id,
                capability_id,
                h_act,
                held_at_ms,
            },
        );
    }

    let tr_len = r.u32()? as usize;
    let mut terminal_receipts = BTreeMap::new();
    for _ in 0..tr_len {
        let action_id = ActionId::new(r.string()?);
        let capability_id = CapabilityId::new(r.string()?);
        let outcome = decode_receipt_outcome(r.u8()?)?;
        let reason_code = r.opt_string()?;
        let detail = r.opt_string()?;
        let termination_reason = match r.u8()? {
            0 => None,
            1 => Some(decode_term_reason(r.u8()?)?),
            tag => {
                return Err(EventFormatError::MalformedRecord(format!(
                    "invalid termination_reason tag {tag}"
                )));
            }
        };
        let progress = r.opt_f32()?;
        let delivered_units = r.opt_u64()?;
        let h_act = r.opt_string()?.map(monoagens_protocol::HAct::new);
        let timestamp_ms = Time::from_millis(r.u64()?);
        terminal_receipts.insert(
            action_id.clone(),
            Receipt {
                action_id,
                capability_id,
                outcome,
                reason_code,
                detail,
                termination_reason,
                progress,
                delivered_units,
                h_act,
                timestamp_ms,
            },
        );
    }

    let unc_len = r.u32()? as usize;
    let mut unconsumed_inputs = BTreeMap::new();
    for _ in 0..unc_len {
        let eid = EventId::new(r.u64()?);
        let raw = r.bytes()?;
        let rec = EventRecord::decode_and_upcast(&raw)?;
        unconsumed_inputs.insert(eid, rec);
    }

    let soul_actor_id = ActorId::new(r.string()?);

    let pp_len = r.u32()? as usize;
    let mut place_presence = BTreeMap::new();
    for _ in 0..pp_len {
        let place = PlaceId::new(r.string()?);
        let tr = decode_activation_trace(&mut r)?;
        place_presence.insert(place, tr);
    }

    let ae_len = r.u32()? as usize;
    let mut actor_exchanges = BTreeMap::new();
    for _ in 0..ae_len {
        let actor = ActorId::new(r.string()?);
        let tr = decode_activation_trace(&mut r)?;
        actor_exchanges.insert(actor, tr);
    }

    let ph_len = r.u32()? as usize;
    let mut place_history = BTreeMap::new();
    for _ in 0..ph_len {
        let place = PlaceId::new(r.string()?);
        let elen = r.u32()? as usize;
        let mut entries = Vec::with_capacity(elen.min(256));
        for _ in 0..elen {
            let event_id = EventId::new(r.u64()?);
            let actor = ActorId::new(r.string()?);
            let audience = decode_audience(&mut r)?;
            let timestamp_ms = Time::from_millis(r.u64()?);
            let dlen = r.u32()? as usize;
            let mut deg_by_actor = BTreeMap::new();
            for _ in 0..dlen {
                let a = ActorId::new(r.string()?);
                let d = r.f64()?;
                deg_by_actor.insert(a, d);
            }
            entries.push(PlaceHistoryEntry {
                event_id,
                actor,
                audience,
                timestamp_ms,
                deg_by_actor,
            });
        }
        place_history.insert(place, entries);
    }

    let part_len = r.u32()? as usize;
    let mut place_participants = BTreeMap::new();
    for _ in 0..part_len {
        let place = PlaceId::new(r.string()?);
        let slen = r.u32()? as usize;
        let mut set = BTreeSet::new();
        for _ in 0..slen {
            set.insert(ActorId::new(r.string()?));
        }
        place_participants.insert(place, set);
    }

    let tt_len = r.u32()? as usize;
    let mut talk_turns = BTreeMap::new();
    for _ in 0..tt_len {
        let place = PlaceId::new(r.string()?);
        let tlen = r.u32()? as usize;
        let mut turns = Vec::with_capacity(tlen.min(1024));
        for _ in 0..tlen {
            let ts = Time::from_millis(r.u64()?);
            let is_soul = r.u8()? != 0;
            turns.push((ts, is_soul));
        }
        talk_turns.insert(place, turns);
    }

    let pm_len = r.u32()? as usize;
    let mut place_memory = BTreeMap::new();
    for _ in 0..pm_len {
        let place = PlaceId::new(r.string()?);
        let h_len = r.u32()? as usize;
        let mut h = Vec::with_capacity(h_len.min(512));
        for _ in 0..h_len {
            let v = r.f32()?;
            if !v.is_finite() {
                return Err(EventFormatError::MalformedRecord(
                    "non-finite place_memory element in snapshot".to_owned(),
                ));
            }
            h.push(v);
        }
        place_memory.insert(place, h);
    }

    let pa_adapt_len = r.u32()? as usize;
    let mut place_adapt = BTreeMap::new();
    for _ in 0..pa_adapt_len {
        let place = PlaceId::new(r.string()?);
        let mut logit_offsets = [0.0_f32; 4];
        for s in &mut logit_offsets {
            *s = r.f32()?;
        }
        let mut film_scale_deltas = [0.0_f32; 2];
        for s in &mut film_scale_deltas {
            *s = r.f32()?;
        }
        let mut film_shift_deltas = [0.0_f32; 2];
        for s in &mut film_shift_deltas {
            *s = r.f32()?;
        }
        let mut arm_pulls = [0_u64; 4];
        for s in &mut arm_pulls {
            *s = r.u64()?;
        }
        let mut arm_reward_sums = [0.0_f64; 4];
        for s in &mut arm_reward_sums {
            *s = r.f64()?;
        }
        let total_pulls = r.u64()?;
        let last_updated_ms = if version >= 11 {
            r.opt_u64()?.map(Time::from_millis)
        } else {
            None
        };
        place_adapt.insert(
            place,
            PlaceAdaptState {
                logit_offsets,
                film_scale_deltas,
                film_shift_deltas,
                arm_pulls,
                arm_reward_sums,
                total_pulls,
                last_updated_ms,
            },
        );
    }

    let mc_len = r.u32()? as usize;
    let mut memory_candidates = Vec::with_capacity(mc_len.min(4096));
    for _ in 0..mc_len {
        let event_id = EventId::new(r.u64()?);
        let audience = decode_audience(&mut r)?;
        let activation = decode_activation_trace(&mut r)?;
        let importance = r.f64()?;
        let embeddings = decode_tagged_embeddings(&mut r)?;
        memory_candidates.push(CandidateMemory {
            event_id,
            audience,
            activation,
            importance,
            embeddings,
        });
    }

    let op_len = r.u32()? as usize;
    let mut open_promises = BTreeMap::new();
    for _ in 0..op_len {
        let eid = EventId::new(r.u64()?);
        let due_ms = Time::from_millis(r.u64()?);
        let urgency_weight = r.f64()?;
        let is_recurring_monitor = r.u8()? != 0;
        let active = r.u8()? != 0;
        open_promises.insert(
            eid,
            PromiseSummary {
                due_ms,
                urgency_weight,
                is_recurring_monitor,
                active,
            },
        );
    }

    let ec_len = r.u32()? as usize;
    let mut expectation_cues = Vec::with_capacity(ec_len.min(1024));
    for _ in 0..ec_len {
        let expectation_id = r.opt_u64()?.map(EventId::new);
        let audience = decode_audience(&mut r)?;
        let counterparty = r.opt_string()?.map(ActorId::new);
        let answers_event = r.u8()? != 0;
        let embeddings = decode_tagged_embeddings(&mut r)?;
        expectation_cues.push(ExpectationCue {
            expectation_id,
            audience,
            counterparty,
            answers_event,
            embeddings,
        });
    }

    let ea_len = r.u32()? as usize;
    let mut event_authors = BTreeMap::new();
    for _ in 0..ea_len {
        let eid = EventId::new(r.u64()?);
        let actor = ActorId::new(r.string()?);
        event_authors.insert(eid, actor);
    }

    let ac_len = r.u32()? as usize;
    let mut active_chains = BTreeSet::new();
    for _ in 0..ac_len {
        active_chains.insert(EventId::new(r.u64()?));
    }

    let dialogue_act_head = match r.u8()? {
        0 => None,
        1 => {
            let w_len = r.u32()? as usize;
            let mut w_gap = Vec::with_capacity(w_len.min(4096));
            for _ in 0..w_len {
                w_gap.push(r.f32()?);
            }
            let b_gap = r.f32()?;
            let expected_tag = match r.u8()? {
                0 => None,
                1 => Some(EmbeddingTag {
                    embed_model_ver: r.string()?,
                    dim_d: r.u32()?,
                    prompt_regime_id: r.string()?,
                }),
                tag => {
                    return Err(EventFormatError::MalformedRecord(format!(
                        "invalid expected_tag tag {tag}"
                    )));
                }
            };
            Some(FrozenDialogueActHead {
                w_gap,
                b_gap,
                expected_tag,
            })
        }
        tag => {
            return Err(EventFormatError::MalformedRecord(format!(
                "invalid dialogue_act_head tag {tag}"
            )));
        }
    };

    let daily_budget_used = r.f64()?;
    let daily_budget_limit = r.f64()?;
    let unconsolidated_count = r.u64()? as usize;
    let last_sleep_ms = r.opt_u64()?.map(Time::from_millis);

    let open_waits = decode_open_waits(&mut r)?;
    let memory_entries = decode_memory_entries(&mut r)?;
    let logged_declass_tokens = decode_logged_declass_tokens(&mut r)?;
    let expectation_book = ExpectationBook::decode(&mut r)?;
    let capability_descriptors = decode_capability_descriptors(&mut r)?;

    let ctx_books = decode_context_and_books(&mut r, version)?;

    let recorded_config = match r.u8()? {
        0 => None,
        1 => Some(decode_core_config(&mut r)?),
        tag => {
            return Err(EventFormatError::MalformedRecord(format!(
                "invalid recorded_config tag {tag}"
            )));
        }
    };
    let config_loaded_count = r.u64()?;
    let decision_neural_hints = if version >= 11 {
        let hint_len = r.u32()? as usize;
        let mut hints = BTreeMap::new();
        for _ in 0..hint_len {
            let eid = EventId::new(r.u64()?);
            let p_verify = match r.u8()? {
                0 => None,
                1 => Some(r.f32()?),
                tag => {
                    return Err(EventFormatError::MalformedRecord(format!(
                        "invalid opt p_verify tag {tag}"
                    )));
                }
            };
            let p_uncertainty = match r.u8()? {
                0 => None,
                1 => Some(r.f32()?),
                tag => {
                    return Err(EventFormatError::MalformedRecord(format!(
                        "invalid opt p_uncertainty tag {tag}"
                    )));
                }
            };
            hints.insert(eid, (p_verify, p_uncertainty));
        }
        hints
    } else {
        BTreeMap::new()
    };

    let startup_recovery_pending = !unconsumed_inputs.is_empty();
    Ok(CoreState {
        last_seq,
        epoch,
        next_action_seq,
        last_timestamp_ms,
        identity,
        body_states,
        actor_facts,
        in_flight_actions,
        pending_approvals,
        terminal_receipts,
        unconsumed_inputs,
        soul_actor_id,
        place_presence,
        actor_exchanges,
        place_history,
        place_participants,
        talk_turns,
        place_memory,
        place_adapt,
        memory_candidates,
        open_promises,
        expectation_cues,
        event_authors,
        active_chains,
        dialogue_act_head,
        daily_budget_used,
        daily_budget_limit,
        unconsolidated_count,
        last_sleep_ms,
        open_waits,
        memory_entries,
        logged_declass_tokens,
        expectation_book,
        capability_descriptors,
        startup_recovery_pending,
        identity_anchor: ctx_books.identity_anchor,
        principles: ctx_books.principles,
        capability_catalog: ctx_books.capability_catalog,
        procedures: ctx_books.procedures,
        knowledge_cutoff: ctx_books.knowledge_cutoff,
        domain_calibration_offsets: ctx_books.domain_calibration_offsets,
        active_wake_snapshots: ctx_books.active_wake_snapshots,
        auto_recall_states: ctx_books.auto_recall_states,
        recent_receipt_cues: ctx_books.recent_receipt_cues,
        wake_count_since_anchor_check: ctx_books.wake_count_since_anchor_check,
        anchor_check_pending_post_sleep: ctx_books.anchor_check_pending_post_sleep,
        last_sleep_seq: ctx_books.last_sleep_seq,
        reliability_alpha: ctx_books.reliability_alpha,
        reliability_beta: ctx_books.reliability_beta,
        latency_samples_ms: ctx_books.latency_samples_ms,
        turn_allocated_by_trigger: ctx_books.turn_allocated_by_trigger,
        calibration_samples: ctx_books.calibration_samples,
        calibration_brier_sum: ctx_books.calibration_brier_sum,
        principle_alpha: ctx_books.principle_alpha,
        principle_beta: ctx_books.principle_beta,
        contradiction_flags: ctx_books.contradiction_flags,
        belief_book: ctx_books.belief_book,
        episode_book: ctx_books.episode_book,
        pending_sleep_run: ctx_books.pending_sleep_run,
        last_global_activity_ms: ctx_books.last_global_activity_ms,
        skill_book: ctx_books.skill_book,
        ledger: ctx_books.ledger,
        procedure_links: ctx_books.procedure_links,
        recalibration_pending: ctx_books.recalibration_pending,
        recorded_config,
        config_loaded_count,
        decision_neural_hints,
    })
}

struct ContextAndBooks {
    identity_anchor: Option<OwnerSigned<IdentityAnchor>>,
    principles: Vec<OwnerSigned<PrincipleDescriptor>>,
    capability_catalog: BTreeMap<CapabilityId, CapabilityCatalogEntry>,
    procedures: BTreeMap<EventId, ProcedureEntry>,
    knowledge_cutoff: Option<String>,
    domain_calibration_offsets: BTreeMap<DomainId, f64>,
    active_wake_snapshots: BTreeMap<EventId, WakeSnapshotRecord>,
    auto_recall_states: BTreeMap<EventId, AutoRecallTriggerState>,
    recent_receipt_cues: Vec<String>,
    wake_count_since_anchor_check: u64,
    anchor_check_pending_post_sleep: bool,
    last_sleep_seq: LogSeq,
    reliability_alpha: f64,
    reliability_beta: f64,
    latency_samples_ms: Vec<u64>,
    turn_allocated_by_trigger: BTreeMap<EventId, bool>,
    calibration_samples: u64,
    calibration_brier_sum: f64,
    principle_alpha: f64,
    principle_beta: f64,
    contradiction_flags: u64,
    belief_book: BeliefBook,
    episode_book: EpisodeBook,
    pending_sleep_run: Option<PendingSleepRun>,
    last_global_activity_ms: Option<Time>,
    skill_book: SkillBook,
    ledger: PerceptionLedger,
    procedure_links: BTreeMap<ActionId, EventId>,
    recalibration_pending: bool,
}

fn encode_context_and_books(w: &mut BinWriter, state: &CoreState) {
    match &state.identity_anchor {
        None => w.u8(0),
        Some(signed) => {
            w.u8(1);
            w.str(signed.signer.as_str());
            w.u64(signed.signed_at_ms.as_millis());
            w.str(&signed.signature);
            w.str(&signed.payload.anchor_id);
            w.bytes(&signed.payload.identity_blob_hash);
            w.bytes(&signed.payload.owner_sig_hash);
            w.bytes(signed.payload.payload.as_bytes());
        }
    }

    w.u32(state.principles.len() as u32);
    for signed in &state.principles {
        w.str(signed.signer.as_str());
        w.u64(signed.signed_at_ms.as_millis());
        w.str(&signed.signature);
        w.str(&signed.payload.id);
        w.u8(match signed.payload.severity {
            PrincipleSeverity::HardLine => 0,
            PrincipleSeverity::StrongNorm => 1,
            PrincipleSeverity::Advisory => 2,
        });
        w.u32(signed.payload.blocked_capabilities.len() as u32);
        for cap in &signed.payload.blocked_capabilities {
            w.str(cap.as_str());
        }
        w.u8(u8::from(signed.payload.forbid_identity_change));
        w.u8(u8::from(signed.payload.forbid_untrusted_taint));
        w.u64(signed.payload.created_seq.as_u64());
        w.bytes(&signed.payload.owner_sig_hash);
    }

    w.u32(state.capability_catalog.len() as u32);
    for (cap_id, entry) in &state.capability_catalog {
        w.str(cap_id.as_str());
        w.u8(u8::from(entry.is_static_core));
        w.u8(u8::from(entry.owner_signed));
        w.u8(u8::from(entry.active));
        encode_audience(w, &entry.audience);
        let single_map: BTreeMap<CapabilityId, CapabilityDescriptor> =
            [(cap_id.clone(), entry.descriptor.clone())]
                .into_iter()
                .collect();
        encode_capability_descriptors(w, &single_map);
        let embs: Vec<TaggedEmbedding> = entry.embedding.iter().cloned().collect();
        encode_tagged_embeddings(w, &embs);
    }

    w.u32(state.procedures.len() as u32);
    for (proc_id, p) in &state.procedures {
        w.u64(proc_id.as_u64());
        w.str(&p.name);
        w.u32(p.steps.len() as u32);
        for s in &p.steps {
            w.str(s);
        }
        w.u32(p.required_cap_ids.len() as u32);
        for cid in &p.required_cap_ids {
            w.str(cid.as_str());
        }
        w.str(p.capability_schema_hash.as_str());
        encode_audience(w, &p.audience);
        w.opt_str(p.domain.as_ref().map(DomainId::as_str));
        w.u8(u8::from(p.active));
        w.u8(u8::from(p.retired));
        w.u8(u8::from(p.owner_signed));
        let embs: Vec<TaggedEmbedding> = p.embedding.iter().cloned().collect();
        encode_tagged_embeddings(w, &embs);
    }

    w.opt_str(state.knowledge_cutoff.as_deref());
    w.u32(state.domain_calibration_offsets.len() as u32);
    for (domain, offset) in &state.domain_calibration_offsets {
        w.str(domain.as_str());
        w.f64(*offset);
    }

    w.u32(state.active_wake_snapshots.len() as u32);
    for (&eid, ws) in &state.active_wake_snapshots {
        w.u64(eid.as_u64());
        w.u32(ws.visible_ids.len() as u32);
        for &vid in &ws.visible_ids {
            w.u64(vid.as_u64());
        }
        encode_audience(w, &ws.visible_scope.audience);
        w.opt_str(ws.visible_scope.target_place.as_ref().map(PlaceId::as_str));
        encode_declass_tokens(w, &ws.visible_scope.declass_tokens);
        w.u32(ws.disclosed_capabilities.len() as u32);
        for cid in &ws.disclosed_capabilities {
            w.str(cid.as_str());
        }
        w.u8(u8::from(ws.turn_allocated));
        w.str(ws.wake_place.as_str());
        w.u8(u8::from(ws.anchor_check_due));
    }

    w.u32(state.auto_recall_states.len() as u32);
    for (ev_id, ar) in &state.auto_recall_states {
        w.u64(ev_id.as_u64());
        w.u8(u8::from(ar.active));
        w.u8(ar.attempt);
        w.u8(ar.max_attempts);
        w.u8(u8::from(ar.coarser_grain_allowed));
        w.opt_str(ar.last_reason.as_deref());
    }

    w.u32(state.recent_receipt_cues.len() as u32);
    for cue in &state.recent_receipt_cues {
        w.str(cue);
    }
    w.u64(state.wake_count_since_anchor_check);
    w.u8(u8::from(state.anchor_check_pending_post_sleep));
    w.u64(state.last_sleep_seq.as_u64());
    w.f64(state.reliability_alpha);
    w.f64(state.reliability_beta);
    w.u32(state.latency_samples_ms.len() as u32);
    for &l in &state.latency_samples_ms {
        w.u64(l);
    }
    w.u32(state.turn_allocated_by_trigger.len() as u32);
    for (&eid, &alloc) in &state.turn_allocated_by_trigger {
        w.u64(eid.as_u64());
        w.u8(u8::from(alloc));
    }
    w.u64(state.calibration_samples);
    w.f64(state.calibration_brier_sum);
    w.f64(state.principle_alpha);
    w.f64(state.principle_beta);
    w.u64(state.contradiction_flags);

    w.opt_u64(state.last_global_activity_ms.map(Time::as_millis));
    encode_belief_book(w, &state.belief_book);
    encode_episode_book(w, &state.episode_book);
    encode_pending_sleep_run(w, state.pending_sleep_run.as_ref());
    w.u8(u8::from(state.recalibration_pending));
    w.u32(state.procedure_links.len() as u32);
    for (aid, &pid) in &state.procedure_links {
        w.str(aid.as_str());
        w.u64(pid.as_u64());
    }
    encode_skill_book(w, &state.skill_book);
    encode_perception_ledger(w, &state.ledger);
}

fn decode_context_and_books(
    r: &mut BinReader<'_>,
    version: u16,
) -> Result<ContextAndBooks, EventFormatError> {
    let identity_anchor = match r.u8()? {
        0 => None,
        1 => {
            let signer = ActorId::new(r.string()?);
            let signed_at_ms = Time::from_millis(r.u64()?);
            let signature = r.string()?;
            let anchor_id = r.string()?;
            let identity_blob_hash = decode_fixed_32(r)?;
            let owner_sig_hash = decode_fixed_32(r)?;
            let payload = OpaquePayload::from_bytes(r.bytes()?);
            Some(OwnerSigned {
                payload: IdentityAnchor {
                    anchor_id,
                    identity_blob_hash,
                    owner_sig_hash,
                    payload,
                },
                signer,
                signed_at_ms,
                signature,
            })
        }
        tag => {
            return Err(EventFormatError::MalformedRecord(format!(
                "invalid identity_anchor tag {tag}"
            )));
        }
    };

    let p_len = r.u32()? as usize;
    let mut principles = Vec::with_capacity(p_len.min(256));
    for _ in 0..p_len {
        let signer = ActorId::new(r.string()?);
        let signed_at_ms = Time::from_millis(r.u64()?);
        let signature = r.string()?;
        let id = r.string()?;
        let severity = match r.u8()? {
            0 => PrincipleSeverity::HardLine,
            1 => PrincipleSeverity::StrongNorm,
            2 => PrincipleSeverity::Advisory,
            tag => {
                return Err(EventFormatError::MalformedRecord(format!(
                    "invalid principle severity tag {tag}"
                )));
            }
        };
        let bc_len = r.u32()? as usize;
        let mut blocked_capabilities = Vec::with_capacity(bc_len.min(256));
        for _ in 0..bc_len {
            blocked_capabilities.push(CapabilityId::new(r.string()?));
        }
        let forbid_identity_change = r.u8()? != 0;
        let forbid_untrusted_taint = r.u8()? != 0;
        let created_seq = LogSeq::new(r.u64()?);
        let owner_sig_hash = decode_fixed_32(r)?;
        principles.push(OwnerSigned {
            payload: PrincipleDescriptor {
                id,
                severity,
                blocked_capabilities,
                forbid_identity_change,
                forbid_untrusted_taint,
                created_seq,
                owner_sig_hash,
            },
            signer,
            signed_at_ms,
            signature,
        });
    }

    let cc_len = r.u32()? as usize;
    let mut capability_catalog = BTreeMap::new();
    for _ in 0..cc_len {
        let cap_id = CapabilityId::new(r.string()?);
        let is_static_core = r.u8()? != 0;
        let owner_signed = r.u8()? != 0;
        let active = r.u8()? != 0;
        let audience = decode_audience(r)?;
        let mut single = decode_capability_descriptors(r)?;
        let descriptor = single.remove(&cap_id).ok_or_else(|| {
            EventFormatError::MalformedRecord("missing catalog capability descriptor".to_owned())
        })?;
        let embedding = decode_tagged_embeddings(r)?.into_iter().next();
        capability_catalog.insert(
            cap_id,
            CapabilityCatalogEntry {
                descriptor,
                is_static_core,
                owner_signed,
                active,
                audience,
                embedding,
            },
        );
    }

    let pr_len = r.u32()? as usize;
    let mut procedures = BTreeMap::new();
    for _ in 0..pr_len {
        let procedure_id = EventId::new(r.u64()?);
        let name = r.string()?;
        let st_len = r.u32()? as usize;
        let mut steps = Vec::with_capacity(st_len.min(128));
        for _ in 0..st_len {
            steps.push(r.string()?);
        }
        let rc_len = r.u32()? as usize;
        let mut required_cap_ids = Vec::with_capacity(rc_len.min(128));
        for _ in 0..rc_len {
            required_cap_ids.push(CapabilityId::new(r.string()?));
        }
        let capability_schema_hash = CapabilitySchemaHash::new(r.string()?);
        let audience = decode_audience(r)?;
        let domain = r.opt_string()?.map(DomainId::new);
        let active = r.u8()? != 0;
        let retired = r.u8()? != 0;
        let owner_signed = r.u8()? != 0;
        let embedding = decode_tagged_embeddings(r)?.into_iter().next();
        procedures.insert(
            procedure_id,
            ProcedureEntry {
                procedure_id,
                name,
                steps,
                required_cap_ids,
                capability_schema_hash,
                audience,
                domain,
                active,
                retired,
                owner_signed,
                embedding,
            },
        );
    }

    let knowledge_cutoff = r.opt_string()?;
    let dco_len = r.u32()? as usize;
    let mut domain_calibration_offsets = BTreeMap::new();
    for _ in 0..dco_len {
        domain_calibration_offsets.insert(DomainId::new(r.string()?), r.f64()?);
    }

    let aws_len = r.u32()? as usize;
    let mut active_wake_snapshots = BTreeMap::new();
    for _ in 0..aws_len {
        let eid = EventId::new(r.u64()?);
        let v_len = r.u32()? as usize;
        let mut visible_ids = BTreeSet::new();
        for _ in 0..v_len {
            visible_ids.insert(EventId::new(r.u64()?));
        }
        let audience = decode_audience(r)?;
        let target_place = r.opt_string()?.map(PlaceId::new);
        let declass_tokens = decode_declass_tokens(r)?;
        let dc_len = r.u32()? as usize;
        let mut disclosed_capabilities = BTreeSet::new();
        for _ in 0..dc_len {
            disclosed_capabilities.insert(CapabilityId::new(r.string()?));
        }
        let turn_allocated = r.u8()? != 0;
        let wake_place = PlaceId::new(r.string()?);
        let anchor_check_due = r.u8()? != 0;
        let mut visible_scope = VisibleScope::new(audience, target_place);
        visible_scope.declass_tokens = declass_tokens;
        active_wake_snapshots.insert(
            eid,
            WakeSnapshotRecord {
                visible_ids,
                visible_scope,
                disclosed_capabilities,
                turn_allocated,
                wake_place,
                anchor_check_due,
            },
        );
    }

    let ar_len = r.u32()? as usize;
    let mut auto_recall_states = BTreeMap::new();
    for _ in 0..ar_len {
        let ev_id = EventId::new(r.u64()?);
        let active = r.u8()? != 0;
        let attempt = r.u8()?;
        let max_attempts = r.u8()?;
        let coarser_grain_allowed = r.u8()? != 0;
        let last_reason = r.opt_string()?;
        auto_recall_states.insert(
            ev_id,
            AutoRecallTriggerState {
                active,
                attempt,
                max_attempts,
                coarser_grain_allowed,
                last_reason,
            },
        );
    }

    let rrc_len = r.u32()? as usize;
    let mut recent_receipt_cues = Vec::with_capacity(rrc_len.min(64));
    for _ in 0..rrc_len {
        recent_receipt_cues.push(r.string()?);
    }
    let wake_count_since_anchor_check = r.u64()?;
    let anchor_check_pending_post_sleep = r.u8()? != 0;
    let last_sleep_seq = LogSeq::new(r.u64()?);
    let reliability_alpha = r.f64()?;
    let reliability_beta = r.f64()?;
    let ls_len = r.u32()? as usize;
    let mut latency_samples_ms = Vec::with_capacity(ls_len.min(128));
    for _ in 0..ls_len {
        latency_samples_ms.push(r.u64()?);
    }
    let ta_len = r.u32()? as usize;
    let mut turn_allocated_by_trigger = BTreeMap::new();
    for _ in 0..ta_len {
        let eid = EventId::new(r.u64()?);
        let alloc = r.u8()? != 0;
        turn_allocated_by_trigger.insert(eid, alloc);
    }
    let calibration_samples = r.u64()?;
    let calibration_brier_sum = r.f64()?;
    let principle_alpha = r.f64()?;
    let principle_beta = r.f64()?;
    let contradiction_flags = r.u64()?;

    let last_global_activity_ms = r.opt_u64()?.map(Time::from_millis);
    let belief_book = decode_belief_book(r)?;
    let episode_book = decode_episode_book(r)?;
    let pending_sleep_run = decode_pending_sleep_run(r)?;
    let recalibration_pending = r.u8()? != 0;
    let pl_len = r.u32()? as usize;
    let mut procedure_links = BTreeMap::new();
    for _ in 0..pl_len {
        procedure_links.insert(ActionId::new(r.string()?), EventId::new(r.u64()?));
    }
    let skill_book = decode_skill_book(r, version)?;
    let ledger = decode_perception_ledger(r)?;

    Ok(ContextAndBooks {
        identity_anchor,
        principles,
        capability_catalog,
        procedures,
        knowledge_cutoff,
        domain_calibration_offsets,
        active_wake_snapshots,
        auto_recall_states,
        recent_receipt_cues,
        wake_count_since_anchor_check,
        anchor_check_pending_post_sleep,
        last_sleep_seq,
        reliability_alpha,
        reliability_beta,
        latency_samples_ms,
        turn_allocated_by_trigger,
        calibration_samples,
        calibration_brier_sum,
        principle_alpha,
        principle_beta,
        contradiction_flags,
        belief_book,
        episode_book,
        pending_sleep_run,
        last_global_activity_ms,
        skill_book,
        ledger,
        procedure_links,
        recalibration_pending,
    })
}

fn encode_beta_credence(w: &mut BinWriter, c: &BetaCredence) {
    w.f64(c.alpha);
    w.f64(c.beta);
    w.f64(c.alpha_0);
    w.f64(c.beta_0);
    w.u64(c.last_updated_ms.as_millis());
}

fn decode_beta_credence(r: &mut BinReader<'_>) -> Result<BetaCredence, EventFormatError> {
    Ok(BetaCredence {
        alpha: r.f64()?,
        beta: r.f64()?,
        alpha_0: r.f64()?,
        beta_0: r.f64()?,
        last_updated_ms: Time::from_millis(r.u64()?),
    })
}

fn encode_belief_book(w: &mut BinWriter, book: &BeliefBook) {
    w.u32(book.actor_priors.len() as u32);
    for (actor, &(a0, b0)) in &book.actor_priors {
        w.str(actor.as_str());
        w.f64(a0);
        w.f64(b0);
    }
    w.u32(book.beliefs.len() as u32);
    for (&bid, b) in &book.beliefs {
        w.u64(bid.as_u64());
        w.str(&b.claim_key);
        w.u8(b.requested_basis.code());
        w.u8(b.basis.code());
        w.u8(u8::from(b.basis_downgraded));
        w.opt_str(b.source_actor.as_ref().map(ActorId::as_str));
        w.u32(b.corroborating_actors.len() as u32);
        for ca in &b.corroborating_actors {
            w.str(ca.as_str());
        }
        w.u8(encode_taint(b.taint));
        w.str(b.domain.as_str());
        encode_audience(w, &b.audience);
        w.u32(b.source_event_ids.len() as u32);
        for &sid in &b.source_event_ids {
            w.u64(sid.as_u64());
        }
        w.u64(b.valid_from_ms.as_millis());
        w.opt_u64(b.valid_to_ms.map(Time::as_millis));
        w.opt_u64(b.replaced_by.map(EventId::as_u64));
        match &b.credence {
            Some(c) => {
                w.u8(1);
                encode_beta_credence(w, c);
            }
            None => w.u8(0),
        }
        w.u8(u8::from(b.is_mind_inference));
        w.u8(u8::from(b.erased));
    }
    w.u32(book.active_claims.len() as u32);
    for (k, &eid) in &book.active_claims {
        w.str(k);
        w.u64(eid.as_u64());
    }
    w.u32(book.source_reliability.len() as u32);
    for ((actor, dom), c) in &book.source_reliability {
        w.str(actor.as_str());
        w.str(dom.as_str());
        encode_beta_credence(w, c);
    }
    w.u32(book.item_credences.len() as u32);
    for (&eid, c) in &book.item_credences {
        w.u64(eid.as_u64());
        encode_beta_credence(w, c);
    }
    w.u32(book.conventions.len() as u32);
    for ((pl, pat), c) in &book.conventions {
        w.str(pl.as_str());
        w.str(pat);
        encode_beta_credence(w, c);
    }
    w.u32(book.log_entries.len() as u32);
    for le in &book.log_entries {
        match le {
            BeliefLogEntry::BeliefRecorded {
                event_id,
                timestamp_ms,
                seq,
                claim_key,
                requested_basis,
                basis,
                basis_downgraded,
                source_actor,
                domain,
                audience,
                source_event_ids,
                taint,
                valid_from_ms,
                valid_to_ms,
                replaced_by,
                alpha_0,
                beta_0,
                is_mind_inference,
            } => {
                w.u8(1);
                w.u64(event_id.as_u64());
                w.u64(timestamp_ms.as_millis());
                w.u64(seq.as_u64());
                w.str(claim_key);
                w.u8(requested_basis.code());
                w.u8(basis.code());
                w.u8(u8::from(*basis_downgraded));
                w.opt_str(source_actor.as_ref().map(ActorId::as_str));
                w.str(domain.as_str());
                encode_audience(w, audience);
                w.u32(source_event_ids.len() as u32);
                for &sid in source_event_ids {
                    w.u64(sid.as_u64());
                }
                w.u8(encode_taint(*taint));
                w.u64(valid_from_ms.as_millis());
                w.opt_u64(valid_to_ms.map(Time::as_millis));
                w.opt_u64(replaced_by.map(EventId::as_u64));
                w.f64(*alpha_0);
                w.f64(*beta_0);
                w.u8(u8::from(*is_mind_inference));
            }
            BeliefLogEntry::EvidenceApplied {
                event_id,
                timestamp_ms,
                seq,
                target_id,
                source_actor,
                domain,
                positive,
                negative,
                is_convention,
                place,
                pattern_id,
            } => {
                w.u8(2);
                w.u64(event_id.as_u64());
                w.u64(timestamp_ms.as_millis());
                w.u64(seq.as_u64());
                w.u64(target_id.as_u64());
                w.opt_str(source_actor.as_ref().map(ActorId::as_str));
                w.str(domain.as_str());
                w.f64(*positive);
                w.f64(*negative);
                w.u8(u8::from(*is_convention));
                w.opt_str(place.as_ref().map(PlaceId::as_str));
                w.opt_str(pattern_id.as_deref());
            }
        }
    }
}

fn decode_belief_book(r: &mut BinReader<'_>) -> Result<BeliefBook, EventFormatError> {
    let mut book = BeliefBook::new();
    let ap_len = r.u32()? as usize;
    for _ in 0..ap_len {
        book.actor_priors
            .insert(ActorId::new(r.string()?), (r.f64()?, r.f64()?));
    }
    let bl_len = r.u32()? as usize;
    for _ in 0..bl_len {
        let id = EventId::new(r.u64()?);
        let claim_key = r.string()?;
        let requested_basis = MemoryBasis::from_code(r.u8()?).ok_or_else(|| {
            EventFormatError::MalformedRecord("invalid requested_basis".to_owned())
        })?;
        let basis = MemoryBasis::from_code(r.u8()?)
            .ok_or_else(|| EventFormatError::MalformedRecord("invalid basis".to_owned()))?;
        let basis_downgraded = r.u8()? != 0;
        let source_actor = r.opt_string()?.map(ActorId::new);
        let ca_len = r.u32()? as usize;
        let mut corroborating_actors = BTreeSet::new();
        for _ in 0..ca_len {
            corroborating_actors.insert(ActorId::new(r.string()?));
        }
        let taint = decode_taint(r.u8()?)?;
        let domain = DomainId::new(r.string()?);
        let audience = decode_audience(r)?;
        let se_len = r.u32()? as usize;
        let mut source_event_ids = Vec::with_capacity(se_len.min(512));
        for _ in 0..se_len {
            source_event_ids.push(EventId::new(r.u64()?));
        }
        let valid_from_ms = Time::from_millis(r.u64()?);
        let valid_to_ms = r.opt_u64()?.map(Time::from_millis);
        let replaced_by = r.opt_u64()?.map(EventId::new);
        let credence = match r.u8()? {
            0 => None,
            1 => Some(decode_beta_credence(r)?),
            t => {
                return Err(EventFormatError::MalformedRecord(format!(
                    "invalid credence tag {t}"
                )));
            }
        };
        let is_mind_inference = r.u8()? != 0;
        let erased = r.u8()? != 0;
        book.beliefs.insert(
            id,
            BeliefRecord {
                belief_id: id,
                claim_key,
                requested_basis,
                basis,
                basis_downgraded,
                source_actor,
                corroborating_actors,
                taint,
                domain,
                audience,
                source_event_ids,
                valid_from_ms,
                valid_to_ms,
                replaced_by,
                credence,
                is_mind_inference,
                erased,
            },
        );
    }
    let ac_len = r.u32()? as usize;
    for _ in 0..ac_len {
        book.active_claims.insert(r.string()?, EventId::new(r.u64()?));
    }
    let sr_len = r.u32()? as usize;
    for _ in 0..sr_len {
        book.source_reliability.insert(
            (ActorId::new(r.string()?), DomainId::new(r.string()?)),
            decode_beta_credence(r)?,
        );
    }
    let ic_len = r.u32()? as usize;
    for _ in 0..ic_len {
        book.item_credences
            .insert(EventId::new(r.u64()?), decode_beta_credence(r)?);
    }
    let conv_len = r.u32()? as usize;
    for _ in 0..conv_len {
        book.conventions.insert(
            (PlaceId::new(r.string()?), r.string()?),
            decode_beta_credence(r)?,
        );
    }
    let le_len = r.u32()? as usize;
    for _ in 0..le_len {
        match r.u8()? {
            1 => {
                let event_id = EventId::new(r.u64()?);
                let timestamp_ms = Time::from_millis(r.u64()?);
                let seq = LogSeq::new(r.u64()?);
                let claim_key = r.string()?;
                let requested_basis = MemoryBasis::from_code(r.u8()?).ok_or_else(|| {
                    EventFormatError::MalformedRecord("invalid requested_basis".to_owned())
                })?;
                let basis = MemoryBasis::from_code(r.u8()?)
                    .ok_or_else(|| EventFormatError::MalformedRecord("invalid basis".to_owned()))?;
                let basis_downgraded = r.u8()? != 0;
                let source_actor = r.opt_string()?.map(ActorId::new);
                let domain = DomainId::new(r.string()?);
                let audience = decode_audience(r)?;
                let se_len = r.u32()? as usize;
                let mut source_event_ids = Vec::with_capacity(se_len.min(512));
                for _ in 0..se_len {
                    source_event_ids.push(EventId::new(r.u64()?));
                }
                let taint = decode_taint(r.u8()?)?;
                let valid_from_ms = Time::from_millis(r.u64()?);
                let valid_to_ms = r.opt_u64()?.map(Time::from_millis);
                let replaced_by = r.opt_u64()?.map(EventId::new);
                let alpha_0 = r.f64()?;
                let beta_0 = r.f64()?;
                let is_mind_inference = r.u8()? != 0;
                book.log_entries.push(BeliefLogEntry::BeliefRecorded {
                    event_id,
                    timestamp_ms,
                    seq,
                    claim_key,
                    requested_basis,
                    basis,
                    basis_downgraded,
                    source_actor,
                    domain,
                    audience,
                    source_event_ids,
                    taint,
                    valid_from_ms,
                    valid_to_ms,
                    replaced_by,
                    alpha_0,
                    beta_0,
                    is_mind_inference,
                });
            }
            2 => {
                let event_id = EventId::new(r.u64()?);
                let timestamp_ms = Time::from_millis(r.u64()?);
                let seq = LogSeq::new(r.u64()?);
                let target_id = EventId::new(r.u64()?);
                let source_actor = r.opt_string()?.map(ActorId::new);
                let domain = DomainId::new(r.string()?);
                let positive = r.f64()?;
                let negative = r.f64()?;
                let is_convention = r.u8()? != 0;
                let place = r.opt_string()?.map(PlaceId::new);
                let pattern_id = r.opt_string()?;
                book.log_entries.push(BeliefLogEntry::EvidenceApplied {
                    event_id,
                    timestamp_ms,
                    seq,
                    target_id,
                    source_actor,
                    domain,
                    positive,
                    negative,
                    is_convention,
                    place,
                    pattern_id,
                });
            }
            tag => {
                return Err(EventFormatError::MalformedRecord(format!(
                    "invalid belief log entry tag {tag}"
                )));
            }
        }
    }
    Ok(book)
}

fn encode_episode_book(w: &mut BinWriter, book: &EpisodeBook) {
    w.u32(book.episodes.len() as u32);
    for (&ep_id, ep) in &book.episodes {
        w.u64(ep_id.as_u64());
        w.str(ep.place.as_str());
        w.str(ep.state.as_str());
        w.u32(ep.event_ids.len() as u32);
        for &eid in &ep.event_ids {
            w.u64(eid.as_u64());
        }
        w.u32(ep.participants.len() as u32);
        for p in &ep.participants {
            w.str(p.as_str());
        }
        w.u64(ep.start_ms.as_millis());
        w.u64(ep.end_ms.as_millis());
        let cents: Vec<TaggedEmbedding> = ep.centroid.iter().cloned().collect();
        encode_tagged_embeddings(w, &cents);
        w.u64(ep.vector_count as u64);
        w.u8(encode_taint(ep.taint));
        w.opt_u64(ep.gist_id.map(EventId::as_u64));
        w.u8(u8::from(ep.dirty));
        w.u8(u8::from(ep.consolidated));
    }
    w.u32(book.active_by_place.len() as u32);
    for (pl, &eid) in &book.active_by_place {
        w.str(pl.as_str());
        w.u64(eid.as_u64());
    }
    w.u32(book.unconsolidated_events.len() as u32);
    for &eid in &book.unconsolidated_events {
        w.u64(eid.as_u64());
    }
    w.u32(book.consolidated_events.len() as u32);
    for &eid in &book.consolidated_events {
        w.u64(eid.as_u64());
    }
}

fn decode_episode_book(r: &mut BinReader<'_>) -> Result<EpisodeBook, EventFormatError> {
    let mut book = EpisodeBook::new();
    let ep_len = r.u32()? as usize;
    for _ in 0..ep_len {
        let episode_id = EventId::new(r.u64()?);
        let place = PlaceId::new(r.string()?);
        let state = EpisodeState::parse(&r.string()?)
            .ok_or_else(|| EventFormatError::MalformedRecord("invalid episode state".to_owned()))?;
        let ev_len = r.u32()? as usize;
        let mut event_ids = Vec::with_capacity(ev_len.min(4096));
        for _ in 0..ev_len {
            event_ids.push(EventId::new(r.u64()?));
        }
        let pa_len = r.u32()? as usize;
        let mut participants = BTreeSet::new();
        for _ in 0..pa_len {
            participants.insert(ActorId::new(r.string()?));
        }
        let start_ms = Time::from_millis(r.u64()?);
        let end_ms = Time::from_millis(r.u64()?);
        let centroid = decode_tagged_embeddings(r)?.into_iter().next();
        let vector_count = r.u64()? as usize;
        let taint = decode_taint(r.u8()?)?;
        let gist_id = r.opt_u64()?.map(EventId::new);
        let dirty = r.u8()? != 0;
        let consolidated = r.u8()? != 0;
        book.episodes.insert(
            episode_id,
            EpisodeRecord {
                episode_id,
                place,
                state,
                event_ids,
                participants,
                start_ms,
                end_ms,
                centroid,
                vector_count,
                taint,
                gist_id,
                dirty,
                consolidated,
            },
        );
    }
    let abp_len = r.u32()? as usize;
    for _ in 0..abp_len {
        book.active_by_place
            .insert(PlaceId::new(r.string()?), EventId::new(r.u64()?));
    }
    let ue_len = r.u32()? as usize;
    for _ in 0..ue_len {
        book.unconsolidated_events.insert(EventId::new(r.u64()?));
    }
    let ce_len = r.u32()? as usize;
    for _ in 0..ce_len {
        book.consolidated_events.insert(EventId::new(r.u64()?));
    }
    Ok(book)
}

fn encode_pending_sleep_run(w: &mut BinWriter, run: Option<&PendingSleepRun>) {
    match run {
        None => w.u8(0),
        Some(run) => {
            w.u8(1);
            w.u64(run.run_id.as_u64());
            w.u64(run.snapshot_ms.as_millis());
            w.u64(run.snapshot_seq.as_u64());
            w.u32(run.covered_event_ids.len() as u32);
            for &id in &run.covered_event_ids {
                w.u64(id.as_u64());
            }
            w.u32(run.pending_episode_ids.len() as u32);
            for &id in &run.pending_episode_ids {
                w.u64(id.as_u64());
            }
            w.u32(run.completed_episode_ids.len() as u32);
            for &id in &run.completed_episode_ids {
                w.u64(id.as_u64());
            }
            w.u32(run.produced_gist_ids.len() as u32);
            for &id in &run.produced_gist_ids {
                w.u64(id.as_u64());
            }
            w.u32(run.episode_taints.len() as u32);
            for (&id, &t) in &run.episode_taints {
                w.u64(id.as_u64());
                w.u8(encode_taint(t));
            }
            w.u64(run.tokens_used);
        }
    }
}

fn decode_pending_sleep_run(
    r: &mut BinReader<'_>,
) -> Result<Option<PendingSleepRun>, EventFormatError> {
    match r.u8()? {
        0 => Ok(None),
        1 => {
            let run_id = EventId::new(r.u64()?);
            let snapshot_ms = Time::from_millis(r.u64()?);
            let snapshot_seq = LogSeq::new(r.u64()?);
            let mut covered_event_ids = Vec::new();
            for _ in 0..(r.u32()? as usize) {
                covered_event_ids.push(EventId::new(r.u64()?));
            }
            let mut pending_episode_ids = Vec::new();
            for _ in 0..(r.u32()? as usize) {
                pending_episode_ids.push(EventId::new(r.u64()?));
            }
            let mut completed_episode_ids = Vec::new();
            for _ in 0..(r.u32()? as usize) {
                completed_episode_ids.push(EventId::new(r.u64()?));
            }
            let mut produced_gist_ids = Vec::new();
            for _ in 0..(r.u32()? as usize) {
                produced_gist_ids.push(EventId::new(r.u64()?));
            }
            let mut episode_taints = BTreeMap::new();
            for _ in 0..(r.u32()? as usize) {
                episode_taints.insert(EventId::new(r.u64()?), decode_taint(r.u8()?)?);
            }
            let tokens_used = r.u64()?;
            Ok(Some(PendingSleepRun {
                run_id,
                snapshot_ms,
                snapshot_seq,
                covered_event_ids,
                pending_episode_ids,
                completed_episode_ids,
                produced_gist_ids,
                episode_taints,
                tokens_used,
            }))
        }
        tag => Err(EventFormatError::MalformedRecord(format!(
            "invalid pending_sleep_run tag {tag}"
        ))),
    }
}

fn encode_skill_book(w: &mut BinWriter, book: &SkillBook) {
    w.u64(book.next_synthetic_id);
    w.u32(book.failure_episodes.len() as u32);
    for f in &book.failure_episodes {
        w.u64(f.episode_id.as_u64());
        w.str(&f.signature_key);
        w.opt_str(f.domain.as_ref().map(DomainId::as_str));
        w.opt_str(f.capability_id.as_ref().map(CapabilityId::as_str));
        w.str(f.place.as_str());
        w.u32(f.actors.len() as u32);
        for a in &f.actors {
            w.str(a.as_str());
        }
        w.u8(encode_taint(f.taint));
        w.u64(f.timestamp_ms.as_millis());
    }
    w.u32(book.success_episodes.len() as u32);
    for s in &book.success_episodes {
        w.u64(s.episode_id.as_u64());
        w.str(&s.signature_key);
        w.opt_str(s.domain.as_ref().map(DomainId::as_str));
        w.opt_str(s.capability_id.as_ref().map(CapabilityId::as_str));
        w.str(s.place.as_str());
        w.u32(s.actors.len() as u32);
        for a in &s.actors {
            w.str(a.as_str());
        }
        w.u8(encode_taint(s.taint));
        w.u64(s.timestamp_ms.as_millis());
    }
    w.u32(book.lessons.len() as u32);
    for (&lid, l) in &book.lessons {
        w.u64(lid.as_u64());
        w.str(&l.pattern);
        w.str(&l.failure_signature);
        w.opt_str(l.domain.as_ref().map(DomainId::as_str));
        w.u32(l.failure_ids.len() as u32);
        for fid in &l.failure_ids {
            w.u64(fid.as_u64());
        }
        w.u32(l.contrastive_success_ids.len() as u32);
        for cid in &l.contrastive_success_ids {
            w.u64(cid.as_u64());
        }
        w.u32(l.actors.len() as u32);
        for a in &l.actors {
            w.str(a.as_str());
        }
        w.f64(l.credence.alpha);
        w.f64(l.credence.beta);
        w.u64(l.credence.last_updated_ms.as_millis());
        w.u8(encode_taint(l.taint));
        w.u64(l.created_at_ms.as_millis());
        w.u64(l.updated_at_ms.as_millis());
        w.u8(u8::from(l.active));
        encode_audience(w, &l.audience);
    }
    w.u32(book.procedures.len() as u32);
    for (&pid, p) in &book.procedures {
        w.u64(pid.as_u64());
        w.str(&p.name);
        w.u32(p.steps.len() as u32);
        for st in &p.steps {
            w.str(st);
        }
        w.u32(p.required_cap_ids.len() as u32);
        for c in &p.required_cap_ids {
            w.str(c.as_str());
        }
        w.str(p.capability_schema_hash.as_str());
        w.opt_str(p.domain.as_ref().map(DomainId::as_str));
        w.u32(p.places.len() as u32);
        for pl in &p.places {
            w.str(pl.as_str());
        }
        w.u32(p.actors.len() as u32);
        for a in &p.actors {
            w.str(a.as_str());
        }
        encode_audience(w, &p.audience);
        w.u8(u8::from(p.owner_signed));
        match &p.embedding {
            Some(emb) => {
                w.u8(1);
                encode_tagged_embedding(w, emb);
            }
            None => w.u8(0),
        }
        w.opt_u64(p.source_goal_id.map(EventId::as_u64));
        w.u32(p.source_event_ids.len() as u32);
        for sid in &p.source_event_ids {
            w.u64(sid.as_u64());
        }
        w.u8(encode_taint(p.taint));
        w.u8(p.state.code());
        w.opt_str(p.retired_reason.as_deref());
        w.opt_u64(p.retired_at_ms.map(Time::as_millis));
        w.u64(p.created_at_ms.as_millis());
        w.u64(p.updated_at_ms.as_millis());
        w.f64(p.alpha);
        w.f64(p.beta);
        w.u64(p.trial_count);
        w.u64(p.success_count);
        match p.cusum_baseline_p0 {
            Some(b) => {
                w.u8(1);
                w.f64(b);
            }
            None => w.u8(0),
        }
        w.f64(p.cusum_stat);
        w.u32(p.warn_count);
    }
}

fn decode_skill_book(r: &mut BinReader<'_>, version: u16) -> Result<SkillBook, EventFormatError> {
    let mut book = SkillBook::new();
    book.next_synthetic_id = r.u64()?;
    let fe_len = r.u32()? as usize;
    for _ in 0..fe_len {
        let episode_id = EventId::new(r.u64()?);
        let signature_key = r.string()?;
        let domain = r.opt_string()?.map(DomainId::new);
        let capability_id = r.opt_string()?.map(CapabilityId::new);
        let place = PlaceId::new(r.string()?);
        let mut actors = Vec::new();
        for _ in 0..(r.u32()? as usize) {
            actors.push(ActorId::new(r.string()?));
        }
        let taint = decode_taint(r.u8()?)?;
        let timestamp_ms = Time::from_millis(r.u64()?);
        book.failure_episodes.push(FailureEpisodeRecord {
            episode_id,
            signature_key,
            domain,
            capability_id,
            place,
            actors,
            taint,
            timestamp_ms,
        });
    }
    let se_len = r.u32()? as usize;
    for _ in 0..se_len {
        let episode_id = EventId::new(r.u64()?);
        let signature_key = r.string()?;
        let domain = r.opt_string()?.map(DomainId::new);
        let capability_id = r.opt_string()?.map(CapabilityId::new);
        let place = PlaceId::new(r.string()?);
        let mut actors = Vec::new();
        for _ in 0..(r.u32()? as usize) {
            actors.push(ActorId::new(r.string()?));
        }
        let taint = decode_taint(r.u8()?)?;
        let timestamp_ms = Time::from_millis(r.u64()?);
        book.success_episodes.push(SuccessEpisodeRecord {
            episode_id,
            signature_key,
            domain,
            capability_id,
            place,
            actors,
            taint,
            timestamp_ms,
        });
    }
    let l_len = r.u32()? as usize;
    for _ in 0..l_len {
        let lesson_id = EventId::new(r.u64()?);
        let pattern = r.string()?;
        let failure_signature = r.string()?;
        let domain = r.opt_string()?.map(DomainId::new);
        let mut failure_ids = Vec::new();
        for _ in 0..(r.u32()? as usize) {
            failure_ids.push(EventId::new(r.u64()?));
        }
        let mut contrastive_success_ids = Vec::new();
        for _ in 0..(r.u32()? as usize) {
            contrastive_success_ids.push(EventId::new(r.u64()?));
        }
        let mut actors = Vec::new();
        for _ in 0..(r.u32()? as usize) {
            actors.push(ActorId::new(r.string()?));
        }
        let alpha = r.f64()?;
        let beta = r.f64()?;
        let last_updated_ms = Time::from_millis(r.u64()?);
        let taint = decode_taint(r.u8()?)?;
        let created_at_ms = Time::from_millis(r.u64()?);
        let updated_at_ms = Time::from_millis(r.u64()?);
        let active = r.u8()? != 0;
        let audience = if version >= 11 {
            decode_audience(r)?
        } else {
            Audience::default()
        };
        book.lessons.insert(
            lesson_id,
            LessonEntry {
                lesson_id,
                pattern,
                failure_signature,
                domain,
                failure_ids,
                contrastive_success_ids,
                actors,
                audience,
                credence: BetaCredence {
                    alpha,
                    beta,
                    alpha_0: 1.0,
                    beta_0: 1.0,
                    last_updated_ms,
                },
                taint,
                created_at_ms,
                updated_at_ms,
                active,
            },
        );
    }
    let p_len = r.u32()? as usize;
    for _ in 0..p_len {
        let procedure_id = EventId::new(r.u64()?);
        let name = r.string()?;
        let mut steps = Vec::new();
        for _ in 0..(r.u32()? as usize) {
            steps.push(r.string()?);
        }
        let mut required_cap_ids = Vec::new();
        for _ in 0..(r.u32()? as usize) {
            required_cap_ids.push(CapabilityId::new(r.string()?));
        }
        let capability_schema_hash = CapabilitySchemaHash::new(r.string()?);
        let domain = r.opt_string()?.map(DomainId::new);
        let mut places = Vec::new();
        for _ in 0..(r.u32()? as usize) {
            places.push(PlaceId::new(r.string()?));
        }
        let mut actors = Vec::new();
        for _ in 0..(r.u32()? as usize) {
            actors.push(ActorId::new(r.string()?));
        }
        let audience = decode_audience(r)?;
        let owner_signed = r.u8()? != 0;
        let embedding = match r.u8()? {
            0 => None,
            1 => Some(decode_tagged_embedding(r)?),
            tag => {
                return Err(EventFormatError::MalformedRecord(format!(
                    "invalid proc emb tag {tag}"
                )));
            }
        };
        let source_goal_id = r.opt_u64()?.map(EventId::new);
        let mut source_event_ids = Vec::new();
        for _ in 0..(r.u32()? as usize) {
            source_event_ids.push(EventId::new(r.u64()?));
        }
        let taint = decode_taint(r.u8()?)?;
        let state = ProcedureLifecycleState::from_code(r.u8()?)
            .ok_or_else(|| EventFormatError::MalformedRecord("invalid proc state".to_owned()))?;
        let retired_reason = r.opt_string()?;
        let retired_at_ms = r.opt_u64()?.map(Time::from_millis);
        let created_at_ms = Time::from_millis(r.u64()?);
        let updated_at_ms = Time::from_millis(r.u64()?);
        let alpha = r.f64()?;
        let beta = r.f64()?;
        let trial_count = r.u64()?;
        let success_count = r.u64()?;
        let cusum_baseline_p0 = match r.u8()? {
            0 => None,
            1 => Some(r.f64()?),
            tag => {
                return Err(EventFormatError::MalformedRecord(format!(
                    "invalid cusum tag {tag}"
                )));
            }
        };
        let cusum_stat = r.f64()?;
        let warn_count = r.u32()?;
        book.procedures.insert(
            procedure_id,
            TrackedProcedure {
                procedure_id,
                name,
                steps,
                required_cap_ids,
                capability_schema_hash,
                domain,
                places,
                actors,
                audience,
                owner_signed,
                embedding,
                source_goal_id,
                source_event_ids,
                taint,
                state,
                retired_reason,
                retired_at_ms,
                created_at_ms,
                updated_at_ms,
                alpha,
                beta,
                trial_count,
                success_count,
                cusum_baseline_p0,
                cusum_stat,
                warn_count,
            },
        );
    }
    Ok(book)
}

fn encode_perception_ledger(w: &mut BinWriter, ledger: &PerceptionLedger) {
    w.u32(ledger.actors.len() as u32);
    for (aid, card) in &ledger.actors {
        w.str(aid.as_str());
        w.f64(card.reliability_alpha);
        w.f64(card.reliability_beta);
        w.u8(u8::from(card.trusted_origin));
        w.u32(card.witnessed.len() as u32);
        for ws in &card.witnessed {
            w.u64(ws.event_id.as_u64());
            w.str(ws.place.as_str());
            w.u64(ws.timestamp_ms.as_millis());
        }
        w.u32(card.open_requests.len() as u32);
        for or in &card.open_requests {
            w.u64(or.expectation_id.as_u64());
            w.u8(encode_expect_kind(or.kind));
            w.u64(or.due_ms.as_millis());
        }
        w.u32(card.told_items.len() as u32);
        for ti in &card.told_items {
            w.u64(ti.belief_id.as_u64());
            w.str(&ti.predicate);
            w.u64(ti.timestamp_ms.as_millis());
            w.u8(u8::from(ti.corroborated));
            w.u8(u8::from(ti.refuted));
        }
        w.u32(card.false_beliefs.len() as u32);
        for fb in &card.false_beliefs {
            w.u64(fb.belief_id.as_u64());
            w.str(&fb.predicate);
            w.u64(fb.flagged_at_ms.as_millis());
        }
    }
    w.u32(ledger.capabilities.len() as u32);
    for (cid, cap) in &ledger.capabilities {
        w.str(cid.as_str());
        w.u64(cap.trials);
        w.u64(cap.successes);
        w.u64(cap.failures);
        w.u32(cap.consec_fail);
        w.f64(cap.alpha);
        w.f64(cap.beta);
        w.u8(u8::from(cap.degraded));
        w.u64(cap.latency_sum_ms.min(u128::from(u64::MAX)) as u64);
        w.u64(cap.latency_samples);
        w.u64(cap.last_updated_ms.as_millis());
    }
    for v in ledger.calibration.bin_incorrect {
        w.u64(v);
    }
    for v in ledger.calibration.bin_correct {
        w.u64(v);
    }
    w.f64(ledger.calibration.brier_sum);
    w.f64(ledger.calibration.signed_error_sum);
    w.u64(ledger.calibration.samples);
    w.f64(ledger.principle_alpha);
    w.f64(ledger.principle_beta);
    w.u64(ledger.contradiction_flags);
    w.f64(ledger.daily_budget_used);
    w.f64(ledger.daily_budget_limit);
    w.u64(ledger.recalibration_count);
    w.opt_u64(ledger.last_recalibration_ms.map(Time::as_millis));
    w.u64(ledger.last_capability_schema_epoch);
}

fn decode_perception_ledger(r: &mut BinReader<'_>) -> Result<PerceptionLedger, EventFormatError> {
    let mut ledger = PerceptionLedger::new();
    let act_len = r.u32()? as usize;
    for _ in 0..act_len {
        let actor_id = ActorId::new(r.string()?);
        let reliability_alpha = r.f64()?;
        let reliability_beta = r.f64()?;
        let trusted_origin = r.u8()? != 0;
        let mut witnessed = Vec::new();
        for _ in 0..(r.u32()? as usize) {
            witnessed.push(WitnessedFactSummary {
                event_id: EventId::new(r.u64()?),
                place: PlaceId::new(r.string()?),
                timestamp_ms: Time::from_millis(r.u64()?),
            });
        }
        let mut open_requests = Vec::new();
        for _ in 0..(r.u32()? as usize) {
            open_requests.push(OpenRequestItem {
                expectation_id: EventId::new(r.u64()?),
                kind: decode_expect_kind(r.u8()?)?,
                due_ms: Time::from_millis(r.u64()?),
            });
        }
        let mut told_items = Vec::new();
        for _ in 0..(r.u32()? as usize) {
            told_items.push(ToldLedgerItem {
                belief_id: EventId::new(r.u64()?),
                predicate: r.string()?,
                timestamp_ms: Time::from_millis(r.u64()?),
                corroborated: r.u8()? != 0,
                refuted: r.u8()? != 0,
            });
        }
        let mut false_beliefs = Vec::new();
        for _ in 0..(r.u32()? as usize) {
            false_beliefs.push(FalseBeliefFlag {
                belief_id: EventId::new(r.u64()?),
                predicate: r.string()?,
                flagged_at_ms: Time::from_millis(r.u64()?),
            });
        }
        ledger.actors.insert(
            actor_id.clone(),
            ActorCard {
                actor_id,
                reliability_alpha,
                reliability_beta,
                trusted_origin,
                witnessed,
                open_requests,
                told_items,
                false_beliefs,
            },
        );
    }
    let cap_len = r.u32()? as usize;
    for _ in 0..cap_len {
        let capability_id = CapabilityId::new(r.string()?);
        let trials = r.u64()?;
        let successes = r.u64()?;
        let failures = r.u64()?;
        let consec_fail = r.u32()?;
        let alpha = r.f64()?;
        let beta = r.f64()?;
        let degraded = r.u8()? != 0;
        let latency_sum_ms = u128::from(r.u64()?);
        let latency_samples = r.u64()?;
        let last_updated_ms = Time::from_millis(r.u64()?);
        ledger.capabilities.insert(
            capability_id.clone(),
            CapabilityHealthStats {
                capability_id,
                trials,
                successes,
                failures,
                consec_fail,
                alpha,
                beta,
                degraded,
                latency_sum_ms,
                latency_samples,
                last_updated_ms,
            },
        );
    }
    let mut bin_incorrect = [0_u64; 5];
    for s in &mut bin_incorrect {
        *s = r.u64()?;
    }
    let mut bin_correct = [0_u64; 5];
    for s in &mut bin_correct {
        *s = r.u64()?;
    }
    ledger.calibration = SelfCalibrationStats {
        bin_incorrect,
        bin_correct,
        brier_sum: r.f64()?,
        signed_error_sum: r.f64()?,
        samples: r.u64()?,
    };
    ledger.principle_alpha = r.f64()?;
    ledger.principle_beta = r.f64()?;
    ledger.contradiction_flags = r.u64()?;
    ledger.daily_budget_used = r.f64()?;
    ledger.daily_budget_limit = r.f64()?;
    ledger.recalibration_count = r.u64()?;
    ledger.last_recalibration_ms = r.opt_u64()?.map(Time::from_millis);
    ledger.last_capability_schema_epoch = r.u64()?;
    Ok(ledger)
}

fn decode_fixed_32(r: &mut BinReader<'_>) -> Result<[u8; 32], EventFormatError> {
    let bytes = r.bytes()?;
    if bytes.len() != 32 {
        return Err(EventFormatError::MalformedRecord(format!(
            "expected 32-byte digest, got {}",
            bytes.len()
        )));
    }
    let mut out = [0_u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn encode_capability_descriptors(
    w: &mut BinWriter,
    caps: &BTreeMap<CapabilityId, CapabilityDescriptor>,
) {
    w.u32(caps.len() as u32);
    for (id, cap) in caps {
        w.str(id.as_str());
        w.str(&cap.description);
        w.u32(cap.places.len() as u32);
        for p in &cap.places {
            w.str(p.as_str());
        }
        w.u8(u8::from(cap.flags.communicative));
        w.u8(u8::from(cap.flags.identity_changing));
        w.u8(u8::from(cap.flags.irreversible));
        w.u64(cap.typical_latency_ms);
        w.str(cap.capability_schema_hash.as_str());
        encode_opt_sensed_effect(w, cap.sensed_effect.as_ref());
    }
}

fn decode_capability_descriptors(
    r: &mut BinReader<'_>,
) -> Result<BTreeMap<CapabilityId, CapabilityDescriptor>, EventFormatError> {
    let len = r.u32()? as usize;
    let mut out = BTreeMap::new();
    for _ in 0..len {
        let id = CapabilityId::new(r.string()?);
        let description = r.string()?;
        let plen = r.u32()? as usize;
        let mut places = Vec::with_capacity(plen.min(128));
        for _ in 0..plen {
            places.push(PlaceId::new(r.string()?));
        }
        let communicative = r.u8()? != 0;
        let identity_changing = r.u8()? != 0;
        let irreversible = r.u8()? != 0;
        let typical_latency_ms = r.u64()?;
        let capability_schema_hash = CapabilitySchemaHash::new(r.string()?);
        let sensed_effect = decode_opt_sensed_effect(r)?;
        out.insert(
            id.clone(),
            CapabilityDescriptor {
                id,
                description,
                places,
                flags: CapabilityFlags {
                    communicative,
                    identity_changing,
                    irreversible,
                },
                typical_latency_ms,
                capability_schema_hash,
                sensed_effect,
            },
        );
    }
    Ok(out)
}

fn encode_activation_trace(w: &mut BinWriter, tr: &ActivationTrace) {
    w.u64(tr.count);
    w.u64(tr.first_ms.as_millis());
    w.u64(tr.last_ms.as_millis());
}

fn decode_activation_trace(r: &mut BinReader<'_>) -> Result<ActivationTrace, EventFormatError> {
    Ok(ActivationTrace {
        count: r.u64()?,
        first_ms: Time::from_millis(r.u64()?),
        last_ms: Time::from_millis(r.u64()?),
    })
}

fn encode_audience(w: &mut BinWriter, aud: &Audience) {
    w.u32(aud.members.len() as u32);
    for m in &aud.members {
        w.str(m.as_str());
    }
}

fn decode_audience(r: &mut BinReader<'_>) -> Result<Audience, EventFormatError> {
    let len = r.u32()? as usize;
    let mut members = Vec::with_capacity(len.min(1024));
    for _ in 0..len {
        members.push(ActorId::new(r.string()?));
    }
    Ok(Audience::new(members))
}

fn encode_tagged_embeddings(w: &mut BinWriter, embs: &[TaggedEmbedding]) {
    w.u32(embs.len() as u32);
    for emb in embs {
        encode_tagged_embedding(w, emb);
    }
}

fn encode_tagged_embedding(w: &mut BinWriter, emb: &TaggedEmbedding) {
    w.str(&emb.tag.embed_model_ver);
    w.u32(emb.tag.dim_d);
    w.str(&emb.tag.prompt_regime_id);
    w.u32(emb.values.len() as u32);
    for &v in &emb.values {
        w.f32(v);
    }
}

fn decode_tagged_embeddings(
    r: &mut BinReader<'_>,
) -> Result<Vec<TaggedEmbedding>, EventFormatError> {
    let len = r.u32()? as usize;
    let mut out = Vec::with_capacity(len.min(64));
    for _ in 0..len {
        out.push(decode_tagged_embedding(r)?);
    }
    Ok(out)
}

fn decode_tagged_embedding(r: &mut BinReader<'_>) -> Result<TaggedEmbedding, EventFormatError> {
    let embed_model_ver = r.string()?;
    let dim_d = r.u32()?;
    let prompt_regime_id = r.string()?;
    let vlen = r.u32()? as usize;
    let mut values = Vec::with_capacity(vlen.min(4096));
    for _ in 0..vlen {
        values.push(r.f32()?);
    }
    Ok(TaggedEmbedding {
        tag: EmbeddingTag {
            embed_model_ver,
            dim_d,
            prompt_regime_id,
        },
        values,
    })
}

pub(crate) fn encode_body_status(status: BodyStatus) -> u8 {
    match status {
        BodyStatus::Ready => 1,
        BodyStatus::Busy => 2,
        BodyStatus::Degraded => 3,
        BodyStatus::Stopped => 4,
    }
}

pub(crate) fn decode_body_status(code: u8) -> Result<BodyStatus, EventFormatError> {
    match code {
        1 => Ok(BodyStatus::Ready),
        2 => Ok(BodyStatus::Busy),
        3 => Ok(BodyStatus::Degraded),
        4 => Ok(BodyStatus::Stopped),
        other => Err(EventFormatError::MalformedRecord(format!(
            "invalid body status code {other}"
        ))),
    }
}

pub(crate) fn parse_body_status(bytes: &[u8]) -> Result<BodyStatus, EventFormatError> {
    if bytes.is_empty() {
        return Err(EventFormatError::MalformedRecord(
            "empty body status payload".to_owned(),
        ));
    }
    decode_body_status(bytes[0])
}

pub(crate) fn encode_actor_facts_from_msg(facts: &ActorFacts) -> Vec<u8> {
    encode_actor_facts(&ActorFactsEntry {
        alpha_0: facts.alpha_0,
        beta_0: facts.beta_0,
        teacher_weight: facts.teacher_weight,
        trusted_origin: facts.trusted_origin,
    })
}

fn encode_actor_facts(entry: &ActorFactsEntry) -> Vec<u8> {
    let mut w = BinWriter::new();
    w.f64(entry.alpha_0);
    w.f64(entry.beta_0);
    w.f64(entry.teacher_weight);
    w.u8(u8::from(entry.trusted_origin));
    w.finish()
}

pub(crate) fn decode_actor_facts(bytes: &[u8]) -> Result<ActorFactsEntry, EventFormatError> {
    let mut r = BinReader::new(bytes);
    Ok(ActorFactsEntry {
        alpha_0: r.f64()?,
        beta_0: r.f64()?,
        teacher_weight: r.f64()?,
        trusted_origin: r.u8()? != 0,
    })
}

pub(crate) fn encode_identity_payload(signed: &OwnerSigned<OpaquePayload>) -> Vec<u8> {
    let mut w = BinWriter::new();
    w.str(signed.signer.as_str());
    w.u64(signed.signed_at_ms.as_millis());
    w.str(&signed.signature);
    w.bytes(signed.payload.as_bytes());
    w.finish()
}

pub(crate) fn decode_identity_payload(
    bytes: &[u8],
) -> Result<OwnerSigned<OpaquePayload>, EventFormatError> {
    let mut r = BinReader::new(bytes);
    let signer = ActorId::new(r.string()?);
    let signed_at_ms = Time::from_millis(r.u64()?);
    let signature = r.string()?;
    let payload = OpaquePayload::from_bytes(r.bytes()?);
    Ok(OwnerSigned {
        payload,
        signer,
        signed_at_ms,
        signature,
    })
}

fn encode_receipt_outcome(o: ReceiptOutcome) -> u8 {
    match o {
        ReceiptOutcome::Done => 1,
        ReceiptOutcome::Refused => 2,
        ReceiptOutcome::Held => 3,
        ReceiptOutcome::Failed => 4,
        ReceiptOutcome::Unknown => 5,
    }
}

fn decode_receipt_outcome(code: u8) -> Result<ReceiptOutcome, EventFormatError> {
    match code {
        1 => Ok(ReceiptOutcome::Done),
        2 => Ok(ReceiptOutcome::Refused),
        3 => Ok(ReceiptOutcome::Held),
        4 => Ok(ReceiptOutcome::Failed),
        5 => Ok(ReceiptOutcome::Unknown),
        other => Err(EventFormatError::MalformedRecord(format!(
            "invalid receipt outcome code {other}"
        ))),
    }
}

fn encode_term_reason(r: TerminationReason) -> u8 {
    match r {
        TerminationReason::Normal => 1,
        TerminationReason::CanceledByCore => 2,
        TerminationReason::PreemptedExternal => 3,
        TerminationReason::TimedOut => 4,
    }
}

fn decode_term_reason(code: u8) -> Result<TerminationReason, EventFormatError> {
    match code {
        1 => Ok(TerminationReason::Normal),
        2 => Ok(TerminationReason::CanceledByCore),
        3 => Ok(TerminationReason::PreemptedExternal),
        4 => Ok(TerminationReason::TimedOut),
        other => Err(EventFormatError::MalformedRecord(format!(
            "invalid termination reason code {other}"
        ))),
    }
}

fn encode_open_waits(w: &mut BinWriter, waits: &BTreeMap<EventId, OpenWait>) {
    w.u32(waits.len() as u32);
    for (&eid, wait) in waits {
        w.u64(eid.as_u64());
        w.u64(wait.target_event_id.as_u64());
        w.str(wait.place.as_str());
        w.str(wait.sender.as_str());
        w.str(wait.medium.as_str());
        encode_audience(w, &wait.audience);
        w.u64(wait.opened_at_ms.as_millis());
        w.u64(wait.due_ms.as_millis());
    }
}

fn decode_open_waits(
    r: &mut BinReader<'_>,
) -> Result<BTreeMap<EventId, OpenWait>, EventFormatError> {
    let len = r.u32()? as usize;
    let mut out = BTreeMap::new();
    for _ in 0..len {
        let expectation_id = EventId::new(r.u64()?);
        let target_event_id = EventId::new(r.u64()?);
        let place = PlaceId::new(r.string()?);
        let sender = ActorId::new(r.string()?);
        let medium = ChannelId::new(r.string()?);
        let audience = decode_audience(r)?;
        let opened_at_ms = Time::from_millis(r.u64()?);
        let due_ms = Time::from_millis(r.u64()?);
        out.insert(
            expectation_id,
            OpenWait {
                expectation_id,
                target_event_id,
                place,
                sender,
                medium,
                audience,
                opened_at_ms,
                due_ms,
            },
        );
    }
    Ok(out)
}

fn encode_memory_entries(w: &mut BinWriter, entries: &BTreeMap<EventId, MemoryEntry>) {
    w.u32(entries.len() as u32);
    for (&eid, entry) in entries {
        w.u64(eid.as_u64());
        w.u32(entry.source_ids.len() as u32);
        for &sid in &entry.source_ids {
            w.u64(sid.as_u64());
        }
        w.u8(u8::from(entry.is_gist));
        w.u64(entry.timestamp_ms.as_millis());
        w.str(entry.place.as_str());
        w.str(entry.actor.as_str());
        w.str(entry.origin_actor.actor_id().as_str());
        w.u8(entry.basis.code());
        w.u8(encode_taint(entry.taint));
        encode_audience(w, &entry.audience);
        w.u32(entry.attachments.len() as u32);
        for att in &entry.attachments {
            w.str(&att.handle_id);
            w.u8(match att.modality_slot {
                ModalitySlot::Image => 1,
                ModalitySlot::AudioVoice => 2,
                ModalitySlot::Video => 3,
                ModalitySlot::SensorBlob => 4,
            });
            w.u64(att.byte_len);
            w.u64(att.expires_at_ms.as_millis());
            w.str(&att.embed_model_ver);
        }
        encode_activation_trace(w, &entry.activation);
        w.f64(entry.importance.i_reason);
        w.u8(u8::from(entry.importance.linked_to_commitment));
        w.f64(entry.importance.i_surprise);
        encode_tagged_embeddings(w, &entry.embeddings);
        w.u32(entry.concepts.len() as u32);
        for c in &entry.concepts {
            w.str(c);
        }
        w.opt_u64(entry.superseded_by.map(EventId::as_u64));
        w.u8(u8::from(entry.erased));
    }
}

fn decode_memory_entries(
    r: &mut BinReader<'_>,
) -> Result<BTreeMap<EventId, MemoryEntry>, EventFormatError> {
    let len = r.u32()? as usize;
    let mut out = BTreeMap::new();
    for _ in 0..len {
        let event_id = EventId::new(r.u64()?);
        let src_len = r.u32()? as usize;
        let mut source_ids = Vec::with_capacity(src_len.min(1024));
        for _ in 0..src_len {
            source_ids.push(EventId::new(r.u64()?));
        }
        let is_gist = r.u8()? != 0;
        let timestamp_ms = Time::from_millis(r.u64()?);
        let place = PlaceId::new(r.string()?);
        let actor = ActorId::new(r.string()?);
        let origin_actor = OriginActor::new(ActorId::new(r.string()?));
        let basis_code = r.u8()?;
        let basis = MemoryBasis::from_code(basis_code).ok_or_else(|| {
            EventFormatError::MalformedRecord(format!("invalid memory basis code {basis_code}"))
        })?;
        let taint = decode_taint(r.u8()?)?;
        let audience = decode_audience(r)?;
        let att_len = r.u32()? as usize;
        let mut attachments = Vec::with_capacity(att_len.min(256));
        for _ in 0..att_len {
            let handle_id = r.string()?;
            let modality_slot = match r.u8()? {
                1 => ModalitySlot::Image,
                2 => ModalitySlot::AudioVoice,
                3 => ModalitySlot::Video,
                4 => ModalitySlot::SensorBlob,
                tag => {
                    return Err(EventFormatError::MalformedRecord(format!(
                        "invalid attachment modality tag {tag}"
                    )));
                }
            };
            let byte_len = r.u64()?;
            let expires_at_ms = Time::from_millis(r.u64()?);
            let embed_model_ver = r.string()?;
            attachments.push(AttachmentHandle {
                handle_id,
                modality_slot,
                byte_len,
                expires_at_ms,
                embed_model_ver,
            });
        }
        let activation = decode_activation_trace(r)?;
        let i_reason = r.f64()?;
        let linked_to_commitment = r.u8()? != 0;
        let i_surprise = r.f64()?;
        let embeddings = decode_tagged_embeddings(r)?;
        let con_len = r.u32()? as usize;
        let mut concepts = Vec::with_capacity(con_len.min(256));
        for _ in 0..con_len {
            concepts.push(r.string()?);
        }
        let superseded_by = r.opt_u64()?.map(EventId::new);
        let erased = r.u8()? != 0;
        out.insert(
            event_id,
            MemoryEntry {
                event_id,
                source_ids,
                is_gist,
                timestamp_ms,
                place,
                actor,
                origin_actor,
                basis,
                taint,
                audience,
                attachments,
                activation,
                importance: ImportanceInputs {
                    i_reason,
                    linked_to_commitment,
                    i_surprise,
                },
                embeddings,
                concepts,
                superseded_by,
                erased,
            },
        );
    }
    Ok(out)
}

fn encode_declass_token(w: &mut BinWriter, token: &DeclassToken) {
    w.str(&token.token_id);
    w.str(token.issuer.as_str());
    w.u32(token.scope.len() as u32);
    for id in &token.scope {
        w.u64(id.as_u64());
    }
    w.u64(token.expiry.as_millis());
    w.opt_str(token.target_place.as_ref().map(PlaceId::as_str));
    encode_audience(w, &token.target_audience);
    w.str(&token.signature);
}

fn decode_declass_token(r: &mut BinReader<'_>) -> Result<DeclassToken, EventFormatError> {
    let token_id = r.string()?;
    let issuer = ActorId::new(r.string()?);
    let scope_len = r.u32()? as usize;
    let mut scope = Vec::with_capacity(scope_len.min(1024));
    for _ in 0..scope_len {
        scope.push(EventId::new(r.u64()?));
    }
    let expiry = Time::from_millis(r.u64()?);
    let target_place = r.opt_string()?.map(PlaceId::new);
    let target_audience = decode_audience(r)?;
    let signature = r.string()?;
    Ok(DeclassToken {
        token_id,
        issuer,
        scope,
        expiry,
        target_place,
        target_audience,
        signature,
    })
}

fn encode_declass_tokens(w: &mut BinWriter, tokens: &[DeclassToken]) {
    w.u32(tokens.len() as u32);
    for token in tokens {
        encode_declass_token(w, token);
    }
}

fn decode_declass_tokens(r: &mut BinReader<'_>) -> Result<Vec<DeclassToken>, EventFormatError> {
    let len = r.u32()? as usize;
    let mut out = Vec::with_capacity(len.min(1024));
    for _ in 0..len {
        out.push(decode_declass_token(r)?);
    }
    Ok(out)
}

fn encode_logged_declass_tokens(w: &mut BinWriter, tokens: &[OwnerSigned<DeclassToken>]) {
    w.u32(tokens.len() as u32);
    for signed in tokens {
        w.str(signed.signer.as_str());
        w.u64(signed.signed_at_ms.as_millis());
        w.str(&signed.signature);
        encode_declass_token(w, &signed.payload);
    }
}

fn decode_logged_declass_tokens(
    r: &mut BinReader<'_>,
) -> Result<Vec<OwnerSigned<DeclassToken>>, EventFormatError> {
    let len = r.u32()? as usize;
    let mut out = Vec::with_capacity(len.min(1024));
    for _ in 0..len {
        let signer = ActorId::new(r.string()?);
        let signed_at_ms = Time::from_millis(r.u64()?);
        let signature = r.string()?;
        let payload = decode_declass_token(r)?;
        out.push(OwnerSigned {
            payload,
            signer,
            signed_at_ms,
            signature,
        });
    }
    Ok(out)
}
