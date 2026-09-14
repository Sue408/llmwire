use llmwire::codec::{Chat, Messages, ProtocolCodec, Responses};
use llmwire::framing::SseFramer;
use llmwire::{converter, resolve, ProtocolId};

#[test]
fn chat_missing_usage_remains_unknown() {
    let output = Chat
        .decode_response(br#"{"choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#)
        .unwrap();
    assert_eq!(output.usage.input, None);
    assert_eq!(output.usage.output, None);
}

#[test]
fn chat_include_usage_chunk_merges_values() {
    let input = concat!(
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"model-a\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"model-a\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"model-a\",\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":4}}\n\n",
        "data: [DONE]\n\n",
    );
    let mut framer = SseFramer::new();
    let mut frames = Vec::new();
    framer.feed(input.as_bytes(), &mut frames).unwrap();
    framer.finish(&mut frames).unwrap();

    let mut state = llmwire::ir::StreamState::new();
    for frame in frames {
        let decoded = Chat.decode_stream_frame(&frame, &state).unwrap();
        state.apply_all(decoded.events).unwrap();
    }
    assert_eq!(state.usage().input, Some(10));
    assert_eq!(state.usage().output, Some(4));
}

#[test]
fn messages_usage_cache_fields_are_preserved() {
    let output = Messages
        .decode_response(
            br#"{
                "id":"msg_1",
                "type":"message",
                "role":"assistant",
                "content":[],
                "model":"claude-test",
                "stop_reason":"end_turn",
                "usage":{
                    "input_tokens":10,
                    "output_tokens":4,
                    "cache_read_input_tokens":3,
                    "cache_creation_input_tokens":2
                }
            }"#,
        )
        .unwrap();
    assert_eq!(output.usage.input, Some(15));
    assert_eq!(output.usage.output, Some(4));
    assert_eq!(output.usage.cached, 3);
    assert_eq!(output.usage.cache_creation, 2);
}

#[test]
fn messages_estimated_input_usage_is_reported() {
    let mut converter = converter(
        ProtocolId::Chat,
        ProtocolId::Messages,
        resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-test"),
    )
    .unwrap();
    let request = br#"{
        "model":"model-a",
        "stream":true,
        "messages":[{"role":"user","content":"hello"}]
    }"#;
    let mut request_out = Vec::new();
    converter.request(request, &mut request_out).unwrap();

    let stream = concat!(
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"model-a\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"model-a\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let mut stream_out = Vec::new();
    converter.feed(stream.as_bytes(), &mut stream_out).unwrap();
    let text = String::from_utf8(stream_out).unwrap();
    assert!(text.contains("\"estimated\":true"));

    let report = converter.take_report();
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.field.as_ref() == "stream.message_start.usage.input"));
}

#[test]
fn responses_usage_details_are_preserved() {
    let output = Responses
        .decode_response(
            br#"{
                "id":"resp_1",
                "status":"completed",
                "model":"gpt-test",
                "output":[],
                "usage":{
                    "input_tokens":10,
                    "output_tokens":4,
                    "input_tokens_details":{"cached_tokens":3},
                    "output_tokens_details":{"reasoning_tokens":2}
                }
            }"#,
        )
        .unwrap();
    assert_eq!(output.usage.input, Some(10));
    assert_eq!(output.usage.output, Some(4));
    assert_eq!(output.usage.cached, 3);
    assert_eq!(output.usage.reasoning, 2);
}
