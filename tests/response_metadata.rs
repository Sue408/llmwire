use llmwire::framing::{SseFrame, SseFramer};
use llmwire::{converter, resolve, ProtocolId, Termination};
use serde_json::Value;

const SOURCE_MODEL: &str = "client-model";
const TARGET_MODEL: &str = "target-model";
const TARGET_ID: &str = "target-id";

fn protocols() -> [ProtocolId; 3] {
    [
        ProtocolId::Chat,
        ProtocolId::Messages,
        ProtocolId::Responses,
    ]
}

fn request_body(protocol: ProtocolId, stream: bool) -> Vec<u8> {
    let stream = if stream { r#","stream":true"# } else { "" };
    let body = match protocol {
        ProtocolId::Chat => format!(
            r#"{{"model":"{SOURCE_MODEL}","messages":[{{"role":"user","content":"hello"}}],"max_completion_tokens":32{stream}}}"#
        ),
        ProtocolId::Messages => format!(
            r#"{{"model":"{SOURCE_MODEL}","max_tokens":32,"messages":[{{"role":"user","content":"hello"}}]{stream}}}"#
        ),
        ProtocolId::Responses => format!(
            r#"{{"model":"{SOURCE_MODEL}","input":"hello","max_output_tokens":32{stream}}}"#
        ),
        _ => unreachable!(),
    };
    body.into_bytes()
}

fn target_response(protocol: ProtocolId, model: Option<&str>) -> Vec<u8> {
    let model = model
        .map(|model| format!(r#""model":"{model}","#))
        .unwrap_or_default();
    let body = match protocol {
        ProtocolId::Chat => format!(
            r#"{{"id":"{TARGET_ID}","object":"chat.completion",{model}"choices":[{{"index":0,"message":{{"role":"assistant","content":"ok"}},"finish_reason":"stop"}}]}}"#
        ),
        ProtocolId::Messages => format!(
            r#"{{"id":"{TARGET_ID}","type":"message","role":"assistant",{model}"content":[{{"type":"text","text":"ok"}}],"stop_reason":"end_turn"}}"#
        ),
        ProtocolId::Responses => format!(
            r#"{{"id":"{TARGET_ID}","object":"response","status":"completed",{model}"output":[{{"type":"message","id":"msg_1","role":"assistant","content":[{{"type":"output_text","text":"ok"}}]}}]}}"#
        ),
        _ => unreachable!(),
    };
    body.into_bytes()
}

fn target_stream(protocol: ProtocolId, model: Option<&str>) -> Vec<u8> {
    let model = model
        .map(|model| format!(r#","model":"{model}""#))
        .unwrap_or_default();
    let body = match protocol {
        ProtocolId::Chat => format!(
            concat!(
                "data: {{\"id\":\"{id}\",\"object\":\"chat.completion.chunk\"{model},\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\"}},\"finish_reason\":null}}]}}\n\n",
                "data: {{\"id\":\"{id}\",\"object\":\"chat.completion.chunk\"{model},\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"ok\"}},\"finish_reason\":null}}]}}\n\n",
                "data: {{\"id\":\"{id}\",\"object\":\"chat.completion.chunk\"{model},\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\n",
                "data: [DONE]\n\n"
            ),
            id = TARGET_ID,
            model = model,
        ),
        ProtocolId::Messages => format!(
            concat!(
                "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"{id}\"{model}}}}}\n\n",
                "event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n",
                "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"ok\"}}}}\n\n",
                "event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n",
                "event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}}}}\n\n",
                "event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
            ),
            id = TARGET_ID,
            model = model,
        ),
        ProtocolId::Responses => format!(
            concat!(
                "event: response.created\ndata: {{\"type\":\"response.created\",\"response\":{{\"id\":\"{id}\"{model}}}}}\n\n",
                "event: response.output_item.added\ndata: {{\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{{\"type\":\"message\",\"id\":\"msg_1\"}}}}\n\n",
                "event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"ok\"}}\n\n",
                "event: response.output_text.done\ndata: {{\"type\":\"response.output_text.done\",\"output_index\":0,\"content_index\":0,\"text\":\"ok\"}}\n\n",
                "event: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"{id}\",\"status\":\"completed\"{model},\"output\":[]}}}}\n\n"
            ),
            id = TARGET_ID,
            model = model,
        ),
        _ => unreachable!(),
    };
    body.into_bytes()
}

fn convert_nonstream(src: ProtocolId, dst: ProtocolId, model: Option<&str>) -> Vec<u8> {
    let mut converter = converter(src, dst, resolve(src, dst, SOURCE_MODEL)).unwrap();
    let mut request = Vec::new();
    converter
        .request(&request_body(src, false), &mut request)
        .unwrap();
    let mut response = Vec::new();
    converter
        .response(&target_response(dst, model), &mut response)
        .unwrap();
    response
}

fn convert_stream(src: ProtocolId, dst: ProtocolId, model: Option<&str>) -> Vec<u8> {
    let mut converter = converter(src, dst, resolve(src, dst, SOURCE_MODEL)).unwrap();
    let mut request = Vec::new();
    converter
        .request(&request_body(src, true), &mut request)
        .unwrap();
    let mut response = Vec::new();
    converter
        .feed(&target_stream(dst, model), &mut response)
        .unwrap();
    let mut tail = Vec::new();
    assert_eq!(converter.finish(&mut tail).unwrap(), Termination::Explicit);
    response.extend_from_slice(&tail);
    response
}

fn parse_json(body: &[u8]) -> Value {
    serde_json::from_slice(body).unwrap()
}

fn frames(body: &[u8]) -> Vec<SseFrame> {
    let mut framer = SseFramer::new();
    let mut frames = Vec::new();
    framer.feed(body, &mut frames).unwrap();
    framer.finish(&mut frames).unwrap();
    frames
}

fn assert_nonstream_metadata(protocol: ProtocolId, body: &[u8], model: &str) {
    let value = parse_json(body);
    assert_eq!(value["id"], TARGET_ID, "{protocol:?}");
    assert_eq!(value["model"], model, "{protocol:?}");
}

fn assert_stream_metadata(protocol: ProtocolId, body: &[u8], model: &str) {
    let mut seen = 0;
    for frame in frames(body) {
        let Some(data) = frame.data.as_deref() else {
            continue;
        };
        if data == "[DONE]" {
            continue;
        }
        let value = parse_json(data.as_bytes());
        match protocol {
            ProtocolId::Chat => {
                assert_eq!(value["id"], TARGET_ID);
                assert_eq!(value["model"], model);
                seen += 1;
            }
            ProtocolId::Messages => {
                if value["type"] == "message_start" {
                    assert_eq!(value["message"]["id"], TARGET_ID);
                    assert_eq!(value["message"]["model"], model);
                    seen += 1;
                }
            }
            ProtocolId::Responses => {
                if let Some(response) = value.get("response") {
                    assert_eq!(response["id"], TARGET_ID);
                    assert_eq!(response["model"], model);
                    seen += 1;
                }
            }
            _ => unreachable!(),
        }
    }
    assert!(seen > 0, "missing metadata event for {protocol:?}");
}

#[test]
fn nonstream_response_metadata_follows_target() {
    for src in protocols() {
        for dst in protocols() {
            if src == dst {
                continue;
            }
            let response = convert_nonstream(src, dst, Some(TARGET_MODEL));
            assert_nonstream_metadata(src, &response, TARGET_MODEL);
        }
    }
}

#[test]
fn stream_response_metadata_follows_target() {
    for src in protocols() {
        for dst in protocols() {
            if src == dst {
                continue;
            }
            let response = convert_stream(src, dst, Some(TARGET_MODEL));
            assert_stream_metadata(src, &response, TARGET_MODEL);
        }
    }
}

#[test]
fn missing_target_model_uses_empty_string() {
    for src in protocols() {
        for dst in protocols() {
            if src == dst {
                continue;
            }
            let response = convert_nonstream(src, dst, None);
            assert_nonstream_metadata(src, &response, "");

            let response = convert_stream(src, dst, None);
            assert_stream_metadata(src, &response, "");
        }
    }
}
