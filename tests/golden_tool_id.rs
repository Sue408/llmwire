use llmwire::codec::{Chat, Messages, ProtocolCodec};
use llmwire::ir::{
    Conversation, Part, RawJson, Role, ToolId, ToolResult, ToolResultContent, ToolUse, ToolUseKind,
    Turn,
};
use serde_json::Value;

#[test]
fn chat_tool_id_is_byte_exact() {
    let id = "call_AbC-01.x:Z";
    let body = format!(
        r#"{{
            "messages":[
                {{"role":"assistant","content":null,"tool_calls":[{{
                    "id":"{id}",
                    "type":"function",
                    "function":{{"name":"lookup","arguments":"{{\"x\":1}}"}}
                }}]}},
                {{"role":"tool","tool_call_id":"{id}","content":"ok"}}
            ]
        }}"#
    );

    let conversation = Chat.decode_request(body.as_bytes()).unwrap();
    let tool_use = conversation.turns[0]
        .parts
        .iter()
        .find_map(|part| match part {
            Part::ToolUse(tool_use) => Some(tool_use),
            _ => None,
        })
        .unwrap();
    let tool_result = conversation.turns[1]
        .parts
        .iter()
        .find_map(|part| match part {
            Part::ToolResult(result) => Some(result),
            _ => None,
        })
        .unwrap();

    assert_eq!(tool_use.id, ToolId(id.into()));
    assert_eq!(tool_result.tool_use_id, tool_use.id);

    let encoded = Chat.encode_request(&conversation).unwrap();
    let output: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(output["messages"][0]["tool_calls"][0]["id"], id);
    assert_eq!(output["messages"][1]["tool_call_id"], id);
}

#[test]
fn messages_tool_ids_are_byte_exact() {
    let client_id = ToolId("toolu_AbC-01.x:Z".into());
    let server_id = ToolId("srvtoolu_AbC-01.x:Z".into());
    let conversation = Conversation {
        turns: vec![
            Turn {
                role: Role::Assistant,
                parts: vec![
                    Part::ToolUse(ToolUse {
                        id: client_id.clone(),
                        name: "lookup".into(),
                        arguments: RawJson::from_raw(r#"{"x":1}"#),
                        kind: ToolUseKind::Client,
                    }),
                    Part::ToolUse(ToolUse {
                        id: server_id.clone(),
                        name: "search".into(),
                        arguments: RawJson::from_raw(r#"{"q":"x"}"#),
                        kind: ToolUseKind::Server,
                    }),
                ],
            },
            Turn {
                role: Role::User,
                parts: vec![
                    Part::ToolResult(ToolResult {
                        tool_use_id: client_id.clone(),
                        content: ToolResultContent::Text("ok".into()),
                    }),
                    Part::ToolResult(ToolResult {
                        tool_use_id: server_id.clone(),
                        content: ToolResultContent::Text("ok".into()),
                    }),
                ],
            },
        ],
        ..Conversation::default()
    };

    let encoded = Messages.encode_request(&conversation).unwrap();
    let decoded = Messages.decode_request(&encoded).unwrap();
    let tool_uses = decoded.turns[0]
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::ToolUse(tool_use) => Some(tool_use),
            _ => None,
        })
        .collect::<Vec<_>>();
    let tool_results = decoded.turns[1]
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::ToolResult(result) => Some(result),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(tool_uses[0].id, client_id);
    assert_eq!(tool_uses[0].kind, ToolUseKind::Client);
    assert_eq!(tool_uses[1].id, server_id);
    assert_eq!(tool_uses[1].kind, ToolUseKind::Server);
    assert_eq!(tool_results[0].tool_use_id, tool_uses[0].id);
    assert_eq!(tool_results[1].tool_use_id, tool_uses[1].id);
}

#[test]
fn chat_messages_tool_id_roundtrip_is_byte_exact() {
    let id = "call_AbC-01.x:Z";
    let body = format!(
        r#"{{
            "messages":[
                {{"role":"assistant","content":null,"tool_calls":[{{
                    "id":"{id}",
                    "type":"function",
                    "function":{{"name":"lookup","arguments":"{{\"x\":1}}"}}
                }}]}},
                {{"role":"tool","tool_call_id":"{id}","content":"ok"}}
            ]
        }}"#
    );

    let ir = Chat.decode_request(body.as_bytes()).unwrap();
    let messages = Messages.encode_request(&ir).unwrap();
    let ir = Messages.decode_request(&messages).unwrap();
    let chat = Chat.encode_request(&ir).unwrap();
    let output: Value = serde_json::from_slice(&chat).unwrap();

    assert_eq!(output["messages"][0]["tool_calls"][0]["id"], id);
    assert_eq!(output["messages"][1]["tool_call_id"], id);
}
