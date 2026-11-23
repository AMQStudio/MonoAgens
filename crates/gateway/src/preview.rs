//! Owner-only draft preview of the reply text while the model is still writing.
//!
//! Safety (owner decision D-3, ARCHITECTURE "the valve runs first and last in every door"):
//! a preview is *not* an action. It is a gateway-local channel to the TUI that submitted the
//! turn. It is never recorded as an event, never enters the input queue, is never read by any
//! decision, and no adapter, robot or other body is ever given a subscription. The delivered
//! `chat.speak` still exists only after the whole structured output was parsed and passed the
//! action door.
//!
//! * [`DraftExtractor`] turns the streamed structured output into the speech text written so
//!   far (a tolerant incremental JSON reader; text only ever grows).
//! * [`PreviewHub`] holds the subscriptions (the in-process TUI, or the admin socket's
//!   `preview.subscribe`) and the set of model requests allowed to stream.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

/// Capability whose payload is the reply text shown in a chat.
pub const SPEAK_CAPABILITY: &str = "chat.speak";

// ---------------------------------------------------------------------------------------------
// Tolerant partial JSON
// ---------------------------------------------------------------------------------------------

struct Partial<'a> {
    s: &'a [char],
    i: usize,
}

impl Partial<'_> {
    fn skip_ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_whitespace() {
            self.i += 1;
        }
    }

    fn eof(&self) -> bool {
        self.i >= self.s.len()
    }

    /// Parses a value; `None` when nothing usable was written yet.
    fn value(&mut self) -> Option<Value> {
        self.skip_ws();
        match *self.s.get(self.i)? {
            '{' => Some(self.object()),
            '[' => Some(self.array()),
            '"' => self.string().map(|(s, _)| Value::String(s)),
            't' | 'f' | 'n' => self.literal(),
            _ => self.number(),
        }
    }

    fn literal(&mut self) -> Option<Value> {
        for (word, v) in [
            ("true", Value::Bool(true)),
            ("false", Value::Bool(false)),
            ("null", Value::Null),
        ] {
            let rest = &self.s[self.i..];
            if rest.len() >= word.len() && rest.iter().take(word.len()).copied().eq(word.chars()) {
                self.i += word.len();
                return Some(v);
            }
        }
        // A partial word at the end, or garbage: consume it, report nothing.
        while !self.eof() && self.s[self.i].is_ascii_alphabetic() {
            self.i += 1;
        }
        None
    }

    fn number(&mut self) -> Option<Value> {
        let start = self.i;
        while !self.eof() && matches!(self.s[self.i], '-' | '+' | '.' | 'e' | 'E' | '0'..='9') {
            self.i += 1;
        }
        if start == self.i {
            // Not a value we know; skip one char so the caller makes progress.
            self.i += 1;
            return None;
        }
        if self.eof() {
            // A number cut by the end of the input may still grow ("7" before "72").
            return None;
        }
        let text: String = self.s[start..self.i].iter().collect();
        serde_json::from_str::<Value>(&text).ok()
    }

    /// Parses a string. Returns the text decoded so far and whether the closing quote was seen.
    /// An incomplete escape (or a high surrogate waiting for its pair) at the end is dropped.
    fn string(&mut self) -> Option<(String, bool)> {
        debug_assert_eq!(self.s.get(self.i), Some(&'"'));
        self.i += 1;
        let mut out = String::new();
        loop {
            let Some(&c) = self.s.get(self.i) else {
                return Some((out, false));
            };
            self.i += 1;
            match c {
                '"' => return Some((out, true)),
                '\\' => {
                    let Some(&e) = self.s.get(self.i) else {
                        return Some((out, false));
                    };
                    self.i += 1;
                    match e {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        '/' => out.push('/'),
                        '\\' => out.push('\\'),
                        '"' => out.push('"'),
                        'u' => {
                            let Some(hi) = self.hex4() else {
                                return Some((out, false));
                            };
                            if (0xD800..0xDC00).contains(&hi) {
                                // Needs the low half; at the end of input wait for it.
                                let save = self.i;
                                if self.s.get(self.i) == Some(&'\\') && self.s.get(self.i + 1) == Some(&'u') {
                                    self.i += 2;
                                    match self.hex4() {
                                        Some(lo) if (0xDC00..0xE000).contains(&lo) => {
                                            let cp = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                                            out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                                        }
                                        Some(_) => {
                                            out.push('\u{fffd}');
                                            self.i = save;
                                        }
                                        None => return Some((out, false)),
                                    }
                                } else if self.eof() || (self.s.get(self.i) == Some(&'\\') && self.i + 1 >= self.s.len()) {
                                    return Some((out, false));
                                } else {
                                    out.push('\u{fffd}');
                                }
                            } else {
                                out.push(char::from_u32(hi).unwrap_or('\u{fffd}'));
                            }
                        }
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        if self.i + 4 > self.s.len() {
            self.i = self.s.len();
            return None;
        }
        let mut v = 0u32;
        for k in 0..4 {
            v = v * 16 + self.s[self.i + k].to_digit(16).unwrap_or(0);
        }
        self.i += 4;
        Some(v)
    }

    fn object(&mut self) -> Value {
        self.i += 1;
        let mut map = serde_json::Map::new();
        loop {
            self.skip_ws();
            match self.s.get(self.i) {
                None => break,
                Some('}') => {
                    self.i += 1;
                    break;
                }
                Some(',') => {
                    self.i += 1;
                    continue;
                }
                Some('"') => {}
                Some(_) => {
                    self.i += 1;
                    continue;
                }
            }
            let Some((key, complete)) = self.string() else { break };
            if !complete {
                break;
            }
            self.skip_ws();
            if self.s.get(self.i) != Some(&':') {
                break;
            }
            self.i += 1;
            match self.value() {
                Some(v) => {
                    map.insert(key, v);
                }
                None => {
                    if self.eof() {
                        break;
                    }
                }
            }
        }
        Value::Object(map)
    }

    fn array(&mut self) -> Value {
        self.i += 1;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.s.get(self.i) {
                None => break,
                Some(']') => {
                    self.i += 1;
                    break;
                }
                Some(',') => {
                    self.i += 1;
                    continue;
                }
                Some(_) => {}
            }
            match self.value() {
                Some(v) => items.push(v),
                None => {
                    if self.eof() {
                        break;
                    }
                }
            }
        }
        Value::Array(items)
    }
}

/// Best-effort parse of a JSON text that may be cut off anywhere.
#[must_use]
pub fn parse_partial_json(text: &str) -> Option<Value> {
    let chars: Vec<char> = text.chars().collect();
    Partial { s: &chars, i: 0 }.value()
}

/// Payload of a speak act: a string, or an array of byte values (UTF-8). A trailing incomplete
/// UTF-8 sequence is left out until the rest arrives.
fn payload_text(payload: &Value) -> Option<String> {
    match payload {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let bytes: Vec<u8> = items
                .iter()
                .filter_map(|v| v.as_u64().and_then(|n| u8::try_from(n).ok()))
                .collect();
            match std::str::from_utf8(&bytes) {
                Ok(s) => Some(s.to_owned()),
                Err(e) if e.error_len().is_none() => {
                    Some(String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned())
                }
                Err(_) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            }
        }
        _ => None,
    }
}

/// The speech text of a (possibly partial) structured model output: the payload of every
/// `chat.speak` act addressed to `place`, joined by a blank line.
#[must_use]
pub fn draft_text_of(output: &str, place: &str) -> String {
    let Some(root) = parse_partial_json(output) else {
        return String::new();
    };
    let Some(acts) = root.get("acts").and_then(Value::as_array) else {
        return String::new();
    };
    let mut parts: Vec<String> = Vec::new();
    for act in acts {
        if act.get("capability_id").and_then(Value::as_str) != Some(SPEAK_CAPABILITY) {
            continue;
        }
        let addressed = act
            .get("places")
            .and_then(Value::as_array)
            .is_some_and(|p| p.iter().any(|v| v.as_str() == Some(place)));
        if !addressed {
            continue;
        }
        if let Some(text) = act.get("payload").and_then(payload_text)
            && !text.is_empty()
        {
            parts.push(text);
        }
    }
    parts.join("\n\n")
}

/// Incremental draft extraction over the streamed output of one model call.
#[derive(Debug)]
pub struct DraftExtractor {
    place: String,
    buf: String,
    last: String,
}

impl DraftExtractor {
    #[must_use]
    pub fn new(place: &str) -> Self {
        Self {
            place: place.to_owned(),
            buf: String::new(),
            last: String::new(),
        }
    }

    /// Adds a text delta; returns the new draft text when it changed.
    pub fn push(&mut self, delta: &str) -> Option<String> {
        self.buf.push_str(delta);
        let text = draft_text_of(&self.buf, &self.place);
        // The draft only grows: a re-parse that yields something shorter or different is held back
        // until it catches up (a half-written escape, a payload still reordering).
        if text.len() > self.last.len() && text.starts_with(&self.last) {
            self.last = text;
            return Some(self.last.clone());
        }
        None
    }

    /// Draft text so far.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.last
    }
}

