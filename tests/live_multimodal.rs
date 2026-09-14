#[path = "live_support/mod.rs"]
mod live_support;

use live_support::{assert_success, load_env, optional, post_json, required};
use llmwire::codec::{Chat, Messages, ProtocolCodec, Responses};
use llmwire::ir::{Conversation, ImageRef, ImageSource, Part, Role, Sampling, Turn};
use llmwire::ProtocolId;

const VISION_PROMPT: &str = "Reply with exactly: llmwire-vision-ok";
const BASE64_IMAGE: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9WlW4xQAAAAASUVORK5CYII=";
const DEFAULT_IMAGE_URL: &str =
    "https://raw.githubusercontent.com/github/explore/main/topics/rust/rust.png";

#[test]
#[ignore = "requires .env live vision settings"]
fn live_multimodal_chat_vision_url_and_base64() {
    let env = load_env().expect(".env is required for live API tests");
    run_protocol_cases(ProtocolId::Chat, &env);
}

#[test]
#[ignore = "requires .env live vision settings"]
fn live_multimodal_messages_vision_url_and_base64() {
    let env = load_env().expect(".env is required for live API tests");
    run_protocol_cases(ProtocolId::Messages, &env);
}

#[test]
#[ignore = "requires .env live vision settings"]
fn live_multimodal_responses_vision_url_and_base64() {
    let env = load_env().expect(".env is required for live API tests");
    run_protocol_cases(ProtocolId::Responses, &env);
}

fn run_protocol_cases(protocol: ProtocolId, env: &std::collections::BTreeMap<String, String>) {
    assert_eq!(
        optional(env, "LLMWIRE_LIVE_VISION_ENABLE").as_deref(),
        Some("1"),
        "set LLMWIRE_LIVE_VISION_ENABLE=1 before running live vision tests; otherwise this is not a verified pass"
    );

    let model = required(env, vision_model_key(protocol));
    let image_url = optional(env, "LLMWIRE_LIVE_VISION_IMAGE_URL")
        .unwrap_or_else(|| DEFAULT_IMAGE_URL.to_owned());

    for source in [
        ImageSource::RemoteUrl(image_url.into_boxed_str()),
        ImageSource::Base64 {
            media_type: "image/png".into(),
            data: BASE64_IMAGE.into(),
        },
    ] {
        run_case(protocol, &model, source, env);
    }
}

fn run_case(
    protocol: ProtocolId,
    model: &str,
    source: ImageSource,
    env: &std::collections::BTreeMap<String, String>,
) {
    let request_body = vision_request_body(protocol, model, source);
    let url = required(env, protocol_url_key(protocol));
    let key = required(env, protocol_key_name(protocol));
    let headers = request_headers(protocol, &key, env);
    let output = post_json(&url, &headers, &request_body);
    assert_success(&output);

    let response = match protocol {
        ProtocolId::Chat => Chat.decode_response(&output.stdout),
        ProtocolId::Messages => Messages.decode_response(&output.stdout),
        ProtocolId::Responses => Responses.decode_response(&output.stdout),
        _ => unreachable!(),
    }
    .unwrap_or_else(|error| panic!("{protocol:?} vision response decode failed: {error}"));

    assert!(response
        .choices
        .iter()
        .flat_map(|choice| &choice.parts)
        .any(|part| matches!(part, Part::Text(_))));
}

fn vision_request_body(protocol: ProtocolId, model: &str, source: ImageSource) -> Vec<u8> {
    let conversation = Conversation {
        system: vec![Part::Text("You are terse.".to_owned())],
        turns: vec![Turn {
            role: Role::User,
            parts: vec![
                Part::Text(VISION_PROMPT.to_owned()),
                Part::Image(ImageRef {
                    source,
                    detail: match protocol {
                        ProtocolId::Messages => None,
                        _ => Some("low".into()),
                    },
                }),
            ],
        }],
        sampling: Sampling {
            max_output_tokens: Some(64),
            temperature: Some(0.0),
            ..Sampling::default()
        },
        ..Conversation::default()
    };

    let mut request: serde_json::Value = match protocol {
        ProtocolId::Chat => serde_json::from_slice(&Chat.encode_request(&conversation).unwrap()),
        ProtocolId::Messages => {
            serde_json::from_slice(&Messages.encode_request(&conversation).unwrap())
        }
        ProtocolId::Responses => {
            serde_json::from_slice(&Responses.encode_request(&conversation).unwrap())
        }
        _ => unreachable!(),
    }
    .unwrap();
    request["model"] = serde_json::Value::String(model.to_owned());
    request["stream"] = serde_json::Value::Bool(false);
    serde_json::to_vec(&request).unwrap()
}

fn request_headers(
    protocol: ProtocolId,
    key: &str,
    env: &std::collections::BTreeMap<String, String>,
) -> Vec<(String, String)> {
    match protocol {
        ProtocolId::Messages => {
            let mut headers = vec![
                ("x-api-key".to_owned(), key.to_owned()),
                (
                    "anthropic-version".to_owned(),
                    required(env, "LLMWIRE_LIVE_MESSAGES_VERSION"),
                ),
            ];
            if let Some(beta) = optional(env, "LLMWIRE_LIVE_MESSAGES_BETA") {
                headers.push(("anthropic-beta".to_owned(), beta));
            }
            headers
        }
        _ => vec![("Authorization".to_owned(), format!("Bearer {key}"))],
    }
}

fn protocol_url_key(protocol: ProtocolId) -> &'static str {
    match protocol {
        ProtocolId::Chat => "LLMWIRE_LIVE_CHAT_URL",
        ProtocolId::Messages => "LLMWIRE_LIVE_MESSAGES_URL",
        ProtocolId::Responses => "LLMWIRE_LIVE_RESPONSES_URL",
        _ => unreachable!(),
    }
}

fn protocol_key_name(protocol: ProtocolId) -> &'static str {
    match protocol {
        ProtocolId::Chat => "LLMWIRE_LIVE_CHAT_API_KEY",
        ProtocolId::Messages => "LLMWIRE_LIVE_MESSAGES_API_KEY",
        ProtocolId::Responses => "LLMWIRE_LIVE_RESPONSES_API_KEY",
        _ => unreachable!(),
    }
}

fn vision_model_key(protocol: ProtocolId) -> &'static str {
    match protocol {
        ProtocolId::Chat => "LLMWIRE_LIVE_VISION_CHAT_MODEL",
        ProtocolId::Messages => "LLMWIRE_LIVE_VISION_MESSAGES_MODEL",
        ProtocolId::Responses => "LLMWIRE_LIVE_VISION_RESPONSES_MODEL",
        _ => unreachable!(),
    }
}
