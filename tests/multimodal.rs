use llmwire::codec::{Chat, Messages, ProtocolCodec, Responses};
use llmwire::ir::{ImageSource, Part};
use llmwire::{converter, resolve, Mode, ProtocolId, Severity, UnmappedReason};
use serde_json::Value;

#[derive(Clone, Copy, Debug)]
enum SourceCase {
    RemoteUrl,
    Base64,
}

#[test]
fn image_roundtrip_covers_three_protocols_and_both_sources() {
    for protocol in [
        ProtocolId::Chat,
        ProtocolId::Messages,
        ProtocolId::Responses,
    ] {
        for source in [SourceCase::RemoteUrl, SourceCase::Base64] {
            let request = image_request(protocol, source);
            let codec = codec(protocol);
            let conversation = codec
                .decode_request(&request)
                .unwrap_or_else(|error| panic!("{protocol:?} {source:?} decode: {error}"));
            assert_image_source(&conversation, source, &format!("{protocol:?}"));

            let encoded = codec
                .encode_request(&conversation)
                .unwrap_or_else(|error| panic!("{protocol:?} {source:?} encode: {error}"));
            assert_wire_image(
                protocol,
                &encoded,
                source,
                &format!("{protocol:?}"),
                Some("high"),
            );

            let decoded = codec
                .decode_request(&encoded)
                .unwrap_or_else(|error| panic!("{protocol:?} {source:?} re-decode: {error}"));
            assert_image_source(&decoded, source, &format!("{protocol:?} re-decode"));
        }
    }
}

#[test]
fn openai_data_uri_and_messages_separate_fields_convert_both_ways() {
    for source in [SourceCase::RemoteUrl, SourceCase::Base64] {
        let chat_request = image_request(ProtocolId::Chat, source);
        let mut messages = Vec::new();
        converter(
            ProtocolId::Chat,
            ProtocolId::Messages,
            resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-test"),
        )
        .unwrap()
        .request(&chat_request, &mut messages)
        .unwrap();
        assert_wire_image(
            ProtocolId::Messages,
            &messages,
            source,
            "Chat->Messages",
            None,
        );

        let messages_request = image_request(ProtocolId::Messages, source);
        let mut chat = Vec::new();
        converter(
            ProtocolId::Messages,
            ProtocolId::Chat,
            resolve(ProtocolId::Messages, ProtocolId::Chat, "gpt-test"),
        )
        .unwrap()
        .request(&messages_request, &mut chat)
        .unwrap();
        assert_wire_image(ProtocolId::Chat, &chat, source, "Messages->Chat", None);
    }
}

#[test]
fn image_matrix_covers_all_protocol_directions_and_sources() {
    for src in [
        ProtocolId::Chat,
        ProtocolId::Messages,
        ProtocolId::Responses,
    ] {
        for dst in [
            ProtocolId::Chat,
            ProtocolId::Messages,
            ProtocolId::Responses,
        ] {
            for source in [SourceCase::RemoteUrl, SourceCase::Base64] {
                let case = format!("{src:?}->{dst:?} {source:?}");
                let mut converter = converter(src, dst, resolve(src, dst, "vision-test")).unwrap();
                let mut out = Vec::new();
                converter
                    .request(&image_request(src, source), &mut out)
                    .unwrap_or_else(|error| panic!("{case}: {error}"));
                let target = codec(dst)
                    .decode_request(&out)
                    .unwrap_or_else(|error| panic!("{case}: target decode {error}"));
                assert_image_source(&target, source, &case);
            }
        }
    }
}

#[test]
fn unsupported_image_inputs_fail_explicitly() {
    let chat_ftp = br#"{
        "messages":[{
            "role":"user",
            "content":[{
                "type":"image_url",
                "image_url":{"url":"ftp://example.test/image.png"}
            }]
        }]
    }"#;
    assert!(matches!(
        Chat.decode_request(chat_ftp),
        Err(llmwire::Error::Unsupported(_))
    ));

    let chat_file = br#"{
        "messages":[{
            "role":"user",
            "content":[{"type":"image_url","file_id":"file_1"}]
        }]
    }"#;
    assert!(matches!(
        Chat.decode_request(chat_file),
        Err(llmwire::Error::Unsupported(_))
    ));

    let responses_file = br#"{
        "input":[{
            "type":"message",
            "role":"user",
            "content":[{"type":"input_image","file_id":"file_1"}]
        }]
    }"#;
    assert!(matches!(
        Responses.decode_request(responses_file),
        Err(llmwire::Error::Unsupported(_))
    ));

    let messages_bad_base64 = br#"{
        "max_tokens":32,
        "messages":[{
            "role":"user",
            "content":[{
                "type":"image",
                "source":{"type":"base64","media_type":"image/png","data":"not-base64"}
            }]
        }]
    }"#;
    assert!(matches!(
        Messages.decode_request(messages_bad_base64),
        Err(llmwire::Error::InvalidInput(_))
    ));
}

