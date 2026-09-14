use llmwire::codec::{Chat, ProtocolCodec};
use llmwire::ids::OpaqueKind;
use llmwire::ir::{
    AssistantOutput, Choice, Conversation, Finish, ImageRef, ImageSource, Part, RawJson, Reasoning,
    ReasoningEffort, Role, Sampling, StopReason, Thinking, ToolChoice, ToolDef, ToolId, ToolResult,
    ToolResultContent, ToolUse, ToolUseKind, Turn, Usage,
};

#[test]
fn constructs_core_types() {
    let schema = RawJson::from_raw(r#"{"type":"object"}"#);
    let opaque = llmwire::ir::Opaque {
        kind: OpaqueKind::ResponsesEncryptedReasoning,
        bytes: b"opaque".to_vec().into_boxed_slice(),
    };

    let tool = ToolDef {
        name: "lookup".into(),
        description: Some("lookup a value".into()),
        parameters: schema.clone(),
        strict: Some(true),
    };

    let tool_use = ToolUse {
        id: ToolId("call_01".into()),
        name: "lookup".into(),
        arguments: RawJson::from_raw(r#"{"key":"weather"}"#),
        kind: ToolUseKind::Client,
    };

    let tool_result = ToolResult {
        tool_use_id: tool_use.id.clone(),
        content: ToolResultContent::Parts(vec![Part::Text("ok".into())]),
    };

    assert_eq!(tool_result.tool_use_id.0.as_ref(), "call_01");

    let conversation = Conversation {
        system: vec![Part::Text("system".into())],
        turns: vec![Turn {
            role: Role::User,
            parts: vec![
                Part::Image(ImageRef {
                    source: ImageSource::Base64 {
                        media_type: "image/png".into(),
                        data: "AAAA".into(),
                    },
                    detail: Some("high".into()),
                }),
                Part::ToolUse(tool_use),
                Part::ToolResult(tool_result),
                Part::Thinking(Thinking {
                    text: "reasoning".into(),
                    signature: Some(opaque),
                }),
            ],
        }],
        tools: vec![tool],
        tool_choice: ToolChoice::Named("lookup".into()),
        sampling: Sampling {
            max_output_tokens: Some(128),
            ..Sampling::default()
        },
        reasoning: Reasoning {
            enabled: true,
            effort: Some(ReasoningEffort::High),
            budget_tokens: Some(64),
        },
    };

    let output = AssistantOutput {
        choices: vec![Choice {
            index: 0,
            parts: vec![Part::Text("answer".into())],
            finish: Finish {
                canonical: StopReason::EndTurn,
                provider_raw: "stop".into(),
            },
        }],
        usage: Usage {
            input: Some(10),
            output: Some(20),
            ..Usage::default()
        },
    };

    assert_eq!(conversation.turns.len(), 1);
    assert_eq!(conversation.tools.len(), 1);
    assert_eq!(output.choices.len(), 1);
    assert_eq!(output.choices[0].finish.canonical, StopReason::EndTurn);
}

#[test]
fn image_source_distinguishes_remote_url_and_data_uri() {
    let data_uri = br#"{
        "messages":[{
            "role":"user",
            "content":[{
                "type":"image_url",
                "image_url":{"url":"data:image/png;base64,AAAA"}
            }]
        }]
    }"#;
    let conversation = Chat.decode_request(data_uri).unwrap();

    assert!(matches!(
        &conversation.turns[0].parts[0],
        Part::Image(ImageRef {
            source: ImageSource::Base64 { media_type, data },
            detail: None,
        }) if media_type.as_ref() == "image/png" && data.as_ref() == "AAAA"
    ));

    let remote_url = br#"{
        "messages":[{
            "role":"user",
            "content":[{
                "type":"image_url",
                "image_url":{"url":"https://example.test/image.png"}
            }]
        }]
    }"#;
    let conversation = Chat.decode_request(remote_url).unwrap();

    assert!(matches!(
        &conversation.turns[0].parts[0],
        Part::Image(ImageRef {
            source: ImageSource::RemoteUrl(url),
            detail: None,
        }) if url.as_ref() == "https://example.test/image.png"
    ));
}

#[test]
fn image_data_uri_without_base64_marker_is_rejected() {
    let body = br#"{
        "messages":[{
            "role":"user",
            "content":[{
                "type":"image_url",
                "image_url":{"url":"data:image/png,AAAA"}
            }]
        }]
    }"#;

    assert!(matches!(
        Chat.decode_request(body),
        Err(llmwire::Error::InvalidInput(message)) if message.contains(";base64")
    ));
}

#[test]
fn raw_json_preserves_raw_text() {
    let raw = RawJson::from_raw(r#"{"b":1,"a":2}"#);

    assert_eq!(raw.raw(), r#"{"b":1,"a":2}"#);
    assert_eq!(
        raw.parsed()
            .and_then(|value| value.get("b"))
            .and_then(|value| value.as_i64()),
        Some(1)
    );

    let invalid = RawJson::from_raw(r#"{"a""#);

    assert_eq!(invalid.raw(), r#"{"a""#);
    assert!(invalid.parsed().is_none());
}

#[test]
fn opaque_debug_does_not_leak_bytes() {
    let opaque = llmwire::ir::Opaque {
        kind: OpaqueKind::AnthropicRedactedThinking,
        bytes: b"secret".to_vec().into_boxed_slice(),
    };

    let debug = format!("{opaque:?}");

    assert!(debug.contains("len"));
    assert!(!debug.contains("secret"));
    assert_eq!(opaque.bytes.as_ref(), b"secret");
}

#[test]
fn usage_keeps_unknown_distinct_from_zero() {
    let unknown = Usage::default();
    let zero = Usage {
        input: Some(0),
        ..Usage::default()
    };

    assert_eq!(unknown.input, None);
    assert_eq!(zero.input, Some(0));
    assert_ne!(unknown.input, zero.input);
}

#[test]
fn default_types_are_constructible() {
    let conversation = Conversation::default();
    let output = AssistantOutput::default();

    assert!(conversation.turns.is_empty());
    assert!(output.choices.is_empty());
}
