#[path = "live_support/mod.rs"]
mod live_support;

use std::collections::BTreeMap;

use live_support::{assert_success, load_env, optional, post_json, required};
use llmwire::codec::{Chat, Messages, ProtocolCodec, Responses};
use llmwire::framing::SseFramer;
use llmwire::ir::{AssistantOutput, Part, StreamState, Termination};
use llmwire::{converter, resolve, ProtocolId};

const CROSS_PAIRS: [(ProtocolId, ProtocolId); 6] = [
    (ProtocolId::Chat, ProtocolId::Messages),
    (ProtocolId::Chat, ProtocolId::Responses),
    (ProtocolId::Messages, ProtocolId::Chat),
    (ProtocolId::Messages, ProtocolId::Responses),
    (ProtocolId::Responses, ProtocolId::Chat),
    (ProtocolId::Responses, ProtocolId::Messages),
];

#[test]
#[ignore = "requires .env live API settings"]
fn live_cross_protocol_nonstream_matrix() {
    let env = load_env().expect(".env is required for live API tests");

    for (src, dst) in CROSS_PAIRS {
        let case = format!("{src:?}->{dst:?}");
        let target = Target::new(dst, &env);
        let source_body = source_request(src, &target.model, false);
        let mut converter = converter(src, dst, resolve(src, dst, &target.model)).unwrap();
        let mut upstream_body = Vec::new();
        converter
            .request(&source_body, &mut upstream_body)
            .unwrap_or_else(|error| panic!("{case} request failed: {error}"));

        let output = post_json(&target.url, &target.headers, &upstream_body);
        assert_success(&output);

        let mut client_body = Vec::new();
        converter
            .response(&output.stdout, &mut client_body)
            .unwrap_or_else(|error| panic!("{case} response failed: {error}"));
        let decoded = codec(src)
            .decode_response(&client_body)
            .unwrap_or_else(|error| panic!("{case} decode failed: {error}"));
        let text = text_from_output(&decoded);
        assert!(
            !text.trim().is_empty(),
            "{case} produced no text; client={} parts={:?}",
            String::from_utf8_lossy(&client_body),
            decoded.choices
        );
    }
}

#[test]
#[ignore = "requires .env live API settings"]
fn live_cross_protocol_stream_matrix() {
    let env = load_env().expect(".env is required for live API tests");

    for (src, dst) in CROSS_PAIRS {
        let case = format!("{src:?}->{dst:?}");
        let target = Target::new(dst, &env);
        let source_body = source_request(src, &target.model, true);
        let mut converter = converter(src, dst, resolve(src, dst, &target.model)).unwrap();
        let mut upstream_body = Vec::new();
        converter
            .request(&source_body, &mut upstream_body)
            .unwrap_or_else(|error| panic!("{case} stream request failed: {error}"));

        let output = post_json(&target.url, &target.headers, &upstream_body);
        assert_success(&output);

        let mut client_stream = Vec::new();
        converter
            .feed(&output.stdout, &mut client_stream)
            .unwrap_or_else(|error| panic!("{case} feed failed: {error}"));
        let mut tail = Vec::new();
        let termination = converter
            .finish(&mut tail)
            .unwrap_or_else(|error| panic!("{case} finish failed: {error}"));
        client_stream.extend_from_slice(&tail);

        assert_eq!(termination, Termination::Explicit, "{case}");
        let target_stream_text = String::from_utf8_lossy(&output.stdout);
        let (state, termination) = decode_stream(src, &client_stream).unwrap_or_else(|error| {
            panic!(
                "{case} source stream decode failed: {error}\nTARGET:\n{target_stream_text}\nCLIENT:\n{}",
                String::from_utf8_lossy(&client_stream)
            )
        });
        assert_eq!(
            termination,
            Some(Termination::Explicit),
            "{case}\n{}",
            String::from_utf8_lossy(&client_stream)
        );
        let output = state
            .assistant_output()
            .unwrap_or_else(|error| panic!("{case} produced no assistant output: {error}"));
        let text = text_from_output(&output);
        assert!(
            !text.trim().is_empty(),
            "{case} produced no text; client={} parts={:?}",
            String::from_utf8_lossy(&client_stream),
            output.choices
        );
    }
}

