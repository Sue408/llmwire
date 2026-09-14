use llmwire::codec::{Chat, ProtocolCodec};
use llmwire::ir::{
    Conversation, Part, RawJson, Reasoning, ReasoningEffort, Role, Sampling, StopReason,
    ToolChoice, ToolDef, ToolId, ToolResult, ToolResultContent, ToolUse, ToolUseKind, Turn, Usage,
};
use proptest::prelude::*;

proptest! {
    #[test]
    fn chat_request_roundtrip(
        system in "[a-z]{0,12}",
        user in "[a-z]{0,12}",
        assistant in "[a-z]{0,12}",
        tool_id in "call_[a-z0-9]{1,8}",
        tool_name in "[a-z]{1,8}",
        arg_key in "[a-z]{1,8}",
        arg_value in "[a-z0-9]{0,12}",
        followup in "[a-z]{0,12}",
    ) {
        let arguments = format!(r#"{{"{}":"{}"}}"#, arg_key, arg_value);
        let parameters = format!(
            r#"{{"type":"object","properties":{{"{}":{{"type":"string"}}}}}}"#,
            arg_key
        );
        let conversation = Conversation {
            system: vec![Part::Text(system)],
            turns: vec![
                Turn {
                    role: Role::User,
                    parts: vec![Part::Text(user)],
                },
                Turn {
                    role: Role::Assistant,
                    parts: vec![
                        Part::Text(assistant),
                        Part::ToolUse(ToolUse {
                            id: ToolId(tool_id.clone().into()),
                            name: tool_name.clone().into(),
                            arguments: RawJson::from_raw(arguments.clone()),
                            kind: ToolUseKind::Client,
                        }),
                    ],
                },
                Turn {
                    role: Role::User,
                    parts: vec![
                        Part::ToolResult(ToolResult {
                            tool_use_id: ToolId(tool_id.clone().into()),
                            content: ToolResultContent::Text("ok".into()),
                        }),
                        Part::Text(followup),
                    ],
                },
            ],
            tools: vec![ToolDef {
                name: tool_name.into(),
                description: Some("lookup".into()),
                parameters: RawJson::from_raw(parameters.clone()),
                strict: Some(true),
            }],
            tool_choice: ToolChoice::Auto,
            sampling: Sampling {
                temperature: Some(0.5),
                max_output_tokens: Some(128),
                n: Some(1),
                ..Sampling::default()
            },
            reasoning: Reasoning {
                enabled: true,
                effort: Some(ReasoningEffort::Low),
                budget_tokens: None,
            },
        };

        let encoded = Chat.encode_request(&conversation).unwrap();
        let decoded = Chat.decode_request(&encoded).unwrap();

        prop_assert_eq!(decoded.system.len(), 1);
        prop_assert_eq!(decoded.turns.len(), 3);
        prop_assert_eq!(decoded.tools.len(), 1);
        prop_assert_eq!(decoded.tools[0].parameters.raw(), parameters.as_str());
        let decoded_tool_use = decoded.turns[1]
            .parts
            .iter()
            .find_map(|part| match part {
                Part::ToolUse(tool_use) => Some(tool_use),
                _ => None,
            })
            .unwrap();
        prop_assert_eq!(decoded_tool_use.arguments.raw(), arguments.as_str());

        let reencoded = Chat.encode_request(&decoded).unwrap();
        prop_assert_eq!(reencoded, encoded);
    }
}

#[test]
fn chat_response_decodes_and_roundtrips() {
    let body = br#"{
        "id":"chatcmpl-test",
        "object":"chat.completion",
        "created":1,
        "model":"test-model",
        "choices":[{
            "index":0,
            "message":{
                "role":"assistant",
                "content":"answer",
                "tool_calls":[{
                    "id":"call_01",
                    "type":"function",
                    "function":{"name":"lookup","arguments":"{\"b\":1,\"a\":2}"}
                }]
            },
            "finish_reason":"tool_calls"
        }],
        "usage":{
            "prompt_tokens":10,
            "completion_tokens":20,
            "prompt_tokens_details":{"cached_tokens":3},
            "completion_tokens_details":{"reasoning_tokens":4}
        }
    }"#;

    let output = Chat.decode_response(body).unwrap();

    assert_eq!(output.choices.len(), 1);
    assert_eq!(output.choices[0].finish.canonical, StopReason::ToolUse);
    assert_eq!(output.choices[0].finish.provider_raw.as_ref(), "tool_calls");
    assert_eq!(output.usage.input, Some(10));
    assert_eq!(output.usage.output, Some(20));
    assert_eq!(output.usage.cached, 3);
    assert_eq!(output.usage.reasoning, 4);

    let encoded = Chat.encode_response(&output).unwrap();
    let decoded = Chat.decode_response(&encoded).unwrap();
    let tool_use = decoded.choices[0]
        .parts
        .iter()
        .find_map(|part| match part {
            Part::ToolUse(tool_use) => Some(tool_use),
            _ => None,
        })
        .unwrap();

    assert_eq!(tool_use.arguments.raw(), r#"{"b":1,"a":2}"#);
    assert_eq!(
        decoded.choices[0].finish.canonical,
        output.choices[0].finish.canonical
    );
    assert_eq!(
        decoded.choices[0].finish.provider_raw,
        output.choices[0].finish.provider_raw
    );
}

#[test]
fn tool_arguments_keep_raw_json_string() {
    let body = br#"{
        "messages":[{
            "role":"assistant",
            "content":null,
            "tool_calls":[{
                "id":"call_01",
                "type":"function",
                "function":{"name":"lookup","arguments":"{\"b\":1,\"a\":2}"}
            }]
        }]
    }"#;

    let conversation = Chat.decode_request(body).unwrap();
    let tool_use = conversation.turns[0]
        .parts
        .iter()
        .find_map(|part| match part {
            Part::ToolUse(tool_use) => Some(tool_use),
            _ => None,
        })
        .unwrap();

    assert_eq!(tool_use.id, ToolId("call_01".into()));
    assert_eq!(tool_use.arguments.raw(), r#"{"b":1,"a":2}"#);
}

#[test]
fn usage_defaults_to_unknown_not_zero() {
    let body = br#"{
        "choices":[{
            "index":0,
            "message":{"role":"assistant","content":"ok"},
            "finish_reason":"stop"
        }]
    }"#;

    let output = Chat.decode_response(body).unwrap();

    assert_eq!(output.usage.input, None);
    assert_eq!(output.usage.output, None);
    assert_eq!(output.choices[0].finish.canonical, StopReason::EndTurn);
    assert_eq!(output.choices[0].finish.provider_raw.as_ref(), "stop");
    assert_eq!(output.usage.input, Usage::default().input);
    assert_eq!(output.usage.output, Usage::default().output);
    assert_eq!(output.usage.cached, Usage::default().cached);
    assert_eq!(output.usage.cache_creation, Usage::default().cache_creation);
    assert_eq!(output.usage.reasoning, Usage::default().reasoning);
}
