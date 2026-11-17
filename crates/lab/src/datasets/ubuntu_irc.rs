//! `ubuntu_irc` (`jkkummerfeld/irc-disentanglement`) converter & `thread_f1` + `parent_speaker_p1` scorer.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use monoagens_protocol::{ActorId, Audience, PlaceId, VisibleScope};

use crate::LabError;
use crate::datasets::framework::{
    ConvertedDataset, Converter, ExternalItem, ExternalPrediction, ExternalProbe, GoldBook,
    GoldEntry, GoldEvidenceRef, HistoryMode, JudgeExecutionConfig, Scorer, ScorerMetric,
    ScorerReport, make_text_percept,
};
use crate::datasets::manifest::DatasetManifestEntry;
use crate::datasets::scoring_utils::binary_prf1;

pub struct UbuntuIrcConverter;

struct ParsedIrcLine {
    ts_ms: u64,
    speaker: String,
    text: String,
}

fn parse_irc_raw_line(
    line: &str,
    prev_min_of_day: &mut u64,
    day_offset: &mut u64,
) -> Option<ParsedIrcLine> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let after_bracket = trimmed.strip_prefix('[')?;
    let (hhmm, rest) = after_bracket.split_once(']')?;
    let (hh_str, mm_str) = hhmm.split_once(':')?;
    let hh: u64 = hh_str.trim().parse().ok()?;
    let mm: u64 = mm_str.trim().parse().ok()?;
    let min_of_day = hh * 60 + mm;
    if min_of_day < *prev_min_of_day {
        *day_offset += 86_400_000;
    }
    *prev_min_of_day = min_of_day;
    let ts_ms = 1_700_000_000_000 + *day_offset + min_of_day * 60_000;

    let rest = rest.trim();
    let (speaker, text) = if let Some(after_lt) = rest.strip_prefix('<') {
        if let Some((nick, body)) = after_lt.split_once('>') {
            (nick.trim().to_owned(), body.trim().to_owned())
        } else {
            ("irc_system".to_owned(), rest.to_owned())
        }
    } else {
        ("irc_system".to_owned(), rest.to_owned())
    };
    Some(ParsedIrcLine {
        ts_ms,
        speaker,
        text,
    })
}

impl Converter for UbuntuIrcConverter {
    fn dataset_id(&self) -> &str {
        "ubuntu_irc"
    }

