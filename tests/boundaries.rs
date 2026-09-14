use llmwire::codec::{Chat, Messages, ProtocolCodec, Responses};
use llmwire::framing::{SseFrame, SseFramer};
use llmwire::ids::OpaqueKind;
use llmwire::ir::{Conversation, Part, Role, Sampling, StreamState, Turn};
use llmwire::{Error, Report, Severity};

#[test]
fn sse_crlf_boundary_can_split_between_carriage_return_and_line_feed() {
    let mut framer = SseFramer::new();
    let mut out = Vec::new();

    framer.feed(b"event: message\r", &mut out).unwrap();
    assert!(out.is_empty());
    framer.feed(b"\ndata: hello\r", &mut out).unwrap();
    assert!(out.is_empty());
    framer.feed(b"\n\r\n", &mut out).unwrap();

    assert_eq!(
        out,
        vec![SseFrame {
            event: Some("message".to_owned()),
            data: Some("hello".to_owned()),
        }]
    );
}

#[test]
fn sse_buffer_limit_allows_exact_incomplete_limit() {
    let mut framer = SseFramer::with_limit(5);
    let mut out = Vec::new();

    framer.feed(b"data:", &mut out).unwrap();
    assert!(out.is_empty());
    framer.feed(b"\n\n", &mut out).unwrap();

    assert_eq!(
        out,
        vec![SseFrame {
            event: None,
            data: Some(String::new()),
        }]
    );
}

#[test]
fn chat_delta_null_does_not_open_part() {
    let frame = SseFrame {
        event: None,
        data: Some(
            r#"{"id":"chatcmpl-null","model":"model-a","choices":[{"index":0,"delta":null,"finish_reason":null}]}"#
                .to_owned(),
        ),
    };

    let decoded = Chat
        .decode_stream_frame(&frame, &StreamState::new())
        .unwrap();

    assert!(decoded
        .events
        .iter()
        .any(|event| matches!(event, llmwire::ir::Event::MessageStart { .. })));
    assert!(!decoded
        .events
        .iter()
        .any(|event| matches!(event, llmwire::ir::Event::PartStart { .. })));
}

#[test]
fn messages_redacted_thinking_bytes_are_preserved() {
    let body = br#"{
        "id":"msg_1",
        "type":"message",
        "role":"assistant",
        "model":"claude-test",
        "content":[{"type":"redacted_thinking","data":"redacted_123"}],
        "stop_reason":"end_turn",
        "usage":{"input_tokens":1,"output_tokens":1}
    }"#;

    let output = Messages.decode_response(body).unwrap();
    let opaque = output.choices[0]
        .parts
        .iter()
        .find_map(|part| match part {
            Part::Opaque(opaque) => Some(opaque),
            _ => None,
        })
        .expect("expected redacted thinking opaque");
    assert_eq!(opaque.kind, OpaqueKind::AnthropicRedactedThinking);
    assert_eq!(opaque.bytes.as_ref(), b"redacted_123");

    let encoded = Messages.encode_response(&output).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(value["content"][0]["type"], "redacted_thinking");
    assert_eq!(value["content"][0]["data"], "redacted_123");
}

#[test]
fn responses_max_output_tokens_boundaries_are_enforced() {
    let too_small = Responses.decode_request(br#"{"input":"hi","max_output_tokens":15}"#);
    assert!(
        matches!(too_small, Err(Error::InvalidInput(message)) if message.contains("at least 16"))
    );

    let minimum = Responses
        .decode_request(br#"{"input":"hi","max_output_tokens":16}"#)
        .unwrap();
    assert_eq!(minimum.sampling.max_output_tokens, Some(16));

    let conversation = Conversation {
        turns: vec![Turn {
            role: Role::User,
            parts: vec![Part::Text("hi".to_owned())],
        }],
        sampling: Sampling {
            max_output_tokens: Some(15),
            ..Sampling::default()
        },
        ..Conversation::default()
    };
    let encoded = Responses.encode_request(&conversation);
    assert!(
        matches!(encoded, Err(Error::InvalidInput(message)) if message.contains("at least 16"))
    );
}

#[test]
fn chat_empty_and_long_system_are_preserved() {
    let long_system = "s".repeat(64 * 1024);
    let body = serde_json::json!({
        "messages": [
            {"role": "system", "content": ""},
            {"role": "system", "content": long_system},
            {"role": "user", "content": "hi"}
        ]
    })
    .to_string()
    .into_bytes();

    let conversation = Chat.decode_request(&body).unwrap();

    assert_eq!(conversation.system.len(), 2);
    assert!(matches!(&conversation.system[0], Part::Text(text) if text.is_empty()));
    assert!(matches!(&conversation.system[1], Part::Text(text) if text.len() == 64 * 1024));
}

#[test]
fn chat_late_system_is_reported_after_promotion() {
    let body = br#"{
        "messages": [
            {"role":"user","content":"hello"},
            {"role":"system","content":"late"}
        ]
    }"#;
    let mut report = Report::new();

    let conversation = Chat.decode_request_with_report(body, &mut report).unwrap();

    assert_eq!(conversation.system.len(), 1);
    assert_eq!(conversation.turns.len(), 1);
    assert!(report.warnings.iter().any(|entry| {
        entry.field.as_ref() == "request.messages[1].role" && entry.severity == Severity::Degraded
    }));
}

#[test]
fn responses_late_system_is_reported_after_promotion() {
    let body = serde_json::json!({
        "input": [
            {"type":"message","role":"user","content":"hello"},
            {"type":"message","role":"developer","content":"late"}
        ]
    })
    .to_string()
    .into_bytes();
    let mut report = Report::new();

    let conversation = Responses
        .decode_request_with_report(&body, &mut report)
        .unwrap();

    assert_eq!(conversation.system.len(), 1);
    assert_eq!(conversation.turns.len(), 1);
    assert!(report.warnings.iter().any(|entry| {
        entry.field.as_ref() == "request.input[1].role" && entry.severity == Severity::Degraded
    }));
}
