//! Server-sent-event reading and the two streaming aggregators.
//!
//! A streamed model call is folded back into the *same* JSON envelope the blocking call returns,
//! so everything after the HTTP read (usage accounting, refusal and limit handling, structured
//! output parsing) is byte-for-byte the code path of a non-streamed call. The only extra effect
//! is the `on_text` callback that sees each text delta while it arrives.

use std::io::{BufRead, BufReader, Read};

use serde_json::{Value, json};

/// Longest single SSE line accepted (a runaway line is a protocol error, not a memory sink).
const MAX_SSE_LINE_BYTES: usize = 4 * 1024 * 1024;

/// Reads `data:` payloads from an SSE byte stream, calling `on_data` for each one. Stops early
/// when `on_data` returns `false` or the stream ends. Comment lines, `event:` / `id:` / `retry:`
/// fields and blank lines are skipped; a `data:` field that spans several lines is joined with
/// `\n` before it is handed over.
pub(crate) fn read_sse<R: Read>(
    reader: R,
    mut on_data: impl FnMut(&str) -> bool,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(reader);
    let mut data = String::new();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.by_ref().take(MAX_SSE_LINE_BYTES as u64).read_line(&mut line)?;
        if n == 0 {
            if !data.is_empty() {
                on_data(&data);
            }
            return Ok(());
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            if !data.is_empty() {
                let keep_going = on_data(&data);
                data.clear();
                if !keep_going {
                    return Ok(());
                }
            }
            continue;
        }
        if trimmed.starts_with(':') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
}

/// Outcome of folding a stream: the synthesized non-stream envelope and whether any text
/// was emitted to the sink.
pub(crate) struct Folded {
    pub envelope: Value,
    pub emitted_text: bool,
}

/// Folds `chat.completion.chunk` payloads into a `chat.completion` envelope.
pub(crate) struct ChatStreamFolder<'a> {
    content: String,
    refusal: String,
    finish_reason: Option<String>,
    model: Option<String>,
    usage: Option<Value>,
    emitted: bool,
    on_text: &'a mut dyn FnMut(&str),
    done: bool,
}

impl<'a> ChatStreamFolder<'a> {
    pub(crate) fn new(on_text: &'a mut dyn FnMut(&str)) -> Self {
        Self {
            content: String::new(),
            refusal: String::new(),
            finish_reason: None,
            model: None,
            usage: None,
            emitted: false,
            on_text,
            done: false,
        }
    }

    /// Feeds one `data:` payload; returns `false` once `[DONE]` was seen.
    pub(crate) fn feed(&mut self, data: &str) -> bool {
        let data = data.trim();
        if data == "[DONE]" {
            self.done = true;
            return false;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            return true;
        };
        if let Some(m) = v.get("model").and_then(Value::as_str)
            && !m.is_empty()
        {
            self.model = Some(m.to_owned());
        }
        if let Some(u) = v.get("usage")
            && !u.is_null()
        {
            self.usage = Some(u.clone());
        }
        if let Some(choice) = v.get("choices").and_then(Value::as_array).and_then(|c| c.first()) {
            if let Some(delta) = choice.get("delta") {
                if let Some(t) = delta.get("content").and_then(Value::as_str)
                    && !t.is_empty()
                {
                    self.content.push_str(t);
                    self.emitted = true;
                    (self.on_text)(t);
                }
                if let Some(r) = delta.get("refusal").and_then(Value::as_str) {
                    self.refusal.push_str(r);
                }
            }
            if let Some(f) = choice.get("finish_reason").and_then(Value::as_str) {
                self.finish_reason = Some(f.to_owned());
            }
        }
        true
    }

    /// The envelope a blocking call would have returned.
    pub(crate) fn finish(self) -> Folded {
        let mut message = json!({ "role": "assistant", "content": self.content });
        if !self.refusal.is_empty() {
            message["refusal"] = json!(self.refusal);
        }
        let mut env = json!({
            "choices": [{ "index": 0, "message": message, "finish_reason": self.finish_reason }],
        });
        if let Some(m) = self.model {
            env["model"] = json!(m);
        }
        if let Some(u) = self.usage {
            env["usage"] = u;
        }
        Folded {
            envelope: env,
            emitted_text: self.emitted,
        }
    }
}

/// Folds Messages-API streaming events into a non-stream `message` envelope.
pub(crate) struct MessagesStreamFolder<'a> {
    text: String,
    model: Option<String>,
    stop_reason: Option<String>,
    input_tokens: u64,
    output_tokens: u64,
    cache_read: Option<u64>,
    saw_usage: bool,
    emitted: bool,
    on_text: &'a mut dyn FnMut(&str),
}

