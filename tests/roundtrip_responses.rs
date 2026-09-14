use llmwire::codec::{ProtocolCodec, Responses};
use llmwire::ids::OpaqueKind;
use llmwire::ir::{Part, StopReason, ToolResultContent, ToolUseKind};
use serde_json::Value;

#[test]
fn responses_request_roundtrip() {
    let arguments = r#"{"b":1,"a":2}"#;
    let parameters = r#"{"type":"object","properties":{"q":{"type":"string"}}}"#;
    let body = format!(
        r#"{{
            "model":"gpt-test",
            "instructions":"system",
            "input":[
                {{"type":"message","role":"user","content":[{{"type":"input_text","text":"hello"}}]}},
                {{"type":"function_call","call_id":"call_AbC_01","name":"lookup","arguments":{arguments:?}}},
                {{"type":"function_call_output","call_id":"call_AbC_01","output":"ok"}},
                {{"type":"message","role":"user","content":[{{"type":"input_text","text":"thanks"}}]}}
            ],
            "tools":[{{"type":"function","name":"lookup","description":"lookup","parameters":{parameters},"strict":true}}],
            "tool_choice":{{"type":"function","name":"lookup"}},
            "reasoning":{{"effort":"high"}},
            "max_output_tokens":128,
            "temperature":0.5,
            "top_p":0.8
        }}"#
    );

    let conversation = Responses.decode_request(body.as_bytes()).unwrap();
    assert!(matches!(&conversation.system[0], Part::Text(text) if text == "system"));
    assert_eq!(conversation.turns.len(), 3);
    assert_eq!(conversation.tools[0].parameters.raw(), parameters);
    assert_eq!(
        conversation.reasoning.effort,
        Some(llmwire::ir::ReasoningEffort::High)
    );

    let tool_use = conversation.turns[1]
        .parts
        .iter()
        .find_map(|part| match part {
            Part::ToolUse(tool_use) => Some(tool_use),
            _ => None,
        })
        .unwrap();
    assert_eq!(tool_use.id.0.as_ref(), "call_AbC_01");
    assert_eq!(tool_use.name.as_ref(), "lookup");
    assert_eq!(tool_use.arguments.raw(), arguments);
    assert_eq!(tool_use.kind, ToolUseKind::Client);

    let tool_result = conversation.turns[2]
        .parts
        .iter()
        .find_map(|part| match part {
            Part::ToolResult(result) => Some(result),
            _ => None,
        })
        .unwrap();
    assert_eq!(tool_result.tool_use_id.0.as_ref(), "call_AbC_01");
    assert!(matches!(
        &tool_result.content,
        ToolResultContent::Text(text) if text.as_ref() == "ok"
    ));

    let encoded = Responses.encode_request(&conversation).unwrap();
    let decoded = Responses.decode_request(&encoded).unwrap();
    let reencoded = Responses.encode_request(&decoded).unwrap();
    assert_eq!(reencoded, encoded);
}

#[test]
fn responses_function_call_response_roundtrip() {
    let body = br#"{
        "id":"resp_1",
        "object":"response",
        "status":"completed",
        "model":"gpt-test",
        "output":[{
            "type":"function_call",
            "id":"fc_1",
            "call_id":"call_AbC_01",
            "name":"lookup",
            "arguments":"{\"b\":1,\"a\":2}",
            "status":"completed"
        }],
        "usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}
    }"#;

    let output = Responses.decode_response(body).unwrap();
    assert_eq!(output.choices[0].finish.canonical, StopReason::ToolUse);
    let tool_use = output.choices[0]
        .parts
        .iter()
        .find_map(|part| match part {
            Part::ToolUse(tool_use) => Some(tool_use),
            _ => None,
        })
        .unwrap();
    assert_eq!(tool_use.id.0.as_ref(), "call_AbC_01");
    assert_eq!(tool_use.arguments.raw(), r#"{"b":1,"a":2}"#);

    let encoded = Responses.encode_response(&output).unwrap();
    let value: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(value["status"], "completed");
    assert_eq!(value["output"][0]["type"], "function_call");
    assert_eq!(value["output"][0]["call_id"], "call_AbC_01");
    assert_eq!(value["output"][0]["arguments"], r#"{"b":1,"a":2}"#);

    let decoded = Responses.decode_response(&encoded).unwrap();
    assert_eq!(decoded.choices[0].finish.canonical, StopReason::ToolUse);
}

#[test]
fn responses_encrypted_content_is_byte_preserved() {
    let encrypted = "abc/def+==\u{0}";
    let body = br#"{
        "id":"resp_1",
        "object":"response",
        "status":"completed",
        "model":"gpt-test",
        "output":[{
            "type":"reasoning",
            "id":"rs_1",
            "summary":[],
            "encrypted_content":"abc/def+==\u0000",
            "status":"completed"
        }]
    }"#;

    let output = Responses.decode_response(body).unwrap();
    let opaque = output.choices[0]
        .parts
        .iter()
        .find_map(|part| match part {
            Part::Opaque(opaque) => Some(opaque),
            _ => None,
        })
        .unwrap();
    assert_eq!(opaque.kind, OpaqueKind::ResponsesEncryptedReasoning);
    assert_eq!(opaque.bytes.as_ref(), encrypted.as_bytes());

    let encoded = Responses.encode_response(&output).unwrap();
    let value: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(value["output"][0]["encrypted_content"], encrypted);
}

#[test]
fn responses_text_response_roundtrip() {
    let body = br#"{
        "id":"resp_1",
        "object":"response",
        "status":"completed",
        "model":"gpt-test",
        "output":[{
            "type":"message",
            "id":"msg_1",
            "role":"assistant",
            "status":"completed",
            "content":[{"type":"output_text","text":"hello","annotations":[]}]
        }],
        "usage":{
            "input_tokens":10,
            "output_tokens":5,
            "input_tokens_details":{"cached_tokens":3},
            "output_tokens_details":{"reasoning_tokens":2}
        }
    }"#;

    let output = Responses.decode_response(body).unwrap();
    assert_eq!(output.choices[0].finish.canonical, StopReason::EndTurn);
    assert!(matches!(&output.choices[0].parts[..], [Part::Text(text)] if text == "hello"));
    assert_eq!(output.usage.input, Some(10));
    assert_eq!(output.usage.output, Some(5));
    assert_eq!(output.usage.cached, 3);
    assert_eq!(output.usage.reasoning, 2);

    let encoded = Responses.encode_response(&output).unwrap();
    let value: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(value["output"][0]["content"][0]["type"], "output_text");
    assert_eq!(value["output"][0]["content"][0]["text"], "hello");
    assert_eq!(value["usage"]["input_tokens"], 10);
    assert_eq!(value["usage"]["output_tokens"], 5);
    assert_eq!(value["usage"]["total_tokens"], 15);
}
