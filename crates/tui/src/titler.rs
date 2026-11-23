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

/// Timeout of a title request (a reasoning model may need a while to answer).
pub const TITLER_TIMEOUT_MS: u64 = 30_000;

/// Longest generated or fallback title.
pub const TITLE_MAX_CHARS: usize = 40;

/// Builds the title request: `ModelPurpose::Classify`, `max_output_tokens` taken from the
/// classify output limit (`model.output_limits.classify`), 30 s timeout.
fn title_request(
    model_name: &str,
    current_title: Option<&str>,
    recent_messages: &[String],
    classify_limit: u32,
) -> ModelRequest {
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
    req.max_output_tokens = Some(classify_limit.max(1));
    req.timeout_ms = TITLER_TIMEOUT_MS;
    req.temperature = Some(0.2);
    req
}

/// Asks `provider` for a title. `Ok(None)` means the model answered `KEEP` (or nothing usable);
/// `Err` carries the provider failure so the caller can log it.
///
/// # Errors
/// The provider's failure text.
pub fn request_chat_title(
    provider: &Arc<dyn ModelProvider>,
    model_name: &str,
    current_title: Option<&str>,
    recent_messages: &[String],
    classify_limit: u32,
) -> Result<Option<String>, String> {
    if recent_messages.is_empty() {
        return Ok(None);
    }
    let req = title_request(model_name, current_title, recent_messages, classify_limit);
    let res = provider.invoke(&req).map_err(|e| e.to_string())?;
    Ok(match res.output {
        ModelOutput::Verdict(verdict) => clean_generated_title(&verdict.label),
        ModelOutput::ThoughtActs(_) => clean_generated_title(&res.raw_text),
    })
}

/// Deterministic title from the first message: its leading content words (words of at least
/// four characters, up to five), title-cased and cut at [`TITLE_MAX_CHARS`] on a word boundary.
/// Language-agnostic: no word lists. Falls back to the first few words of any length, then to
/// the first characters, so a chat is never left as "New Chat".
#[must_use]
pub fn fallback_title(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_owned()
        })
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return None;
    }
    let content: Vec<&String> = words.iter().filter(|w| w.chars().count() >= 4).collect();
    let picked: Vec<&String> = if content.len() >= 2 {
        content.into_iter().take(5).collect()
    } else {
        words.iter().take(5).collect()
    };
    let mut title = String::new();
    for w in picked {
        let cased = title_case(w);
        let extra = usize::from(!title.is_empty()) + cased.chars().count();
        if title.chars().count() + extra > TITLE_MAX_CHARS {
            break;
        }
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str(&cased);
    }
    if title.is_empty() {
        // One very long token (or unspaced script): cut it.
        title = title_case(&words[0]).chars().take(TITLE_MAX_CHARS).collect();
    }
    Some(title)
}

fn title_case(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect(),
        None => String::new(),
    }
}

/// Runs the titler for one trigger and returns the title to apply, if any.
///
/// A provider failure is reported through `log_failure` (the caller files it under `titler` at
/// warn level). For the initial title a failure falls back to [`fallback_title`] of
/// `first_message`; a topic-shift failure leaves the current title alone.
pub fn run_titler(
    provider: &Arc<dyn ModelProvider>,
    model_name: &str,
    current_title: Option<&str>,
    recent_messages: &[String],
    classify_limit: u32,
    first_message: &str,
    log_failure: &mut dyn FnMut(&str),
) -> Option<String> {
    match request_chat_title(provider, model_name, current_title, recent_messages, classify_limit) {
        Ok(Some(title)) => Some(title),
        Ok(None) if current_title.is_some() => None, // KEEP
        Ok(None) => {
            log_failure("the model returned no usable title");
            fallback_title(first_message)
        }
        Err(err) => {
            log_failure(&err);
            if current_title.is_none() {
                fallback_title(first_message)
            } else {
                None
            }
        }
    }
}