struct Target {
    url: String,
    model: String,
    headers: Vec<(String, String)>,
}

impl Target {
    fn new(protocol: ProtocolId, env: &BTreeMap<String, String>) -> Self {
        match protocol {
            ProtocolId::Chat => {
                let key = required(env, "LLMWIRE_LIVE_CHAT_API_KEY");
                Self {
                    url: required(env, "LLMWIRE_LIVE_CHAT_URL"),
                    model: required(env, "LLMWIRE_LIVE_CHAT_MODEL"),
                    headers: vec![("Authorization".to_owned(), format!("Bearer {key}"))],
                }
            }
            ProtocolId::Messages => {
                let key = required(env, "LLMWIRE_LIVE_MESSAGES_API_KEY");
                let version = required(env, "LLMWIRE_LIVE_MESSAGES_VERSION");
                let mut headers = vec![
                    ("x-api-key".to_owned(), key),
                    ("anthropic-version".to_owned(), version),
                ];
                if let Some(beta) = optional(env, "LLMWIRE_LIVE_MESSAGES_BETA") {
                    headers.push(("anthropic-beta".to_owned(), beta));
                }
                Self {
                    url: required(env, "LLMWIRE_LIVE_MESSAGES_URL"),
                    model: required(env, "LLMWIRE_LIVE_MESSAGES_MODEL"),
                    headers,
                }
            }
            ProtocolId::Responses => {
                let key = required(env, "LLMWIRE_LIVE_RESPONSES_API_KEY");
                Self {
                    url: required(env, "LLMWIRE_LIVE_RESPONSES_URL"),
                    model: required(env, "LLMWIRE_LIVE_RESPONSES_MODEL"),
                    headers: vec![("Authorization".to_owned(), format!("Bearer {key}"))],
                }
            }
            _ => unreachable!(),
        }
    }
}

fn codec(protocol: ProtocolId) -> Box<dyn ProtocolCodec> {
    match protocol {
        ProtocolId::Chat => Box::new(Chat),
        ProtocolId::Messages => Box::new(Messages),
        ProtocolId::Responses => Box::new(Responses),
        _ => unreachable!(),
    }
}

fn source_request(protocol: ProtocolId, model: &str, stream: bool) -> Vec<u8> {
    match protocol {
        ProtocolId::Chat => serde_json::json!({
            "model": model,
            "stream": stream,
            "messages": [{"role": "user", "content": "Reply with exactly: llmwire-ok"}],
            "max_completion_tokens": 64,
            "temperature": 0.0
        }),
        ProtocolId::Messages => serde_json::json!({
            "model": model,
            "stream": stream,
            "messages": [{"role": "user", "content": "Reply with exactly: llmwire-ok"}],
            "max_tokens": 64,
            "temperature": 0.0
        }),
        ProtocolId::Responses => serde_json::json!({
            "model": model,
            "stream": stream,
            "input": "Reply with exactly: llmwire-ok",
            "max_output_tokens": 64,
            "temperature": 0.0
        }),
        _ => unreachable!(),
    }
    .to_string()
    .into_bytes()
}

fn decode_stream(
    protocol: ProtocolId,
    input: &[u8],
) -> Result<(StreamState, Option<Termination>), llmwire::Error> {
    let mut framer = SseFramer::new();
    let mut frames = Vec::new();
    framer.feed(input, &mut frames).unwrap();
    framer.finish(&mut frames).unwrap();

    let codec = codec(protocol);
    let mut state = StreamState::new();
    let mut termination = None;
    for frame in frames {
        let decoded = codec.decode_stream_frame(&frame, &state)?;
        state.apply_all(decoded.events)?;
        if decoded.termination.is_some() {
            termination = decoded.termination;
        }
    }
    Ok((state, termination))
}

fn text_from_output(output: &AssistantOutput) -> String {
    output
        .choices
        .iter()
        .flat_map(|choice| &choice.parts)
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}
