#[path = "live_support/mod.rs"]
mod live_support;

use live_support::{assert_success, load_env, optional, post_json, required};
use llmwire::codec::{Messages, ProtocolCodec};
use llmwire::ids::OpaqueKind;
use llmwire::ir::{Conversation, Opaque, Part, Reasoning, Role, Sampling, Turn};

#[test]
#[ignore = "requires .env live API settings"]
fn live_messages_nonstream_roundtrip() {
    let env = load_env().expect(".env is required for live API tests");
    let url = required(&env, "LLMWIRE_LIVE_MESSAGES_URL");
    let key = required(&env, "LLMWIRE_LIVE_MESSAGES_API_KEY");
    let model = required(&env, "LLMWIRE_LIVE_MESSAGES_MODEL");
    let version = required(&env, "LLMWIRE_LIVE_MESSAGES_VERSION");
    let request_body = messages_request_body(&model, smoke_conversation());

    let output = post_json(&url, &messages_headers(&key, &version, &env), &request_body);
    assert_success(&output);

    let response = Messages.decode_response(&output.stdout).unwrap();
    assert!(!response.choices.is_empty());
    assert!(response
        .choices
        .iter()
        .flat_map(|choice| &choice.parts)
        .any(|part| matches!(part, Part::Text(_) | Part::Thinking(_))));
}

#[test]
#[ignore = "requires .env live API settings"]
fn live_messages_cache_control_and_usage_roundtrip() {
    let env = load_env().expect(".env is required for live API tests");
    let url = required(&env, "LLMWIRE_LIVE_MESSAGES_URL");
    let key = required(&env, "LLMWIRE_LIVE_MESSAGES_API_KEY");
    let model = required(&env, "LLMWIRE_LIVE_MESSAGES_MODEL");
    let version = required(&env, "LLMWIRE_LIVE_MESSAGES_VERSION");

    let mut conversation = smoke_conversation();
    conversation.system = vec![Part::Opaque(Opaque {
        kind: OpaqueKind::ProviderSpecific("anthropic_system_block"),
        bytes: serde_json::to_vec(&serde_json::json!({
            "type": "text",
            "text": "You are terse.",
            "cache_control": {"type": "ephemeral"}
        }))
        .unwrap()
        .into_boxed_slice(),
    })];
    let request_body = messages_request_body(&model, conversation);

    let output = post_json(&url, &messages_headers(&key, &version, &env), &request_body);
    assert_success(&output);

    let response = Messages.decode_response(&output.stdout).unwrap();
    assert!(response.usage.input.is_some());
    assert!(response.usage.output.is_some());
}

#[test]
#[ignore = "requires .env live API settings and LLMWIRE_LIVE_ENABLE_THINKING=1"]
fn live_messages_thinking_signature_roundtrip() {
    let env = load_env().expect(".env is required for live API tests");
    assert_eq!(
        optional(&env, "LLMWIRE_LIVE_ENABLE_THINKING").as_deref(),
        Some("1"),
        "set LLMWIRE_LIVE_ENABLE_THINKING=1 before running the live thinking test"
    );
    let url = required(&env, "LLMWIRE_LIVE_MESSAGES_URL");
    let key = required(&env, "LLMWIRE_LIVE_MESSAGES_API_KEY");
    let model = required(&env, "LLMWIRE_LIVE_MESSAGES_MODEL");
    let version = required(&env, "LLMWIRE_LIVE_MESSAGES_VERSION");

    let mut conversation = smoke_conversation();
    conversation.sampling.max_output_tokens = Some(2048);
    conversation.reasoning = Reasoning {
        enabled: true,
        budget_tokens: Some(1024),
        ..Reasoning::default()
    };
    let request_body = messages_request_body(&model, conversation);

    let output = post_json(&url, &messages_headers(&key, &version, &env), &request_body);
    assert_success(&output);

    let response = Messages.decode_response(&output.stdout).unwrap();
    let signatures = response
        .choices
        .iter()
        .flat_map(|choice| &choice.parts)
        .filter_map(|part| match part {
            Part::Thinking(thinking) => thinking.signature.as_ref(),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!signatures.is_empty());
    assert!(signatures
        .iter()
        .all(|signature| !signature.bytes.is_empty()));
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

fn messages_request_body(model: &str, conversation: Conversation) -> Vec<u8> {
    let mut request: serde_json::Value =
        serde_json::from_slice(&Messages.encode_request(&conversation).unwrap()).unwrap();
    request["model"] = serde_json::Value::String(model.to_owned());
    request["stream"] = serde_json::Value::Bool(false);
    serde_json::to_vec(&request).unwrap()
}

fn messages_headers(
    key: &str,
    version: &str,
    env: &std::collections::BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let mut headers = vec![
        ("x-api-key".to_owned(), key.to_owned()),
        ("anthropic-version".to_owned(), version.to_owned()),
    ];
    if let Some(beta) = optional(env, "LLMWIRE_LIVE_MESSAGES_BETA") {
        headers.push(("anthropic-beta".to_owned(), beta));
    }
    headers
}
