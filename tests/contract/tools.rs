use llmwire::codec::{Chat, Messages, ProtocolCodec, Responses};
use llmwire::ids::OpaqueKind;
use llmwire::ir::{Part, Role, ToolResultContent, ToolUseKind};

#[test]
fn chat_tool_call_id_is_byte_exact() {
    let response = br#"{
        "id":"chatcmpl-1",
        "choices":[{
            "index":0,
            "message":{
                "role":"assistant",
                "tool_calls":[{
                    "id":"call_AbC.01",
                    "type":"function",
                    "function":{"name":"lookup","arguments":"{\"x\":1}"}
                }]
            },
            "finish_reason":"tool_calls"
        }]
    }"#;

    let output = Chat.decode_response(response).unwrap();
    let Part::ToolUse(tool_use) = &output.choices[0].parts[0] else {
        panic!("expected tool use");
    };
    assert_eq!(tool_use.id.0.as_ref(), "call_AbC.01");
    assert_eq!(tool_use.arguments.raw(), r#"{"x":1}"#);

    let encoded = Chat.encode_response(&output).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(
        value["choices"][0]["message"]["tool_calls"][0]["id"],
        "call_AbC.01"
    );
}

#[test]
fn messages_tool_results_aggregate_in_one_user_turn() {
    let request = serde_json::json!({
        "model": "claude-test",
        "max_tokens": 64,
        "messages": [{
            "role": "user",
            "content": [
                {"type":"tool_result","tool_use_id":"toolu_a","content":"a"},
                {"type":"tool_result","tool_use_id":"toolu_b","content":[{"type":"text","text":"b"}]},
                {"type":"text","text":"continue"}
            ]
        }]
    });
    let conversation = Messages
        .decode_request(&serde_json::to_vec(&request).unwrap())
        .unwrap();

    assert_eq!(conversation.turns.len(), 1);
    assert_eq!(conversation.turns[0].role, Role::User);
    let results = conversation.turns[0]
        .parts
        .iter()
        .filter(|part| matches!(part, Part::ToolResult(_)))
        .count();
    assert_eq!(results, 2);
    assert!(matches!(
        &conversation.turns[0].parts[1],
        Part::ToolResult(result)
            if matches!(&result.content, ToolResultContent::Parts(parts) if matches!(&parts[..], [Part::Text(text)] if text == "b"))
    ));
}

#[test]
fn messages_server_tool_prefix_is_preserved() {
    let response = br#"{
        "id":"msg_1",
        "type":"message",
        "role":"assistant",
        "model":"claude-test",
        "stop_reason":"tool_use",
        "content":[{"type":"tool_use","id":"srvtoolu_01","name":"web_search","input":{"query":"rust"}}]
    }"#;
    let output = Messages.decode_response(response).unwrap();
    let Part::ToolUse(tool_use) = &output.choices[0].parts[0] else {
        panic!("expected tool use");
    };
    assert_eq!(tool_use.id.0.as_ref(), "srvtoolu_01");
    assert_eq!(tool_use.kind, ToolUseKind::Server);
}

#[test]
fn responses_function_call_id_is_byte_exact() {
    let response = br#"{
        "id":"resp_1",
        "object":"response",
        "status":"completed",
        "model":"gpt-test",
        "output":[{
            "type":"function_call",
            "id":"fc_1",
            "call_id":"call_AbC.01",
            "name":"lookup",
            "arguments":"{\"x\":1}"
        }]
    }"#;
    let output = Responses.decode_response(response).unwrap();
    let encoded = Responses.encode_response(&output).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(value["output"][0]["call_id"], "call_AbC.01");
    assert_eq!(value["output"][0]["arguments"], r#"{"x":1}"#);
}

#[test]
fn responses_builtin_item_roundtrips_as_opaque() {
    let response = br#"{
        "id":"resp_1",
        "object":"response",
        "status":"completed",
        "model":"gpt-test",
        "output":[{
            "type":"web_search_call",
            "id":"ws_1",
            "status":"completed",
            "action":{"type":"search","query":"rust"}
        }]
    }"#;
    let output = Responses.decode_response(response).unwrap();
    let Part::Opaque(opaque) = &output.choices[0].parts[0] else {
        panic!("expected opaque item");
    };
    assert_eq!(opaque.kind, OpaqueKind::ProviderSpecific("responses_item"));

    let encoded = Responses.encode_response(&output).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(value["output"][0]["type"], "web_search_call");
    assert_eq!(value["output"][0]["id"], "ws_1");
}