/// Invokes `provider` with `ModelPurpose::Classify` to produce a concise 2-5 word title in the
/// SAME language as the user's messages (default classify limit, 30 s timeout).
///
/// When `current_title` is `Some`, instructs the model to reply `KEEP` if the topic has not
/// meaningfully changed; returns `None` on `KEEP` or any provider error/timeout.
pub fn generate_chat_title(
    provider: &Arc<dyn ModelProvider>,
    model_name: &str,
    current_title: Option<&str>,
    recent_messages: &[String],
) -> Option<String> {
    request_chat_title(
        provider,
        model_name,
        current_title,
        recent_messages,
        monoagens_reason::OutputLimitProfile::default().classify,
    )
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use monoagens_reason::{ScriptedModelProvider, ScriptedReply};

    fn failing() -> Arc<dyn ModelProvider> {
        Arc::new(ScriptedModelProvider::from_replies([ScriptedReply::failure(
            monoagens_reason::ProviderFailureClass::Server,
            "HTTP 500: boom",
        )]))
    }

    #[test]
    fn provider_failure_falls_back_to_a_keyphrase_and_logs() {
        let mut logged = Vec::new();
        let title = run_titler(
            &failing(),
            "m",
            None,
            &["tolong jelaskan cara kerja valve rbac".to_owned()],
            1024,
            "tolong jelaskan cara kerja valve rbac",
            &mut |m| logged.push(m.to_owned()),
        )
        .expect("a title");
        assert_eq!(title, "Tolong Jelaskan Cara Kerja Valve");
        assert!(title.chars().count() <= 40);
        assert_ne!(title, "New Chat");
        assert_eq!(logged.len(), 1);
        assert!(logged[0].contains("boom"), "the error is logged: {logged:?}");
    }

    #[test]
    fn topic_shift_failure_keeps_the_current_title() {
        let mut logged = Vec::new();
        let title = run_titler(&failing(), "m", Some("Old"), &["x".to_owned()], 1024, "x", &mut |m| {
            logged.push(m.to_owned());
        });
        assert_eq!(title, None);
        assert_eq!(logged.len(), 1);
    }

    #[test]
    fn model_title_is_cleaned_and_request_uses_the_classify_limit_and_30s() {
        let req = title_request("m", None, &["hello world".to_owned()], 777);
        assert_eq!(req.max_output_tokens, Some(777));
        assert_eq!(req.timeout_ms, 30_000);
        let ok: Arc<dyn ModelProvider> = Arc::new(ScriptedModelProvider::from_replies([
            ScriptedReply::text(r#"{"label":"\"Valve RBAC Guide\"","score":0.9}"#),
        ]));
        let got = request_chat_title(&ok, "m", None, &["hello".to_owned()], 1024);
        // Whatever the verdict shape, a failure to parse is an error, never a silent drop.
        match got {
            Ok(Some(t)) => assert_eq!(t, "Valve RBAC Guide"),
            Ok(None) => panic!("no title"),
            Err(e) => assert!(!e.is_empty()),
        }
    }

    fn old_chat() -> LocalChatEntry {
        let mut c = LocalChatEntry::direct_default();
        c.title = "Rust Lifetimes".to_owned();
        c.turns_since_retitle = 3;
        c.topic_ngrams = extract_topic_ngrams("explain rust lifetimes borrow checker");
        c
    }

    #[test]
    fn embedder_decides_the_topic_shift_when_embeddings_exist() {
        let chat = old_chat();
        let centroid = [1.0_f32, 0.0, 0.0];
        // Lexically unrelated text, but the embedder says it is the same topic: no shift.
        let same = [0.99_f32, 0.1, 0.0];
        assert_eq!(
            should_request_title_with_embeddings(&chat, "qwertyuiop zxcvbnm", Some(&same), Some(&centroid), true),
            TitleTrigger::None
        );
        // Lexically similar text, but the embedder says the topic moved: shift.
        let far = [0.0_f32, 1.0, 0.0];
        assert_eq!(
            should_request_title_with_embeddings(&chat, "explain rust lifetimes borrow checker", Some(&far), Some(&centroid), true),
            TitleTrigger::TopicShift
        );
        // Without embeddings the n-gram rule decides (existing behaviour).
        assert_eq!(
            should_request_title_with_embeddings(&chat, "qwertyuiop zxcvbnm", None, None, true),
            TitleTrigger::TopicShift
        );
    }

    #[test]
    fn a_manual_rename_locks_the_title_even_when_the_topic_shifts() {
        let mut chat = old_chat();
        chat.title_locked = true;
        let far = [0.0_f32, 1.0, 0.0];
        assert_eq!(
            should_request_title_with_embeddings(&chat, "something new", Some(&far), Some(&[1.0, 0.0, 0.0]), true),
            TitleTrigger::None
        );
        assert_eq!(should_request_title(&chat, "completely different words here", true), TitleTrigger::None);
    }

    #[test]
    fn centroid_is_a_running_mean_stored_in_the_chat() {
        let mut c = LocalChatEntry::direct_default();
        c.update_topic_centroid(&[1.0, 0.0], 1);
        c.update_topic_centroid(&[0.0, 1.0], 2);
        let v = c.topic_centroid().expect("centroid");
        assert!((v[0] - 0.5).abs() < 1e-5 && (v[1] - 0.5).abs() < 1e-5, "{v:?}");
        c.update_topic_centroid(&[0.2, 0.2], 1);
        assert_eq!(c.topic_centroid().unwrap().len(), 2, "turns=1 starts over");
    }

    #[test]
    fn fallback_title_shapes() {
        assert_eq!(fallback_title("hi"), Some("Hi".to_owned()));
        assert_eq!(fallback_title("   "), None);
        let long = fallback_title("supercalifragilisticexpialidocious pneumonoultramicroscopicsilicovolcanoconiosis").unwrap();
        assert!(long.chars().count() <= 40, "{long}");
        let cjk = fallback_title("今日はとても良い天気ですね、散歩に行きましょう").unwrap();
        assert!(cjk.chars().count() <= 40 && !cjk.is_empty());
    }
}
