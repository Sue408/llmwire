use llmwire::codec::{ProtocolCodec, Responses};
use llmwire::{converter, resolve, Error, ProtocolId};
use serde_json::Value;

fn stateless_error(body: &[u8]) -> Error {
    Responses.decode_request(body).unwrap_err()
}

#[test]
fn responses_unsupported_store_true() {
    let error = stateless_error(
        br#"{
            "model":"gpt-test",
            "store":true,
            "input":"hello"
        }"#,
    );
    assert!(matches!(error, Error::Unsupported(message) if message.contains("store=true")));
}

#[test]
fn responses_unsupported_previous_response_id() {
    let error = stateless_error(
        br#"{
            "model":"gpt-test",
            "previous_response_id":"resp_previous",
            "input":"hello"
        }"#,
    );
    assert!(
        matches!(error, Error::Unsupported(message) if message.contains("previous_response_id"))
    );
}

#[test]
fn responses_unsupported_store_false_allowed() {
    let conversation = Responses
        .decode_request(
            br#"{
                "model":"gpt-test",
                "store":false,
                "input":"hello"
            }"#,
        )
        .unwrap();
    let encoded = Responses.encode_request(&conversation).unwrap();
    let value: Value = serde_json::from_slice(&encoded).unwrap();

    assert!(value.get("store").is_none());
    assert!(value.get("previous_response_id").is_none());
    assert_eq!(value["input"][0]["content"][0]["text"], "hello");
}

#[test]
fn responses_unsupported_converter_rejects_stateful_request() {
    let mut converter = converter(
        ProtocolId::Responses,
        ProtocolId::Chat,
        resolve(ProtocolId::Responses, ProtocolId::Chat, "gpt-test"),
    )
    .unwrap();
    let mut out = Vec::new();
    let error = converter
        .request(
            br#"{
                "model":"gpt-test",
                "store":true,
                "input":"hello"
            }"#,
            &mut out,
        )
        .unwrap_err();

    assert!(matches!(error, Error::Unsupported(message) if message.contains("store=true")));
    assert!(out.is_empty());
}