#[test]
fn messages_target_reports_image_detail_and_strict_rejects() {
    let request = image_request(ProtocolId::Chat, SourceCase::RemoteUrl);
    let mut converted = converter(
        ProtocolId::Chat,
        ProtocolId::Messages,
        resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-test"),
    )
    .unwrap();
    let mut out = Vec::new();
    converted.request(&request, &mut out).unwrap();
    let report = converted.take_report();
    assert!(report.unmapped.iter().any(|entry| {
        entry.field.as_ref() == "request.messages[0].content[0].detail"
            && entry.reason == UnmappedReason::NotRepresentable
            && entry.severity == Severity::Degraded
    }));

    let mut caps = resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-test");
    caps.mode = Mode::Strict;
    let mut strict = converter(ProtocolId::Chat, ProtocolId::Messages, caps).unwrap();
    let mut out = Vec::new();
    assert!(matches!(
        strict.request(&request, &mut out),
        Err(llmwire::Error::Unsupported(_))
    ));
    assert!(strict.take_report().has_fatal());
}

#[test]
fn responses_preserves_detail_on_same_protocol_roundtrip() {
    let request = image_request(ProtocolId::Responses, SourceCase::RemoteUrl);
    let conversation = Responses.decode_request(&request).unwrap();
    let encoded = Responses.encode_request(&conversation).unwrap();
    let value: Value = serde_json::from_slice(&encoded).unwrap();

    assert_eq!(value["input"][0]["content"][0]["detail"], "high");
}

fn image_request(protocol: ProtocolId, source: SourceCase) -> Vec<u8> {
    let value = match protocol {
        ProtocolId::Chat => serde_json::json!({
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image_url",
                    "image_url": {"url": openai_url(source), "detail": "high"}
                }]
            }]
        }),
        ProtocolId::Messages => serde_json::json!({
            "max_tokens": 32,
            "messages": [{
                "role": "user",
                "content": [{"type": "image", "source": messages_source(source)}]
            }]
        }),
        ProtocolId::Responses => serde_json::json!({
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_image",
                    "image_url": openai_url(source),
                    "detail": "high"
                }]
            }]
        }),
        _ => unreachable!(),
    };
    value.to_string().into_bytes()
}

fn openai_url(source: SourceCase) -> &'static str {
    match source {
        SourceCase::RemoteUrl => "https://example.test/image.png?x=1",
        SourceCase::Base64 => "data:image/png;base64,AAAA",
    }
}

fn messages_source(source: SourceCase) -> Value {
    match source {
        SourceCase::RemoteUrl => serde_json::json!({
            "type": "url",
            "url": "https://example.test/image.png?x=1"
        }),
        SourceCase::Base64 => serde_json::json!({
            "type": "base64",
            "media_type": "image/png",
            "data": "AAAA"
        }),
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

fn assert_image_source(conversation: &llmwire::ir::Conversation, expected: SourceCase, case: &str) {
    let Part::Image(image) = &conversation.turns[0].parts[0] else {
        panic!("{case}: missing image part");
    };
    assert_source(&image.source, expected, case);
}

fn assert_source(source: &ImageSource, expected: SourceCase, case: &str) {
    match (source, expected) {
        (ImageSource::RemoteUrl(url), SourceCase::RemoteUrl) => {
            assert_eq!(url.as_ref(), "https://example.test/image.png?x=1", "{case}");
        }
        (ImageSource::Base64 { media_type, data }, SourceCase::Base64) => {
            assert_eq!(media_type.as_ref(), "image/png", "{case}");
            assert_eq!(data.as_ref(), "AAAA", "{case}");
        }
        _ => panic!("{case}: image source kind changed"),
    }
}

fn assert_wire_image(
    protocol: ProtocolId,
    body: &[u8],
    source: SourceCase,
    case: &str,
    expected_detail: Option<&str>,
) {
    let value: Value = serde_json::from_slice(body).unwrap();
    match protocol {
        ProtocolId::Chat => {
            assert_eq!(value["messages"][0]["content"][0]["type"], "image_url");
            assert_eq!(
                value["messages"][0]["content"][0]["image_url"]["url"],
                openai_url(source),
                "{case}"
            );
            assert_detail(
                &value["messages"][0]["content"][0]["image_url"]["detail"],
                expected_detail,
                case,
            );
        }
        ProtocolId::Messages => {
            assert_eq!(value["messages"][0]["content"][0]["type"], "image");
            match source {
                SourceCase::RemoteUrl => {
                    assert_eq!(
                        value["messages"][0]["content"][0]["source"]["type"], "url",
                        "{case}"
                    );
                    assert_eq!(
                        value["messages"][0]["content"][0]["source"]["url"],
                        "https://example.test/image.png?x=1",
                        "{case}"
                    );
                }
                SourceCase::Base64 => {
                    assert_eq!(
                        value["messages"][0]["content"][0]["source"]["type"], "base64",
                        "{case}"
                    );
                    assert_eq!(
                        value["messages"][0]["content"][0]["source"]["media_type"], "image/png",
                        "{case}"
                    );
                    assert_eq!(
                        value["messages"][0]["content"][0]["source"]["data"], "AAAA",
                        "{case}"
                    );
                }
            }
        }
        ProtocolId::Responses => {
            assert_eq!(value["input"][0]["content"][0]["type"], "input_image");
            assert_eq!(
                value["input"][0]["content"][0]["image_url"],
                openai_url(source),
                "{case}"
            );
            assert_detail(
                &value["input"][0]["content"][0]["detail"],
                expected_detail,
                case,
            );
        }
        _ => unreachable!(),
    }
}

fn assert_detail(value: &Value, expected: Option<&str>, case: &str) {
    match expected {
        Some(detail) => assert_eq!(value, detail, "{case}"),
        None => assert!(value.is_null(), "{case}"),
    }
}
