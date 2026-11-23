//! Streaming variants of the two HTTP dialects against a local fake server: the streamed call
//! returns what the blocking call returns (same text, same usage) and only asks for SSE when
//! the caller passes a sink.

use monoagens_core::ModelPurpose;
use monoagens_protocol::EventId;
use monoagens_reason::{
    MessagesModelConfig, MessagesModelProvider, ModelProvider, ModelRequest, OpenAiModelConfig,
    OpenAiModelProvider,
};
use serde_json::{Value, json};

use crate::fake_http::{FakeServer, Reply};

const OUTPUT: &str = r#"{"protocol_version":{"major":1,"minor":0},"acts":[]}"#;

fn request() -> ModelRequest {
    ModelRequest::text(EventId::new(7), ModelPurpose::Reason, "test-model", "sys", "hello")
}

fn sse(events: &[String]) -> Reply {
    let mut body = String::new();
    for e in events {
        body.push_str("data: ");
        body.push_str(e);
        body.push_str("\n\n");
    }
    let mut r = Reply::json(200, &body);
    r.headers.push(("Content-Type".to_owned(), "text/event-stream".to_owned()));
    r
}

fn chat_events() -> Vec<String> {
    let (a, b) = OUTPUT.split_at(OUTPUT.len() / 2);
    vec![
        json!({"model":"m","choices":[{"delta":{"role":"assistant","content":a}}]}).to_string(),
        json!({"choices":[{"delta":{"content":b},"finish_reason":"stop"}]}).to_string(),
        json!({"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}})
            .to_string(),
        "[DONE]".to_owned(),
    ]
}

fn messages_events() -> Vec<String> {
    let (a, b) = OUTPUT.split_at(OUTPUT.len() / 2);
    vec![
        json!({"type":"message_start","message":{"model":"m","usage":{"input_tokens":7,"output_tokens":1}}}).to_string(),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":a}}).to_string(),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":b}}).to_string(),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}).to_string(),
        json!({"type":"message_stop"}).to_string(),
    ]
}

fn chat_server() -> FakeServer {
    FakeServer::start(|req| {
        let body: Value = serde_json::from_str(&req.body).unwrap_or(Value::Null);
        if body.get("stream") == Some(&json!(true)) {
            sse(&chat_events())
        } else {
            Reply::json(
                200,
                &json!({"model":"m","choices":[{"message":{"content":OUTPUT},"finish_reason":"stop"}],
                    "usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}})
                .to_string(),
            )
        }
    })
}

fn messages_server() -> FakeServer {
    FakeServer::start(|req| {
        let body: Value = serde_json::from_str(&req.body).unwrap_or(Value::Null);
        if body.get("stream") == Some(&json!(true)) {
            sse(&messages_events())
        } else {
            Reply::json(
                200,
                &json!({"model":"m","content":[{"type":"text","text":OUTPUT}],"stop_reason":"end_turn",
                    "usage":{"input_tokens":7,"output_tokens":3}})
                .to_string(),
            )
        }
    })
}

#[test]
fn openai_chat_streamed_call_equals_blocking_call() {
    let srv = chat_server();
    let provider =
        OpenAiModelProvider::new(OpenAiModelConfig::new(format!("{}/v1/chat/completions", srv.url()), None))
            .expect("provider");
    let blocking = provider.invoke(&request()).expect("blocking");
    let mut seen = String::new();
    let streamed = provider
        .invoke_streaming(&request(), &mut |t| seen.push_str(t))
        .expect("streamed");

    assert_eq!(seen, OUTPUT, "deltas concatenate to the model output");
    assert_eq!(streamed.raw_text, blocking.raw_text);
    assert_eq!(streamed.output, blocking.output);
    assert_eq!(streamed.record.usage, blocking.record.usage, "usage recorded as before");
    assert_eq!(streamed.record.usage.expect("usage").total_tokens, 10);

    let reqs = srv.captured();
    assert_eq!(reqs.len(), 2);
    let b0: Value = serde_json::from_str(&reqs[0].body).unwrap();
    let b1: Value = serde_json::from_str(&reqs[1].body).unwrap();
    assert!(b0.get("stream").is_none(), "blocking request carries no stream flag");
    assert_eq!(b1["stream"], json!(true));
    assert_eq!(b1["stream_options"]["include_usage"], json!(true));
}

#[test]
fn messages_streamed_call_equals_blocking_call() {
    let srv = messages_server();
    let provider =
        MessagesModelProvider::new(MessagesModelConfig::new(format!("{}/v1/messages", srv.url()), None))
            .expect("provider");
    let blocking = provider.invoke(&request()).expect("blocking");
    let mut seen = String::new();
    let streamed = provider
        .invoke_streaming(&request(), &mut |t| seen.push_str(t))
        .expect("streamed");

    assert_eq!(seen, OUTPUT);
    assert_eq!(streamed.raw_text, blocking.raw_text);
    assert_eq!(streamed.output, blocking.output);
    assert_eq!(streamed.record.usage, blocking.record.usage);
    let reqs = srv.captured();
    let b0: Value = serde_json::from_str(&reqs[0].body).unwrap();
    let b1: Value = serde_json::from_str(&reqs[1].body).unwrap();
    assert!(b0.get("stream").is_none());
    assert_eq!(b1["stream"], json!(true));
}

#[test]
fn default_invoke_streaming_falls_back_to_blocking() {
    use monoagens_reason::{ScriptedModelProvider, ScriptedReply};
    let p = ScriptedModelProvider::from_replies([ScriptedReply::text(OUTPUT)]);
    let mut n = 0usize;
    let out = p.invoke_streaming(&request(), &mut |_| n += 1).expect("call");
    assert_eq!(out.raw_text, OUTPUT);
    assert!(n > 0, "the scripted provider streams its text in chunks");
    assert_eq!(p.streamed_calls(), 1);
}

#[test]
fn a_server_that_ignores_stream_true_is_read_as_a_plain_body() {
    // The fake always answers a blocking JSON body, whatever the request asked for.
    let srv = FakeServer::start(|_| {
        Reply::json(
            200,
            &json!({"model":"m","choices":[{"message":{"content":OUTPUT},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}})
            .to_string(),
        )
    });
    let provider =
        OpenAiModelProvider::new(OpenAiModelConfig::new(format!("{}/v1/chat/completions", srv.url()), None))
            .expect("provider");
    let mut seen = String::new();
    let out = provider.invoke_streaming(&request(), &mut |t| seen.push_str(t)).expect("call");
    assert_eq!(out.raw_text, OUTPUT);
    assert!(seen.is_empty(), "no deltas without an event stream");
    assert_eq!(out.record.usage.expect("usage").total_tokens, 10);
}
