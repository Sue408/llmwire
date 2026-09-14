use llmwire::codec::{Chat, Messages, ProtocolCodec};
use llmwire::ids::OpaqueKind;
use llmwire::ir::{
    AssistantOutput, Choice, Conversation, Finish, Part, RawJson, Reasoning, Role, Sampling,
    StopReason, Thinking, ToolChoice, ToolDef, ToolId, ToolResult, ToolResultContent, ToolUse,
    ToolUseKind, Turn, Usage,
};
use serde_json::Value;

#[test]
fn messages_request_roundtrip() {
    let arguments = r#"{"b":1,"a":2}"#;
    let parameters = r#"{"type":"object","properties":{"q":{"type":"string"}}}"#;
    let conversation = Conversation {
        system: vec![Part::Text("system".into())],
        turns: vec![
            Turn {
                role: Role::User,
                parts: vec![Part::Text("hello".into())],
            },
            Turn {
                role: Role::Assistant,
                parts: vec![
                    Part::Text("answer".into()),
                    Part::ToolUse(ToolUse {
                        id: ToolId("toolu_01".into()),
                        name: "lookup".into(),
                        arguments: RawJson::from_raw(arguments),
                        kind: ToolUseKind::Client,
                    }),
                ],
            },
            Turn {
                role: Role::User,
                parts: vec![
                    Part::ToolResult(ToolResult {
                        tool_use_id: ToolId("toolu_01".into()),
                        content: ToolResultContent::Text("ok".into()),
                    }),
                    Part::Text("thanks".into()),
                ],
            },
        ],
        tools: vec![ToolDef {
            name: "lookup".into(),
            description: Some("lookup".into()),
            parameters: RawJson::from_raw(parameters),
            strict: None,
        }],
        tool_choice: ToolChoice::Named("lookup".into()),
        sampling: Sampling {
            temperature: Some(0.5),
            top_p: Some(0.8),
            top_k: Some(20),
            max_output_tokens: Some(128),
            stop: vec!["done".into()],
            ..Sampling::default()
        },
        reasoning: Reasoning {
            enabled: true,
            effort: None,
            budget_tokens: Some(256),
        },
    };

    let encoded = Messages.encode_request(&conversation).unwrap();
    let decoded = Messages.decode_request(&encoded).unwrap();
    let reencoded = Messages.encode_request(&decoded).unwrap();

    assert_eq!(reencoded, encoded);
    assert_eq!(decoded.tools[0].parameters.raw(), parameters);
    let tool_use = decoded.turns[1]
        .parts
        .iter()
        .find_map(|part| match part {
            Part::ToolUse(tool_use) => Some(tool_use),
            _ => None,
        })
        .unwrap();
    assert_eq!(tool_use.id.0.as_ref(), "toolu_01");
    assert_eq!(tool_use.arguments.raw(), arguments);
}

#[test]
fn messages_cache_control_roundtrip() {
    let body = br#"{
        "model":"claude-test",
        "max_tokens":64,
        "system":[{"type":"text","text":"sys","cache_control":{"type":"ephemeral"}}],
        "messages":[{
            "role":"user",
            "content":[{"type":"text","text":"hello","cache_control":{"type":"ephemeral"}}]
        }]
    }"#;

    let conversation = Messages.decode_request(body).unwrap();
    let encoded = Messages.encode_request(&conversation).unwrap();
    let output: Value = serde_json::from_slice(&encoded).unwrap();

    assert_eq!(output["system"][0]["text"], "sys");
    assert_eq!(output["system"][0]["cache_control"]["type"], "ephemeral");
    assert_eq!(output["messages"][0]["content"][0]["text"], "hello");
    assert_eq!(
        output["messages"][0]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
}

#[test]
fn chat_messages_tool_id_roundtrip() {
    let body = br#"{
        "model":"gpt-test",
        "messages":[
            {"role":"system","content":"system"},
            {"role":"user","content":"hello"},
            {"role":"assistant","content":null,"tool_calls":[{"id":"call_AbC_01","type":"function","function":{"name":"lookup","arguments":"{\"b\":1,\"a\":2}"}}]},
            {"role":"tool","tool_call_id":"call_AbC_01","content":"ok"}
        ],
        "tools":[{"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}]
    }"#;

    let ir = Chat.decode_request(body).unwrap();
    let messages = Messages.encode_request(&ir).unwrap();
    let ir = Messages.decode_request(&messages).unwrap();
    let chat = Chat.encode_request(&ir).unwrap();
    let output: Value = serde_json::from_slice(&chat).unwrap();

    let tool_call = &output["messages"][2]["tool_calls"][0];
    assert_eq!(tool_call["id"], "call_AbC_01");
    assert_eq!(tool_call["function"]["arguments"], r#"{"b":1,"a":2}"#);
    assert_eq!(output["messages"][3]["tool_call_id"], "call_AbC_01");
}

#[test]
fn messages_response_roundtrip() {
    let body = br#"{
        "id":"msg_test",
        "type":"message",
        "role":"assistant",
        "model":"claude-test",
        "content":[
            {"type":"thinking","thinking":"reason","signature":"sig_123"},
            {"type":"redacted_thinking","data":"redacted_123"},
            {"type":"text","text":"answer"},
            {"type":"tool_use","id":"srvtoolu_01","name":"search","input":{"q":"x"}}
        ],
        "stop_reason":"tool_use",
        "usage":{"input_tokens":10,"output_tokens":5}
    }"#;

    let output = Messages.decode_response(body).unwrap();
    assert_eq!(output.choices[0].finish.canonical, StopReason::ToolUse);
    assert_eq!(output.usage.input, Some(10));
    assert_eq!(output.usage.output, Some(5));

    let signature = output.choices[0]
        .parts
        .iter()
        .find_map(|part| match part {
            Part::Thinking(Thinking { signature, .. }) => signature.as_ref(),
            _ => None,
        })
        .unwrap();
    assert_eq!(signature.kind, OpaqueKind::AnthropicThinkingSignature);
    assert_eq!(signature.bytes.as_ref(), b"sig_123");
    let tool_kind = output.choices[0]
        .parts
        .iter()
        .find_map(|part| match part {
            Part::ToolUse(tool_use) => Some(&tool_use.kind),
            _ => None,
        })
        .unwrap();
    assert_eq!(tool_kind, &ToolUseKind::Server);

    let encoded = Messages.encode_response(&output).unwrap();
    let decoded = Messages.decode_response(&encoded).unwrap();
    assert_eq!(decoded.choices[0].finish.canonical, StopReason::ToolUse);
    assert_eq!(decoded.choices[0].parts.len(), 4);
}

#[test]
fn messages_response_maps_usage() {
    let output = AssistantOutput {
        id: None,
        model: None,
        choices: vec![Choice {
            index: 0,
            parts: vec![Part::Text("answer".into())],
            finish: Finish {
                canonical: StopReason::EndTurn,
                provider_raw: "end_turn".into(),
            },
        }],
        usage: Usage {
            input: Some(15),
            output: Some(5),
            cached: 3,
            cache_creation: 2,
            ..Usage::default()
        },
    };

    let encoded = Messages.encode_response(&output).unwrap();
    let decoded = Messages.decode_response(&encoded).unwrap();

    assert_eq!(decoded.usage.input, Some(15));
    assert_eq!(decoded.usage.cached, 3);
    assert_eq!(decoded.usage.cache_creation, 2);
}
