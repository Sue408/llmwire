use llmwire::codec::{Chat, Messages, ProtocolCodec, Responses};
use llmwire::framing::{SseFrame, SseFramer};
use llmwire::ir::{Part, StreamState, Termination, ToolUseKind};
use llmwire::Error;

fn frames(input: &[u8]) -> Vec<SseFrame> {
    let mut framer = SseFramer::new();
    let mut frames = Vec::new();
    framer.feed(input, &mut frames).unwrap();
    framer.finish(&mut frames).unwrap();
    frames
}

fn decode_stream(
    codec: &dyn ProtocolCodec,
    input: &[u8],
) -> Result<(StreamState, Option<Termination>), Error> {
    let mut state = StreamState::new();
    let mut termination = None;
    for frame in frames(input) {
        let decoded = codec.decode_stream_frame(&frame, &state)?;
        state.apply_all(decoded.events)?;
        if decoded.termination.is_some() {
            termination = decoded.termination;
        }
    }
    Ok((state, termination))
}

#[test]
fn chat_tolerates_comments_missing_role_and_argument_escape_split() {
    let first = serde_json::json!({
        "id":"chatcmpl-1",
        "model":"model-a",
        "choices":[{
            "index":0,
            "delta":{"tool_calls":[{
                "index":0,
                "id":"call_a",
                "type":"function",
                "function":{"name":"alpha","arguments":"{\"a\":\"x\\"}
            }]},
            "finish_reason":null
        }]
    });
    let second = serde_json::json!({
        "id":"chatcmpl-1",
        "model":"model-a",
        "choices":[{
            "index":0,
            "delta":{
                "role":"assistant",
                "tool_calls":[{"index":0,"function":{"arguments":"\"y\"}"}}]
            },
            "finish_reason":null
        }]
    });
    let input = [
        ": keep-alive\n\n".to_owned(),
        format!("data: {first}\n\n"),
        format!("data: {second}\n\n"),
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"model-a\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n".to_owned(),
        "data: [DONE]\n\n".to_owned(),
    ]
    .concat();

    let (state, termination) = decode_stream(&Chat, input.as_bytes()).unwrap();
    assert_eq!(termination, Some(Termination::Explicit));
    let Part::ToolUse(tool_use) = &state.assistant_output().unwrap().choices[0].parts[0] else {
        panic!("expected tool use");
    };
    assert_eq!(tool_use.id.0.as_ref(), "call_a");
    assert_eq!(tool_use.arguments.raw(), r#"{"a":"x\"y"}"#);
}

#[test]
fn chat_n_greater_than_one_stream_is_explicitly_unsupported() {
    let frame = frames(
        b"data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"index\":1,\"delta\":{\"content\":\"x\"},\"finish_reason\":null}]}\n\n",
    )
    .remove(0);
    let error = Chat
        .decode_stream_frame(&frame, &StreamState::new())
        .unwrap_err();
    assert!(matches!(error, Error::Unsupported(_)));
}

#[test]
fn messages_parallel_tool_use_and_input_json_split() {
    let input = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-test\"}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_b\",\"name\":\"beta\",\"input\":{}}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_a\",\"name\":\"alpha\",\"input\":{}}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"a\\\":\\\"x\\\\\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"y\\\"}\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"b\\\":2}\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );
    let (state, termination) = decode_stream(&Messages, input.as_bytes()).unwrap();
    assert_eq!(termination, Some(Termination::Explicit));
    let parts = &state.assistant_output().unwrap().choices[0].parts;
    let Part::ToolUse(first) = &parts[0] else {
        panic!("expected first tool");
    };
    let Part::ToolUse(second) = &parts[1] else {
        panic!("expected second tool");
    };
    assert_eq!(first.id.0.as_ref(), "toolu_a");
    assert_eq!(first.arguments.raw(), r#"{"a":"x\"y"}"#);
    assert_eq!(first.kind, ToolUseKind::Client);
    assert_eq!(second.id.0.as_ref(), "toolu_b");
    assert_eq!(second.arguments.raw(), r#"{"b":2}"#);
}

#[test]
fn messages_ping_is_ignored() {
    let input = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-test\"}}\n\n",
        "event: ping\ndata: {\"type\":\"ping\"}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );
    let (state, termination) = decode_stream(&Messages, input.as_bytes()).unwrap();
    assert_eq!(termination, Some(Termination::Explicit));
    assert!(state.is_finished());
}

#[test]
fn responses_output_index_and_content_index_locate_parts() {
    let input = concat!(
        "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\",\"model\":\"gpt-test\"}}\n\n",
        "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"type\":\"function_call\",\"call_id\":\"call_b\",\"name\":\"beta\",\"arguments\":\"\"}}\n\n",
        "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"call_id\":\"call_a\",\"name\":\"alpha\",\"arguments\":\"\"}}\n\n",
        "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":0,\"delta\":\"{\\\"a\\\":1}\"}\n\n",
        "event: response.function_call_arguments.done\ndata: {\"type\":\"response.function_call_arguments.done\",\"output_index\":0,\"arguments\":\"{\\\"a\\\":1}\"}\n\n",
        "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":1,\"delta\":\"{\\\"b\\\":2}\"}\n\n",
        "event: response.function_call_arguments.done\ndata: {\"type\":\"response.function_call_arguments.done\",\"output_index\":1,\"arguments\":\"{\\\"b\\\":2}\"}\n\n",
        "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"model\":\"gpt-test\",\"output\":[]}}\n\n",
    );
    let (state, termination) = decode_stream(&Responses, input.as_bytes()).unwrap();
    assert_eq!(termination, Some(Termination::Explicit));
    let parts = &state.assistant_output().unwrap().choices[0].parts;
    assert!(matches!(&parts[0], Part::ToolUse(tool) if tool.id.0.as_ref() == "call_a"));
    assert!(matches!(&parts[1], Part::ToolUse(tool) if tool.id.0.as_ref() == "call_b"));
}

#[test]
fn responses_rejects_done_marker() {
    let frame = SseFrame {
        event: None,
        data: Some("[DONE]".to_owned()),
    };
    let error = Responses
        .decode_stream_frame(&frame, &StreamState::new())
        .unwrap_err();
    assert!(matches!(error, Error::Protocol(_)));
}