    fn converter_version(&self) -> &str {
        "1.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw_txt = fs::read_to_string(dataset_dir.join("data/test/test.raw.txt"))?;
        let ann_txt = fs::read_to_string(dataset_dir.join("data/test/test.annotation.txt"))?;

        let mut prev_min = 0_u64;
        let mut day_offset = 0_u64;
        let mut parsed_lines = Vec::new();
        for line in raw_txt.lines() {
            if let Some(p) = parse_irc_raw_line(line, &mut prev_min, &mut day_offset) {
                parsed_lines.push(p);
            }
        }

        // Parse `.annotation.txt` reply-to edges (`child_idx parent_idx` or `child parent -`).
        let mut parent_of: BTreeMap<usize, usize> = BTreeMap::new();
        for line in ann_txt.lines().filter(|l| !l.trim().is_empty()) {
            let nums: Vec<usize> = line
                .split_whitespace()
                .filter_map(|t| t.parse::<usize>().ok())
                .collect();
            if nums.len() >= 2 {
                parent_of.insert(nums[0], nums[1]);
            }
        }

        let soul = ActorId::new("soul");
        let proctor = ActorId::new("lab_proctor");
        let place = PlaceId::new("#ubuntu");
        let mut speakers: BTreeSet<ActorId> = parsed_lines
            .iter()
            .map(|l| ActorId::new(&l.speaker))
            .collect();
        speakers.insert(proctor.clone());
        speakers.insert(soul.clone());
        let scene_aud = Audience::new(speakers);
        let probe_aud = Audience::new([proctor, soul]);

        let mut history = Vec::with_capacity(parsed_lines.len());
        let mut ev_refs = Vec::with_capacity(parsed_lines.len());
        let mut last_ts = 1_700_000_000_000_u64;
        for (idx, l) in parsed_lines.iter().enumerate() {
            last_ts = last_ts.max(l.ts_ms);
            history.push(make_text_percept(
                &l.speaker,
                place.as_str(),
                scene_aud.members.iter().cloned(),
                l.ts_ms,
                format!("[msg_{idx}] {}", l.text),
            ));
            ev_refs.push(GoldEvidenceRef {
                turn_index: idx,
                audience: scene_aud.clone(),
                place: place.clone(),
            });
        }

        let mut probes = Vec::with_capacity(parsed_lines.len());
        let mut gold_entries = Vec::with_capacity(parsed_lines.len());
        for idx in 0..parsed_lines.len() {
            let probe_id = format!("msg_{idx}");
            let parent_idx = parent_of.get(&idx).copied().unwrap_or(idx);
            let parent_speaker = parsed_lines
                .get(parent_idx)
                .map_or_else(|| "irc_system".to_owned(), |l| l.speaker.clone());

            probes.push(ExternalProbe {
                probe_id: probe_id.clone(),
                percept: make_text_percept(
                    "lab_proctor",
                    place.as_str(),
                    probe_aud.members.iter().cloned(),
                    last_ts + ((idx + 1) as u64) * 1_000,
                    format!("Identify reply-to parent message index and speaker for msg_{idx}."),
                ),
                isolated_checkpoint: true,
                probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                expected_capability: None,
            });

            let mut metadata = BTreeMap::new();
            metadata.insert("msg_idx".to_owned(), idx.to_string());
            metadata.insert("parent_idx".to_owned(), parent_idx.to_string());
            metadata.insert("parent_speaker".to_owned(), parent_speaker.clone());

            gold_entries.push(GoldEntry {
                item_id: "ubuntu_irc_test".to_owned(),
                probe_id,
                cluster_id: "ubuntu_irc_test".to_owned(),
                gold_answer: format!("msg_{parent_idx} {parent_speaker}"),
                accepted_answers: vec![parent_idx.to_string(), parent_speaker],
                gold_evidence: ev_refs.clone(),
                privacy_conflict: false,
                category: Some("disentanglement".to_owned()),
                metadata,
            });
        }

        let item = ExternalItem {
            item_id: "ubuntu_irc_test".to_owned(),
            cluster_id: "ubuntu_irc_test".to_owned(),
            strata_key: "contiguous_window".to_owned(),
            history,
            checkpoint_group: Some("ubuntu_irc_test".to_owned()),
            probes,
            history_mode: HistoryMode::StaticSilent,
            capabilities: Vec::new(),
        };

        Ok(ConvertedDataset {
            dataset_id: "ubuntu_irc".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items: vec![item],
            gold_book: GoldBook {
                dataset_id: "ubuntu_irc".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct UbuntuIrcScorer;

fn connected_components(n: usize, edges: &[(usize, usize)]) -> BTreeSet<Vec<usize>> {
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for &(a, b) in edges {
        if a < n && b < n {
            let ra = find(&mut parent, a);
            let rb = find(&mut parent, b);
            if ra != rb {
                parent[rb] = ra;
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    groups.into_values().collect()
}

impl Scorer for UbuntuIrcScorer {
    fn dataset_id(&self) -> &str {
        "ubuntu_irc"
    }

    fn scorer_version(&self) -> &str {
        "1.0.0"
    }

    fn score(
        &self,
        predictions: &[ExternalPrediction],
        gold: &GoldBook,
        _judge: &JudgeExecutionConfig<'_>,
        _seed: u64,
    ) -> Result<ScorerReport, LabError> {
        let n = predictions.len();
        let mut gold_edges = Vec::with_capacity(n);
        let mut pred_edges = Vec::with_capacity(n);
        let mut speaker_hits = 0usize;

        for pred in predictions {
            let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
                LabError::Protocol(format!("ubuntu_irc missing gold for {}", pred.probe_id))
            })?;
            let msg_idx: usize = entry
                .metadata
                .get("msg_idx")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let g_parent: usize = entry
                .metadata
                .get("parent_idx")
                .and_then(|s| s.parse().ok())
                .unwrap_or(msg_idx);
            let g_spk = entry
                .metadata
                .get("parent_speaker")
                .map(String::as_str)
                .unwrap_or("");
            gold_edges.push((msg_idx, g_parent));

            let resp = pred.response_text.as_deref().unwrap_or("");
            let p_parent = resp
                .split(|c: char| !c.is_ascii_digit())
                .filter(|s| !s.is_empty())
                .find_map(|s| s.parse::<usize>().ok())
                .filter(|&p| p < n)
                .unwrap_or(msg_idx);
            pred_edges.push((msg_idx, p_parent));

            if !g_spk.is_empty()
                && resp
                    .to_ascii_lowercase()
                    .contains(&g_spk.to_ascii_lowercase())
            {
                speaker_hits += 1;
            }
        }

        let g_clusters = connected_components(n, &gold_edges);
        let p_clusters = connected_components(n, &pred_edges);
        let tp = g_clusters.intersection(&p_clusters).count();
        let fp = p_clusters.len().saturating_sub(tp);
        let fn_ = g_clusters.len().saturating_sub(tp);
        let (_, _, thread_f1) = binary_prf1(tp, fp, fn_);
        let parent_speaker_p1 = if n == 0 {
            0.0
        } else {
            speaker_hits as f64 / n as f64
        };

        Ok(ScorerReport {
            dataset_id: "ubuntu_irc".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![
                ScorerMetric {
                    name: "thread_f1".to_owned(),
                    value: Some(thread_f1),
                    ci95_low: None,
                    ci95_high: None,
                    n_clusters: Some(g_clusters.len()),
                    not_scored_reason: None,
                },
                ScorerMetric {
                    name: "parent_speaker_p1".to_owned(),
                    value: Some(parent_speaker_p1),
                    ci95_low: None,
                    ci95_high: None,
                    n_clusters: Some(1),
                    not_scored_reason: None,
                },
            ],
            judge_model: None,
            judge_prompt_version: None,
        })
    }
}