// ---------------------------------------------------------------------------------------------
// Hub
// ---------------------------------------------------------------------------------------------

/// How a streamed model call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewEnd {
    /// The call finished and its output goes through the valve; the draft stays until the real
    /// outcome (reply, verdict, silence) is known.
    Completed,
    /// The provider failed.
    Failed,
    /// The owner cancelled it, or it timed out.
    Cancelled,
}

/// One preview notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewEvent {
    /// The full draft text so far for `request_id`.
    Draft { request_id: u64, text: String },
    /// The streamed call for `request_id` is over.
    End { request_id: u64, outcome: PreviewEnd },
}

struct HubInner {
    place: String,
    next: AtomicU64,
    subs: Mutex<Vec<(u64, mpsc::UnboundedSender<PreviewEvent>)>>,
    eligible: Mutex<HashSet<u64>>,
}

/// Subscriptions and eligibility of draft previews (cheap to clone, shared).
#[derive(Clone)]
pub struct PreviewHub {
    inner: Arc<HubInner>,
}

impl std::fmt::Debug for PreviewHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewHub")
            .field("place", &self.inner.place)
            .field("subscribers", &self.subscriber_count())
            .finish()
    }
}

impl Default for PreviewHub {
    fn default() -> Self {
        Self::new("cli:terminal")
    }
}

