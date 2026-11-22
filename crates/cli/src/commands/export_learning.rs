use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;

use monoagens_core::{EventDetail, EventRecord, MemStore, ReplayMode, Store};
use monoagens_protocol::{ActorId, Audience, PlaceId};
use serde_json::json;

use crate::ExportLearningArgs;
use crate::common::{load_paths, open_store_readonly, write_json_pretty};

fn is_private_audience(aud: &Audience, soul_id: &ActorId) -> bool {
    if aud.is_empty() {
        return true;
    }
    let non_soul_count = aud.members.iter().filter(|a| *a != soul_id).count();
    non_soul_count <= 1
}

fn format_audience_label(aud: &Audience) -> String {
    if aud.is_empty() {
        return "empty".to_owned();
    }
    let members: Vec<&str> = aud.members.iter().map(ActorId::as_str).collect();
    members.join(",")
}

fn place_is_private(
    place: &PlaceId,
    participants: Option<&BTreeSet<ActorId>>,
    soul_id: &ActorId,
) -> bool {
    match participants {
        Some(set) if !set.is_empty() => set.iter().filter(|a| *a != soul_id).count() <= 1,
        _ => place.as_str().starts_with("dm:"),
    }
}

pub(crate) fn cmd_export_learning(
    config_path: Option<&Path>,
    json_mode: bool,
    args: ExportLearningArgs,
    out: &mut dyn Write,
) -> Result<u8, String> {
    use std::fmt::Write as _;

    let (_cfg_opt, paths) = load_paths(config_path)?;
    let events: Vec<EventRecord> = if let Some(log_path) = &args.log_json {
        let raw = fs::read_to_string(log_path)
            .map_err(|e| format!("failed to read log JSON '{}': {e}", log_path.display()))?;
        serde_json::from_str(&raw)
            .map_err(|e| format!("failed to parse log JSON '{}': {e}", log_path.display()))?
    } else {
        let data_dir = args.data_dir.clone().unwrap_or(paths.data_dir);
        let store = open_store_readonly(&data_dir)?;
        store
            .read_in_order(None, usize::MAX)
            .map_err(|e| format!("failed to read store log: {e}"))?
    };

    let mut mem = MemStore::new();
    for rec in &events {
        mem.append(rec.clone())
            .map_err(|e| format!("offline replay store build failed: {e}"))?;
    }
    let report = monoagens_core::replay_log(&mem, ReplayMode::Unpinned, None)
        .map_err(|e| format!("offline replay failed: {e}"))?;
    let state = &report.state;
    let soul_id = &state.soul_actor_id;

    let model_blake3 = events
        .iter()
        .rev()
        .find_map(|ev| match &ev.detail {
            Some(EventDetail::Decision {
                trace: Some(tr), ..
            }) => tr.model_blake3.map(|h| monoagens_core::blake3::to_hex(&h)),
            _ => None,
        })
        .unwrap_or_else(|| "none".to_owned());

    let place_filter = args.place.as_deref().map(PlaceId::new);
    let mut body = String::new();

    // Section 2: Per-place adaptation equalizer.
    let mut adapt_count = 0usize;
    body.push_str("## 2. Per-Place Adaptation Equalizer\n\n");
    for (place, ad) in &state.place_adapt {
        if let Some(pf) = &place_filter
            && place != pf
        {
            continue;
        }
        let participants = state.place_participants.get(place);
        let is_priv = place_is_private(place, participants, soul_id);
        if is_priv && !args.include_private {
            continue;
        }
        let aud_label = participants.map_or_else(
            || if is_priv { "private" } else { "group" }.to_owned(),
            |set| {
                set.iter()
                    .map(ActorId::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            },
        );
        let _ = writeln!(
            body,
            "- **place={}** audience=[{}] logit_offsets=[{:.4}, {:.4}, {:.4}, {:.4}] film_scale_deltas=[{:.4}, {:.4}] film_shift_deltas=[{:.4}, {:.4}] arm_pulls={:?} last_updated_ms={}",
            place.as_str(),
            aud_label,
            ad.logit_offsets[0],
            ad.logit_offsets[1],
            ad.logit_offsets[2],
            ad.logit_offsets[3],
            ad.film_scale_deltas[0],
            ad.film_scale_deltas[1],
            ad.film_shift_deltas[0],
            ad.film_shift_deltas[1],
            ad.arm_pulls,
            ad.last_updated_ms
                .map_or_else(|| "none".to_owned(), |t| t.as_millis().to_string()),
        );
        adapt_count += 1;
    }
    if adapt_count == 0 {
        body.push_str("- none\n");
    }
    body.push('\n');

    // Section 3: Learned procedures.
    let mut proc_count = 0usize;
    body.push_str("## 3. Learned Procedures\n\n");
    for (pid, proc_rec) in &state.skill_book.procedures {
        if let Some(pf) = &place_filter
            && !proc_rec.places.is_empty()
            && !proc_rec.places.contains(pf)
        {
            continue;
        }
        if !args.include_private && is_private_audience(&proc_rec.audience, soul_id) {
            continue;
        }
        let caps: Vec<&str> = proc_rec
            .required_cap_ids
            .iter()
            .map(|c| c.as_str())
            .collect();
        let _ = writeln!(
            body,
            "- **procedure_id={}** name={} state={:?} reliability={:.4} trials={} successes={} cusum={:.4} retired_reason={} required_caps=[{}] steps=[{}] audience=[{}]",
            pid.as_u64(),
            proc_rec.name,
            proc_rec.state,
            proc_rec.posterior_mean(),
            proc_rec.trial_count,
            proc_rec.success_count,
            proc_rec.cusum_stat,
            proc_rec.retired_reason.as_deref().unwrap_or("none"),
            caps.join(","),
            proc_rec.steps.join(" -> "),
            format_audience_label(&proc_rec.audience),
        );
        proc_count += 1;
    }
    if proc_count == 0 {
        body.push_str("- none\n");
    }
    body.push('\n');

    // Section 4: Contrastive lessons.
    let mut lesson_count = 0usize;
    body.push_str("## 4. Contrastive Lessons\n\n");
    for (lid, lesson) in &state.skill_book.lessons {
        if !args.include_private && is_private_audience(&lesson.audience, soul_id) {
            continue;
        }
        if let Some(pf) = &place_filter {
            let matches_place = lesson
                .failure_ids
                .iter()
                .chain(lesson.contrastive_success_ids.iter())
                .filter_map(|id| state.memory_entries.get(id))
                .any(|m| &m.place == pf);
            if !matches_place {
                continue;
            }
        }
        let fids: Vec<String> = lesson
            .failure_ids
            .iter()
            .map(|id| id.as_u64().to_string())
            .collect();
        let cids: Vec<String> = lesson
            .contrastive_success_ids
            .iter()
            .map(|id| id.as_u64().to_string())
            .collect();
        let _ = writeln!(
            body,
            "- **lesson_id={}** pattern={} signature={} domain={} credence={:.4} active={} audience=[{}] failure_ids=[{}] contrastive_success_ids=[{}]",
            lid.as_u64(),
            lesson.pattern,
            lesson.failure_signature,
            lesson.domain.as_ref().map_or("none", |d| d.as_str()),
            lesson.credence.expected(),
            lesson.active,
            format_audience_label(&lesson.audience),
            fids.join(","),
            cids.join(","),
        );
        lesson_count += 1;
    }
    if lesson_count == 0 {
        body.push_str("- none\n");
    }
    body.push('\n');

    // Section 5: Beliefs & conventions.
    let mut belief_count = 0usize;
    body.push_str("## 5. Beliefs & Conventions\n\n");
    for (bid, belief) in &state.belief_book.beliefs {
        if belief.erased {
            continue;
        }
        if !args.include_private && is_private_audience(&belief.audience, soul_id) {
            continue;
        }
        if let Some(pf) = &place_filter {
            let matches_place = belief
                .source_event_ids
                .iter()
                .filter_map(|id| state.memory_entries.get(id))
                .any(|m| &m.place == pf);
            if !matches_place {
                continue;
            }
        }
        let cred = belief
            .credence
            .as_ref()
            .map_or_else(|| "none".to_owned(), |c| format!("{:.4}", c.expected()));
        let _ = writeln!(
            body,
            "- **belief_id={}** claim_key={} basis={:?} credence={} domain={} taint={:?} superseded_by={} audience=[{}]",
            bid.as_u64(),
            belief.claim_key,
            belief.basis,
            cred,
            belief.domain.as_str(),
            belief.taint,
            belief
                .replaced_by
                .map_or_else(|| "none".to_owned(), |id| id.as_u64().to_string()),
            format_audience_label(&belief.audience),
        );
        belief_count += 1;
    }
    for ((place, pattern_id), cred) in &state.belief_book.conventions {
        if let Some(pf) = &place_filter
            && place != pf
        {
            continue;
        }
        let participants = state.place_participants.get(place);
        if place_is_private(place, participants, soul_id) && !args.include_private {
            continue;
        }
        let _ = writeln!(
            body,
            "- **convention** place={} pattern_id={} credence={:.4}",
            place.as_str(),
            pattern_id,
            cred.expected(),
        );
        belief_count += 1;
    }
    if belief_count == 0 {
        body.push_str("- none\n");
    }
    body.push('\n');

    // Section 6: Consolidated episodes & gists.
    let mut episode_count = 0usize;
    body.push_str("## 6. Consolidated Episodes & Gists\n\n");
    for (ep_id, ep) in &state.episode_book.episodes {
        if let Some(pf) = &place_filter
            && &ep.place != pf
        {
            continue;
        }
        let gist_entry = ep.gist_id.and_then(|gid| state.memory_entries.get(&gid));
        if gist_entry.is_some_and(|m| m.erased) {
            continue;
        }
        let aud = gist_entry
            .map(|m| m.audience.clone())
            .unwrap_or_else(|| Audience::new(ep.participants.iter().cloned()));
        if !args.include_private && is_private_audience(&aud, soul_id) {
            continue;
        }
        let ev_ids: Vec<String> = ep.event_ids.iter().map(|id| id.as_u64().to_string()).collect();
        let _ = writeln!(
            body,
            "- **episode_id={}** place={} state={} events=[{}] gist_id={} taint={:?} audience=[{}]",
            ep_id.as_u64(),
            ep.place.as_str(),
            ep.state.as_str(),
            ev_ids.join(","),
            ep.gist_id
                .map_or_else(|| "none".to_owned(), |id| id.as_u64().to_string()),
            ep.taint,
            format_audience_label(&aud),
        );
        episode_count += 1;
    }
    for (mid, entry) in &state.memory_entries {
        if !entry.is_gist || entry.erased {
            continue;
        }
        if let Some(pf) = &place_filter
            && &entry.place != pf
        {
            continue;
        }
        if !args.include_private && is_private_audience(&entry.audience, soul_id) {
            continue;
        }
        let src_ids: Vec<String> = entry
            .source_ids
            .iter()
            .map(|id| id.as_u64().to_string())
            .collect();
        let _ = writeln!(
            body,
            "- **gist_id={}** place={} sources=[{}] concepts=[{}] taint={:?} audience=[{}]",
            mid.as_u64(),
            entry.place.as_str(),
            src_ids.join(","),
            entry.concepts.join(","),
            entry.taint,
            format_audience_label(&entry.audience),
        );
        episode_count += 1;
    }
    if episode_count == 0 {
        body.push_str("- none\n");
    }

    let export_blake3 =
        monoagens_core::blake3::to_hex(&monoagens_core::blake3::hash(body.as_bytes()));
    let mut doc = String::new();
    doc.push_str("# MonoAgens Learned State Export\n\n");
    doc.push_str("## 1. Audit & Reproducibility\n\n");
    let _ = writeln!(doc, "- `log_seq`: {}", state.last_seq.as_u64());
    let _ = writeln!(doc, "- `state_epoch`: {}", state.epoch);
    let _ = writeln!(doc, "- `model_blake3`: {model_blake3}");
    let _ = writeln!(
        doc,
        "- `online_adaptation`: {}",
        report.active_config.online_adaptation
    );
    let _ = writeln!(doc, "- `include_private`: {}", args.include_private);
    let _ = writeln!(
        doc,
        "- `place_filter`: {}",
        args.place.as_deref().unwrap_or("all")
    );
    let _ = writeln!(doc, "- `export_blake3`: {export_blake3}\n");
    doc.push_str(&body);

    if let Some(parent) = args.out.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create export dir: {e}"))?;
    }
    fs::write(&args.out, doc.as_bytes()).map_err(|e| {
        format!(
            "failed to write export markdown '{}': {e}",
            args.out.display()
        )
    })?;

    if json_mode {
        write_json_pretty(
            out,
            &json!({
                "status": "ok",
                "out": args.out.display().to_string(),
                "export_blake3": export_blake3,
                "log_seq": state.last_seq.as_u64(),
                "state_epoch": state.epoch,
                "model_blake3": model_blake3,
                "online_adaptation": report.active_config.online_adaptation,
                "include_private": args.include_private,
                "counts": {
                    "place_adapt": adapt_count,
                    "procedures": proc_count,
                    "lessons": lesson_count,
                    "beliefs": belief_count,
                    "episodes_and_gists": episode_count,
                },
            }),
        )?;
    } else {
        writeln!(
            out,
            "exported learned state to {} (blake3={} seq={} epoch={})",
            args.out.display(),
            export_blake3,
            state.last_seq.as_u64(),
            state.epoch
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(0)
}