impl<'a> MessagesStreamFolder<'a> {
    pub(crate) fn new(on_text: &'a mut dyn FnMut(&str)) -> Self {
        Self {
            text: String::new(),
            model: None,
            stop_reason: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_read: None,
            saw_usage: false,
            emitted: false,
            on_text,
        }
    }

    /// Feeds one `data:` payload; returns `false` after `message_stop`.
    pub(crate) fn feed(&mut self, data: &str) -> bool {
        let Ok(v) = serde_json::from_str::<Value>(data.trim()) else {
            return true;
        };
        match v.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(m) = v.pointer("/message/model").and_then(Value::as_str) {
                    self.model = Some(m.to_owned());
                }
                if let Some(u) = v.pointer("/message/usage") {
                    self.absorb_usage(u);
                }
            }
            Some("content_block_delta") => {
                if v.pointer("/delta/type").and_then(Value::as_str) == Some("text_delta")
                    && let Some(t) = v.pointer("/delta/text").and_then(Value::as_str)
                    && !t.is_empty()
                {
                    self.text.push_str(t);
                    self.emitted = true;
                    (self.on_text)(t);
                }
            }
            Some("message_delta") => {
                if let Some(s) = v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = Some(s.to_owned());
                }
                if let Some(u) = v.get("usage") {
                    self.absorb_usage(u);
                }
            }
            Some("message_stop") => return false,
            _ => {}
        }
        true
    }

    fn absorb_usage(&mut self, u: &Value) {
        self.saw_usage = true;
        if let Some(n) = u.get("input_tokens").and_then(Value::as_u64) {
            self.input_tokens = n;
        }
        if let Some(n) = u.get("output_tokens").and_then(Value::as_u64) {
            self.output_tokens = n;
        }
        if let Some(n) = u.get("cache_read_input_tokens").and_then(Value::as_u64) {
            self.cache_read = Some(n);
        }
    }

    pub(crate) fn finish(self) -> Folded {
        let mut env = json!({
            "content": [{ "type": "text", "text": self.text }],
            "stop_reason": self.stop_reason,
        });
        if let Some(m) = self.model {
            env["model"] = json!(m);
        }
        if self.saw_usage {
            let mut usage = json!({
                "input_tokens": self.input_tokens,
                "output_tokens": self.output_tokens,
            });
            if let Some(c) = self.cache_read {
                usage["cache_read_input_tokens"] = json!(c);
            }
            env["usage"] = usage;
        }
        Folded {
            envelope: env,
            emitted_text: self.emitted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_data_fields_across_lines_and_comments() {
        let raw = ": keep-alive\n\nevent: x\ndata: one\n\ndata: two\ndata: three\n\ndata: tail";
        let mut got = Vec::new();
        read_sse(raw.as_bytes(), |d| {
            got.push(d.to_owned());
            true
        })
        .unwrap();
        assert_eq!(got, vec!["one", "two\nthree", "tail"]);
    }

    #[test]
    fn chat_folder_matches_blocking_envelope() {
        let mut seen = String::new();
        let mut sink = |t: &str| seen.push_str(t);
        let mut f = ChatStreamFolder::new(&mut sink);
        for d in [
            r#"{"model":"m1","choices":[{"delta":{"role":"assistant","content":"He"}}]}"#,
            r#"{"choices":[{"delta":{"content":"llo"},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#,
        ] {
            assert!(f.feed(d));
        }
        assert!(!f.feed("[DONE]"));
        let folded = f.finish();
        assert!(folded.emitted_text);
        let env = folded.envelope;
        assert_eq!(env["model"], "m1");
        assert_eq!(env["choices"][0]["message"]["content"], "Hello");
        assert_eq!(env["choices"][0]["finish_reason"], "stop");
        assert_eq!(env["usage"]["total_tokens"], 5);
        assert_eq!(seen, "Hello");
    }

    #[test]
    fn messages_folder_matches_blocking_envelope() {
        let mut seen = String::new();
        let mut sink = |t: &str| seen.push_str(t);
        let mut f = MessagesStreamFolder::new(&mut sink);
        for d in [
            r#"{"type":"message_start","message":{"model":"m2","usage":{"input_tokens":7,"output_tokens":1,"cache_read_input_tokens":4}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi "}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"there"}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":9}}"#,
        ] {
            assert!(f.feed(d));
        }
        assert!(!f.feed(r#"{"type":"message_stop"}"#));
        let env = f.finish().envelope;
        assert_eq!(env["content"][0]["text"], "Hi there");
        assert_eq!(env["usage"]["input_tokens"], 7);
        assert_eq!(env["usage"]["output_tokens"], 9);
        assert_eq!(env["usage"]["cache_read_input_tokens"], 4);
        assert_eq!(env["stop_reason"], "end_turn");
        assert_eq!(seen, "Hi there");
    }
}
