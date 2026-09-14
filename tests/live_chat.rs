#[path = "live_support/mod.rs"]
mod live_support;

use llmwire::codec::{Chat, ProtocolCodec};
use llmwire::ir::{Conversation, Part, Role, Sampling, Turn};
use llmwire::{converter, resolve, ProtocolId, Termination};

use live_support::{assert_success, load_env, post_json, required};

#[test]
#[ignore = "requires .env live API settings"]
fn live_chat_nonstream_roundtrip() {
    let env = load_env().expect(".env is required for live API tests");
    let url = required(&env, "LLMWIRE_LIVE_CHAT_URL");
    let key = required(&env, "LLMWIRE_LIVE_CHAT_API_KEY");
    let model = required(&env, "LLMWIRE_LIVE_CHAT_MODEL");
    let request_body = chat_request_body(&model, false);

    let output = post_json(
        &url,
        &[("Authorization".to_owned(), format!("Bearer {key}"))],
        &request_body,
    );
    assert_success(&output);

    let response = Chat.decode_response(&output.stdout).unwrap();
    assert!(!response.choices.is_empty());
    assert!(response
        .choices
        .iter()
        .flat_map(|choice| &choice.parts)
        .any(|part| matches!(part, Part::Text(_))));
}

#[test]
#[ignore = "requires .env live API settings"]
fn live_chat_stream_terminates_explicitly() {
    let env = load_env().expect(".env is required for live API tests");
    let url = required(&env, "LLMWIRE_LIVE_CHAT_URL");
    let key = required(&env, "LLMWIRE_LIVE_CHAT_API_KEY");
    let model = required(&env, "LLMWIRE_LIVE_CHAT_MODEL");
    let request_body = chat_request_body(&model, true);

    let mut converter = converter(
        ProtocolId::Chat,
        ProtocolId::Chat,
        resolve(ProtocolId::Chat, ProtocolId::Chat, &model),
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

    assert_eq!(termination, Termination::Explicit);
    let text = String::from_utf8(client_stream).unwrap();
    assert!(text.contains("data: [DONE]"));
}

fn chat_request_body(model: &str, stream: bool) -> Vec<u8> {
    let conversation = Conversation {
        system: vec![Part::Text("You are terse.".to_owned())],
        turns: vec![Turn {
            role: Role::User,
            parts: vec![Part::Text("Reply with exactly: llmwire-ok".to_owned())],
        }],
        sampling: Sampling {
            max_output_tokens: Some(32),
            temperature: Some(0.0),
            ..Sampling::default()
        },
        ..Conversation::default()
    };

    let mut request: serde_json::Value =
        serde_json::from_slice(&Chat.encode_request(&conversation).unwrap()).unwrap();
    request["model"] = serde_json::Value::String(model.to_owned());
    request["stream"] = serde_json::Value::Bool(stream);
    serde_json::to_vec(&request).unwrap()
}
