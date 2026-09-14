#[path = "live_support/mod.rs"]
mod live_support;

use live_support::{assert_success, load_env, post_json, required};
use llmwire::codec::{ProtocolCodec, Responses};
use llmwire::ir::{Conversation, Part, RawJson, Role, Sampling, ToolChoice, ToolDef, Turn};
use llmwire::{converter, resolve, ProtocolId, Termination};

#[test]
#[ignore = "requires .env live API settings"]
fn live_responses_nonstream_roundtrip() {
    let env = load_env().expect(".env is required for live API tests");
    let url = required(&env, "LLMWIRE_LIVE_RESPONSES_URL");
    let key = required(&env, "LLMWIRE_LIVE_RESPONSES_API_KEY");
    let model = required(&env, "LLMWIRE_LIVE_RESPONSES_MODEL");
    let request_body = responses_request_body(&model, smoke_conversation(), false);

    let output = post_json(
        &url,
        &[("Authorization".to_owned(), format!("Bearer {key}"))],
        &request_body,
    );
    assert_success(&output);

    let response = Responses.decode_response(&output.stdout).unwrap();
    assert!(!response.choices.is_empty());
    assert!(response
        .choices
        .iter()
        .flat_map(|choice| &choice.parts)
        .any(|part| matches!(part, Part::Text(_) | Part::Thinking(_))));
}

#[test]
#[ignore = "requires .env live API settings"]
fn live_responses_function_call_id_roundtrip() {
    let env = load_env().expect(".env is required for live API tests");
    let url = required(&env, "LLMWIRE_LIVE_RESPONSES_URL");
    let key = required(&env, "LLMWIRE_LIVE_RESPONSES_API_KEY");
    let model = required(&env, "LLMWIRE_LIVE_RESPONSES_MODEL");
    let request_body = responses_request_body(&model, tool_conversation(), false);

    let output = post_json(
        &url,
        &[("Authorization".to_owned(), format!("Bearer {key}"))],
        &request_body,
    );
    assert_success(&output);

    let response = Responses.decode_response(&output.stdout).unwrap();
    let tool_use = response
        .choices
        .iter()
        .flat_map(|choice| &choice.parts)
        .find_map(|part| match part {
            Part::ToolUse(tool_use) => Some(tool_use),
            _ => None,
        })
        .expect("expected function_call");

    let encoded = Responses.encode_response(&response).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    let output = value["output"].as_array().unwrap();
    let index = output
        .iter()
        .position(|item| item["type"] == "function_call")
        .expect("encoded output missing function_call");
    assert_eq!(output[index]["call_id"], tool_use.id.0.as_ref());
    assert_eq!(output[index]["arguments"], tool_use.arguments.raw());
}

#[test]
#[ignore = "requires .env live API settings"]
fn live_responses_stream_terminates_with_completed() {
    let env = load_env().expect(".env is required for live API tests");
    let url = required(&env, "LLMWIRE_LIVE_RESPONSES_URL");
    let key = required(&env, "LLMWIRE_LIVE_RESPONSES_API_KEY");
    let model = required(&env, "LLMWIRE_LIVE_RESPONSES_MODEL");
    let mut conversation = smoke_conversation();
    conversation.sampling.max_output_tokens = Some(1024);
    let request_body = responses_request_body(&model, conversation, true);

    let mut converter = converter(
        ProtocolId::Responses,
        ProtocolId::Responses,
        resolve(ProtocolId::Responses, ProtocolId::Responses, &model),
    )
    .unwrap();
    let mut upstream_body = Vec::new();
    converter
        .request(&request_body, &mut upstream_body)
        .unwrap();

    let output = post_json(
        &url,
        &[("Authorization".to_owned(), format!("Bearer {key}"))],
        &upstream_body,
    );
    assert_success(&output);

    let mut client_stream = Vec::new();
    converter.feed(&output.stdout, &mut client_stream).unwrap();
    let mut tail = Vec::new();
    let termination = converter.finish(&mut tail).unwrap();

    let text = String::from_utf8(client_stream).unwrap();
    assert_eq!(termination, Termination::Explicit, "{text}");
    assert!(text.contains("event: response.completed"));
    assert!(!text.contains("[DONE]"));
}

fn smoke_conversation() -> Conversation {
    Conversation {
        system: vec![Part::Text("You are terse.".to_owned())],
        turns: vec![Turn {
            role: Role::User,
            parts: vec![Part::Text("Reply with exactly: llmwire-ok".to_owned())],
        }],
        sampling: Sampling {
            max_output_tokens: Some(64),
            temperature: Some(0.0),
            ..Sampling::default()
        },
        ..Conversation::default()
    }
}

fn tool_conversation() -> Conversation {
    Conversation {
        system: vec![Part::Text("Use the ping tool when asked.".to_owned())],
        turns: vec![Turn {
            role: Role::User,
            parts: vec![Part::Text("Call ping with value ok.".to_owned())],
        }],
        tools: vec![ToolDef {
            name: "ping".into(),
            description: Some("Return the provided value.".into()),
            parameters: RawJson::from_raw(
                r#"{"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false}"#,
            ),
            strict: Some(true),
        }],
        tool_choice: ToolChoice::Auto,
        sampling: Sampling {
            max_output_tokens: Some(512),
            temperature: Some(0.0),
            ..Sampling::default()
        },
        ..Conversation::default()
    }
}

fn responses_request_body(model: &str, conversation: Conversation, stream: bool) -> Vec<u8> {
    let mut request: serde_json::Value =
        serde_json::from_slice(&Responses.encode_request(&conversation).unwrap()).unwrap();
    request["model"] = serde_json::Value::String(model.to_owned());
    request["stream"] = serde_json::Value::Bool(stream);
    serde_json::to_vec(&request).unwrap()
}
