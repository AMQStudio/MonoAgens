//! Non-blocking, language-agnostic LLM chat auto-naming and topic-shift re-titling.
//!
//! Uses optional semantic embedding cosine similarity (e.g. multilingual EmbeddingGemma 2)
//! and falls back to language-agnostic Unicode character 3-gram Jaccard overlap (zero
//! hardcoded word lists).

use std::collections::BTreeSet;
use std::sync::Arc;

use monoagens_core::ModelPurpose;
use monoagens_protocol::EventId;
use monoagens_reason::{ModelOutput, ModelProvider, ModelRequest};

use crate::config::LocalChatEntry;

/// Trigger decision for background LLM chat titling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleTrigger {
    None,
    Initial,
    TopicShift,
}

/// Computes cosine similarity in `[-1.0, 1.0]` between two embedding vectors.
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.is_empty() || a.len() != b.len() {
        return None;
    }
    let mut dot = 0.0_f64;
    let mut na = 0.0_f64;
    let mut nb = 0.0_f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let xf = f64::from(*x);
        let yf = f64::from(*y);
        dot += xf * yf;
        na += xf * xf;
        nb += yf * yf;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom <= f64::EPSILON {
        None
    } else {
        Some((dot / denom) as f32)
    }
}

/// Extracts language-agnostic Unicode character 3-grams over lowercased non-whitespace `char`s.
///
/// Works uniformly across Latin, CJK, Arabic, Cyrillic, and mixed-script text without any
/// language-specific stop-word lists.
#[must_use]
pub fn extract_topic_ngrams(text: &str) -> Vec<String> {
    let mut set = BTreeSet::new();
    for segment in text.split_whitespace() {
        let chars: Vec<char> = segment
            .chars()
            .flat_map(char::to_lowercase)
            .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
            .collect();
        if chars.is_empty() {
            continue;
        }
        if chars.len() < 3 {
            set.insert(chars.into_iter().collect());
        } else {
            for win in chars.windows(3) {
                set.insert(win.iter().collect());
            }
        }
    }
    set.into_iter().collect()
}

/// Computes Jaccard similarity in `[0.0, 1.0]` between two n-gram fingerprints.
#[must_use]
pub fn ngram_jaccard(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let set_a: BTreeSet<&str> = a.iter().map(String::as_str).collect();
    let set_b: BTreeSet<&str> = b.iter().map(String::as_str).collect();
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Determines whether a background title request should fire for `chat` given `user_text`,
/// optionally using semantic embedding cosine similarity before falling back to 3-gram Jaccard.
#[must_use]
pub fn should_request_title_with_embeddings(
    chat: &LocalChatEntry,
    user_text: &str,
    user_embedding: Option<&[f32]>,
    topic_embedding: Option<&[f32]>,
    auto_chat_title: bool,
) -> TitleTrigger {
    if !auto_chat_title || chat.title_locked {
        return TitleTrigger::None;
    }
    let is_default_title = chat.title == "Direct" || chat.title == "New Chat";
    if is_default_title && chat.turns_since_retitle == 0 && chat.topic_ngrams.is_empty() {
        return TitleTrigger::Initial;
    }
    if chat.turns_since_retitle >= 2 {
        if let (Some(u_emb), Some(t_emb)) = (user_embedding, topic_embedding)
            && let Some(sim) = cosine_similarity(u_emb, t_emb)
        {
            return if sim < 0.45 {
                TitleTrigger::TopicShift
            } else {
                TitleTrigger::None
            };
        }

        let current_ngrams = extract_topic_ngrams(user_text);
        if !current_ngrams.is_empty() && !chat.topic_ngrams.is_empty() {
            let overlap = ngram_jaccard(&chat.topic_ngrams, &current_ngrams);
            if overlap < 0.15 {
                return TitleTrigger::TopicShift;
            }
        }
    }
    TitleTrigger::None
}

/// Embedding-free entry point using Unicode character 3-gram Jaccard similarity.
#[must_use]
pub fn should_request_title(
    chat: &LocalChatEntry,
    user_text: &str,
    auto_chat_title: bool,
) -> TitleTrigger {
    should_request_title_with_embeddings(chat, user_text, None, None, auto_chat_title)
}

/// Cleans an LLM-returned title (strips quotes, markdown, prefixes, newlines) and clamps to 40 chars.
#[must_use]
pub fn clean_generated_title(raw: &str) -> Option<String> {
    let first_line = raw.lines().next().unwrap_or("").trim();
    let stripped_prefix = first_line
        .strip_prefix("Title:")
        .or_else(|| first_line.strip_prefix("title:"))
        .unwrap_or(first_line)
        .trim();
    let cleaned: String = stripped_prefix
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '*' | '#' | '-' | ':' | '.'))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty() || cleaned.eq_ignore_ascii_case("KEEP") {
        return None;
    }
    let clamped: String = cleaned.chars().take(40).collect();
    let trimmed = clamped.trim().to_owned();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("KEEP") {
        None
    } else {
        Some(trimmed)
    }
}

/// Invokes `provider` with `ModelPurpose::Classify` and `max_output_tokens = 32` to produce
/// a concise 2–5 word title in the SAME language as the user's messages.
///
/// When `current_title` is `Some`, instructs the model to reply `KEEP` if the topic has not
/// meaningfully changed; returns `None` on `KEEP` or any provider error/timeout.
pub fn generate_chat_title(
    provider: &Arc<dyn ModelProvider>,
    model_name: &str,
    current_title: Option<&str>,
    recent_messages: &[String],
) -> Option<String> {
    if recent_messages.is_empty() {
        return None;
    }

    let context_block = recent_messages
        .iter()
        .rev()
        .take(4)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let (system_prompt, user_prompt) = match current_title {
        Some(cur) if !cur.trim().is_empty() => (
            "You name chat conversations. Reply with KEEP in the JSON 'label' field if the \
             conversation topic has not meaningfully changed; otherwise return a concise 2 to 5 \
             word title in the SAME language as the user's messages. No quotes or markdown.",
            format!(
                "Current title: {cur}\nRecent messages:\n{context_block}\nReturn KEEP or updated 2-5 word title:"
            ),
        ),
        _ => (
            "You name chat conversations. Return a concise 2 to 5 word title in the JSON 'label' \
             field in the SAME language as the user's messages. No quotes, no punctuation, no markdown.",
            format!("Messages:\n{context_block}\nReturn concise 2-5 word title:"),
        ),
    };

    let mut req = ModelRequest::text(
        EventId::new(0),
        ModelPurpose::Classify,
        model_name,
        system_prompt,
        user_prompt,
    );
    req.max_output_tokens = Some(32);
    req.timeout_ms = 5_000;
    req.temperature = Some(0.2);

    let res = provider.invoke(&req).ok()?;
    match res.output {
        ModelOutput::Verdict(verdict) => clean_generated_title(&verdict.label),
        ModelOutput::ThoughtActs(_) => clean_generated_title(&res.raw_text),
    }
}