/// A live subscription; dropping it unsubscribes.
pub struct PreviewSubscription {
    id: u64,
    hub: PreviewHub,
    rx: mpsc::UnboundedReceiver<PreviewEvent>,
}

impl PreviewSubscription {
    /// Subscription id (as reported to admin clients).
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// Next notification; `None` once unsubscribed.
    pub async fn recv(&mut self) -> Option<PreviewEvent> {
        self.rx.recv().await
    }
}

impl Drop for PreviewSubscription {
    fn drop(&mut self) {
        self.hub.unsubscribe(self.id);
    }
}

impl PreviewHub {
    /// A hub whose previews are only produced for model calls triggered at `place` (the TUI's
    /// own place).
    #[must_use]
    pub fn new(place: &str) -> Self {
        Self {
            inner: Arc::new(HubInner {
                place: place.to_owned(),
                next: AtomicU64::new(1),
                subs: Mutex::new(Vec::new()),
                eligible: Mutex::new(HashSet::new()),
            }),
        }
    }

    /// The place previews are limited to.
    #[must_use]
    pub fn place(&self) -> &str {
        &self.inner.place
    }

    /// Number of subscribers.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.inner.subs.lock().expect("preview subs lock").len()
    }

    /// Whether anyone listens (no subscriber, no streaming request).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.subscriber_count() > 0
    }

    /// Adds a subscriber.
    #[must_use]
    pub fn subscribe(&self) -> PreviewSubscription {
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        self.inner.subs.lock().expect("preview subs lock").push((id, tx));
        PreviewSubscription {
            id,
            hub: self.clone(),
            rx,
        }
    }

    /// Removes a subscriber (its receiver then yields `None`).
    pub fn unsubscribe(&self, id: u64) {
        self.inner
            .subs
            .lock()
            .expect("preview subs lock")
            .retain(|(sid, _)| *sid != id);
    }

    /// Marks a model request as allowed to stream (called when it is prepared for the TUI place).
    pub fn mark_eligible(&self, request_id: u64) {
        let mut set = self.inner.eligible.lock().expect("preview eligible lock");
        if set.len() > 256 {
            set.clear();
        }
        set.insert(request_id);
    }

    /// Consumes the eligibility of `request_id`.
    #[must_use]
    pub fn take_eligible(&self, request_id: u64) -> bool {
        self.inner
            .eligible
            .lock()
            .expect("preview eligible lock")
            .remove(&request_id)
    }

    /// Sends a notification to every subscriber.
    pub fn publish(&self, event: &PreviewEvent) {
        let mut subs = self.inner.subs.lock().expect("preview subs lock");
        subs.retain(|(_, tx)| tx.send(event.clone()).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLACE: &str = "cli:terminal";

    fn output(text: &str) -> String {
        let bytes: Vec<String> = text.bytes().map(|b| b.to_string()).collect();
        format!(
            r#"{{"protocol_version":{{"major":1,"minor":0}},"acts":[{{"act":"think","thought":"x","motivation":3,"confidence_0_20":10}},{{"act":"act","capability_id":"chat.speak","places":["{PLACE}"],"people":[],"payload":[{}],"motivation":4}}]}}"#,
            bytes.join(",")
        )
    }

    fn string_output(text: &str) -> String {
        format!(
            r#"{{"acts":[{{"act":"act","capability_id":"chat.speak","places":["{PLACE}"],"payload":{}}}]}}"#,
            serde_json::to_string(text).unwrap()
        )
    }

    /// Every prefix (at every char boundary) yields a draft that is a prefix of the final text,
    /// drafts never shrink, and the whole output yields exactly the final text.
    fn assert_monotonic(full: &str, expected: &str) {
        let mut prev = String::new();
        for (i, _) in full.char_indices().chain(std::iter::once((full.len(), ' '))) {
            let d = draft_text_of(&full[..i], PLACE);
            assert!(expected.starts_with(&d), "prefix {i}: {d:?} is not a prefix of {expected:?}");
            assert!(d.starts_with(&prev), "draft shrank at {i}: {prev:?} -> {d:?}");
            prev = d;
        }
        assert_eq!(prev, expected);
    }

    #[test]
    fn byte_array_payload_every_offset() {
        let text = "Halo dunia 🙂 ünï\nbaris dua";
        assert_monotonic(&output(text), text);
    }

    #[test]
    fn string_payload_with_escapes_and_astral_chars_every_offset() {
        let text = "quote \" backslash \\ tab\t newline\n unicode é 🙂 end";
        let full = string_output(text);
        // serde_json writes the astral char raw; also test an explicit surrogate pair escape.
        assert_monotonic(&full, text);
        let escaped = r#"{"acts":[{"capability_id":"chat.speak","places":["cli:terminal"],"payload":"a😀bé\n"}]}"#;
        assert_monotonic(escaped, "a😀bé\n");
    }

    #[test]
    fn speech_field_order_does_not_matter_and_other_places_are_ignored() {
        let payload_first = r#"{"acts":[{"payload":"hi","capability_id":"chat.speak","places":["cli:terminal"]}]}"#;
        assert_eq!(draft_text_of(payload_first, PLACE), "hi");
        let elsewhere = r#"{"acts":[{"capability_id":"chat.speak","places":["wa:123"],"payload":"secret"}]}"#;
        assert_eq!(draft_text_of(elsewhere, PLACE), "", "a speak for another place is never previewed");
        let other_cap = r#"{"acts":[{"capability_id":"fs.write","places":["cli:terminal"],"payload":"nope"}]}"#;
        assert_eq!(draft_text_of(other_cap, PLACE), "");
        let thought_only = r#"{"acts":[{"act":"think","thought":"private reasoning"}]}"#;
        assert_eq!(draft_text_of(thought_only, PLACE), "", "think acts are never a draft");
    }

    #[test]
    fn extractor_publishes_only_growth() {
        let full = string_output("hello world");
        let mut ex = DraftExtractor::new(PLACE);
        let mut seen = Vec::new();
        for ch in full.chars() {
            if let Some(t) = ex.push(&ch.to_string()) {
                seen.push(t);
            }
        }
        assert_eq!(seen.last().map(String::as_str), Some("hello world"));
        assert!(seen.windows(2).all(|w| w[1].len() > w[0].len() && w[1].starts_with(&w[0])));
        assert_eq!(ex.text(), "hello world");
    }

    #[tokio::test]
    async fn hub_delivers_to_subscribers_only_and_unsubscribes_on_drop() {
        let hub = PreviewHub::new(PLACE);
        assert!(!hub.is_active());
        let mut sub = hub.subscribe();
        assert!(hub.is_active());
        hub.publish(&PreviewEvent::Draft { request_id: 1, text: "x".into() });
        let got = tokio::time::timeout(std::time::Duration::from_secs(10), sub.recv())
            .await
            .expect("no hang");
        assert_eq!(got, Some(PreviewEvent::Draft { request_id: 1, text: "x".into() }));
        hub.mark_eligible(5);
        assert!(hub.take_eligible(5));
        assert!(!hub.take_eligible(5));
        drop(sub);
        assert!(!hub.is_active());
    }
}
