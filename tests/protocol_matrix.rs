use llmwire::codec::{Chat, Messages, ProtocolCodec, Responses};
use llmwire::framing::SseFramer;
use llmwire::ir::{AssistantOutput, Part, StreamState, Termination, ToolUse};
use llmwire::{converter, resolve, ProtocolId};

const PROTOCOLS: [ProtocolId; 3] = [
    ProtocolId::Chat,
    ProtocolId::Messages,
    ProtocolId::Responses,
];

#[test]
fn converter_text_matrix_covers_request_response_and_stream() {
    for src in PROTOCOLS {
        for dst in PROTOCOLS {
            let case = format!("{src:?}->{dst:?}");

            let mut request_converter =
                converter(src, dst, resolve(src, dst, "matrix-model")).unwrap();
            let mut target_request = Vec::new();
            request_converter
                .request(&text_request(src, false), &mut target_request)
                .unwrap_or_else(|error| panic!("{case} request failed: {error}"));
            assert_text_request(dst, &target_request, &case);

            let mut response_converter =
                converter(src, dst, resolve(src, dst, "matrix-model")).unwrap();
            let mut ignored_request = Vec::new();
            response_converter
                .request(&text_request(src, false), &mut ignored_request)
                .unwrap();
            let mut client_response = Vec::new();
            response_converter
                .response(text_response(dst), &mut client_response)
                .unwrap_or_else(|error| panic!("{case} response failed: {error}"));
            let decoded_response = codec(src).decode_response(&client_response).unwrap();
            assert_eq!(text_from_output(&decoded_response), "world", "{case}");
            assert_eq!(decoded_response.usage.input, Some(3), "{case}");
            assert_eq!(decoded_response.usage.output, Some(2), "{case}");

            let mut stream_converter =
                converter(src, dst, resolve(src, dst, "matrix-model")).unwrap();
            let mut target_stream_request = Vec::new();
            stream_converter
                .request(&text_request(src, true), &mut target_stream_request)
                .unwrap_or_else(|error| panic!("{case} stream request failed: {error}"));
            assert_target_stream_flag(dst, &target_stream_request, &case);

            let mut client_stream = Vec::new();
            stream_converter
                .feed(text_stream(dst), &mut client_stream)
                .unwrap_or_else(|error| panic!("{case} feed failed: {error}"));
            let mut tail = Vec::new();
            let termination = stream_converter
                .finish(&mut tail)
                .unwrap_or_else(|error| panic!("{case} finish failed: {error}"));
            client_stream.extend_from_slice(&tail);

            assert_eq!(termination, Termination::Explicit, "{case}");
            let (state, termination) = decode_stream(src, &client_stream);
            assert_eq!(termination, Some(Termination::Explicit), "{case}");
            let output = state.assistant_output().unwrap();
            assert_eq!(text_from_output(&output), "world", "{case}");
        }
    }
}

#[test]
fn converter_tool_matrix_covers_request_and_response() {
    for src in PROTOCOLS {
        for dst in PROTOCOLS {
            let case = format!("{src:?}->{dst:?}");

            let mut request_converter =
                converter(src, dst, resolve(src, dst, "matrix-model")).unwrap();
            let mut target_request = Vec::new();
            request_converter
                .request(&tool_request(src), &mut target_request)
                .unwrap_or_else(|error| panic!("{case} tool request failed: {error}"));
            let target_conversation = codec(dst)
                .decode_request(&target_request)
                .unwrap_or_else(|error| panic!("{case} target request decode failed: {error}"));
            assert_eq!(target_conversation.tools.len(), 1, "{case}");
            assert_eq!(
                target_conversation.tools[0].name.as_ref(),
                "lookup",
                "{case}",
            );

            let mut response_converter =
                converter(src, dst, resolve(src, dst, "matrix-model")).unwrap();
            let mut ignored_request = Vec::new();
            response_converter
                .request(&text_request(src, false), &mut ignored_request)
                .unwrap();
            let mut client_response = Vec::new();
            response_converter
                .response(tool_response(dst), &mut client_response)
                .unwrap_or_else(|error| panic!("{case} tool response failed: {error}"));
            let decoded_response = codec(src).decode_response(&client_response).unwrap();
            let tool_use = find_tool_use(&decoded_response).unwrap_or_else(|| {
                panic!("{case} missing tool use in {:?}", decoded_response.choices)
            });
            assert_eq!(tool_use.id.0.as_ref(), "call_matrix", "{case}");
            assert_eq!(tool_use.name.as_ref(), "lookup", "{case}");
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

fn text_request(protocol: ProtocolId, stream: bool) -> Vec<u8> {
    match protocol {
        ProtocolId::Chat => serde_json::json!({
            "model": "matrix-model",
            "stream": stream,
            "messages": [{"role": "user", "content": "hello"}],
            "max_completion_tokens": 32
        }),
        ProtocolId::Messages => serde_json::json!({
            "model": "matrix-model",
            "stream": stream,
            "max_tokens": 32,
            "messages": [{"role": "user", "content": "hello"}]
        }),
        ProtocolId::Responses => serde_json::json!({
            "model": "matrix-model",
            "stream": stream,
            "input": "hello",
            "max_output_tokens": 32
        }),
        _ => unreachable!(),
    }
    .to_string()
    .into_bytes()
}

fn tool_request(protocol: ProtocolId) -> Vec<u8> {
    match protocol {
        ProtocolId::Chat => serde_json::json!({
            "model": "matrix-model",
            "messages": [{"role": "user", "content": "Use lookup"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "lookup",
                    "description": "Lookup a value",
                    "parameters": {"type": "object"}
                }
            }]
        }),
        ProtocolId::Messages => serde_json::json!({
            "model": "matrix-model",
            "max_tokens": 32,
            "messages": [{"role": "user", "content": "Use lookup"}],
            "tools": [{
                "name": "lookup",
                "description": "Lookup a value",
                "input_schema": {"type": "object"}
            }]
        }),
        ProtocolId::Responses => serde_json::json!({
            "model": "matrix-model",
            "input": "Use lookup",
            "tools": [{
                "type": "function",
                "name": "lookup",
                "description": "Lookup a value",
                "parameters": {"type": "object"}
            }]
        }),
        _ => unreachable!(),
    }
    .to_string()
    .into_bytes()
}

fn text_response(protocol: ProtocolId) -> &'static [u8] {
    match protocol {
        ProtocolId::Chat => br#"{
            "id":"chatcmpl-matrix",
            "choices":[{"index":0,"message":{"role":"assistant","content":"world"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}
        }"#,
        ProtocolId::Messages => br#"{
            "id":"msg_matrix",
            "type":"message",
            "role":"assistant",
            "content":[{"type":"text","text":"world"}],
            "model":"matrix-model",
            "stop_reason":"end_turn",
            "usage":{"input_tokens":3,"output_tokens":2}
        }"#,
        ProtocolId::Responses => br#"{
            "id":"resp_matrix",
            "object":"response",
            "status":"completed",
            "model":"matrix-model",
            "output":[{"type":"message","id":"msg_matrix","role":"assistant","content":[{"type":"output_text","text":"world"}]}],
            "usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}
        }"#,
        _ => unreachable!(),
    }
}

fn tool_response(protocol: ProtocolId) -> &'static [u8] {
    match protocol {
        ProtocolId::Chat => br#"{
            "id":"chatcmpl-tool",
            "choices":[{
                "index":0,
                "message":{
                    "role":"assistant",
                    "content":null,
                    "tool_calls":[{
                        "id":"call_matrix",
                        "type":"function",
                        "function":{"name":"lookup","arguments":"{\"x\":1}"}
                    }]
                },
                "finish_reason":"tool_calls"
            }],
            "usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}
        }"#,
        ProtocolId::Messages => br#"{
            "id":"msg_tool",
            "type":"message",
            "role":"assistant",
            "content":[{"type":"tool_use","id":"call_matrix","name":"lookup","input":{"x":1}}],
            "model":"matrix-model",
            "stop_reason":"tool_use",
            "usage":{"input_tokens":3,"output_tokens":2}
        }"#,
        ProtocolId::Responses => br#"{
            "id":"resp_tool",
            "object":"response",
            "status":"completed",
            "model":"matrix-model",
            "output":[{"type":"function_call","call_id":"call_matrix","name":"lookup","arguments":"{\"x\":1}"}],
            "usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}
        }"#,
        _ => unreachable!(),
    }
}

fn text_stream(protocol: ProtocolId) -> &'static [u8] {
    match protocol {
        ProtocolId::Chat => concat!(
            "data: {\"id\":\"chatcmpl-stream\",\"model\":\"matrix-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"world\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-stream\",\"model\":\"matrix-model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        )
        .as_bytes(),
        ProtocolId::Messages => concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_stream\",\"model\":\"matrix-model\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"world\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        )
        .as_bytes(),
        ProtocolId::Responses => concat!(
            "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_stream\",\"model\":\"matrix-model\"}}\n\n",
            "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"message\",\"id\":\"msg_stream\"}}\n\n",
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"world\"}\n\n",
            "event: response.output_text.done\ndata: {\"type\":\"response.output_text.done\",\"output_index\":0,\"content_index\":0,\"text\":\"world\"}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_stream\",\"status\":\"completed\",\"model\":\"matrix-model\",\"output\":[]}}\n\n",
        )
        .as_bytes(),
        _ => unreachable!(),
    }
}

fn assert_text_request(protocol: ProtocolId, body: &[u8], case: &str) {
    let value: serde_json::Value = serde_json::from_slice(body).unwrap();
    match protocol {
        ProtocolId::Chat => {
            assert_eq!(value["messages"][0]["content"], "hello", "{case}");
            assert_eq!(value["max_completion_tokens"], 32, "{case}");
        }
        ProtocolId::Messages => {
            assert_eq!(
                value["messages"][0]["content"][0]["text"],
                "hello",
                "{case}: {}",
                String::from_utf8_lossy(body)
            );
            assert_eq!(value["max_tokens"], 32, "{case}");
        }
        ProtocolId::Responses => {
            assert_eq!(value["input"][0]["content"][0]["text"], "hello", "{case}");
            assert_eq!(value["max_output_tokens"], 32, "{case}");
        }
        _ => unreachable!(),
    }
}

fn assert_target_stream_flag(protocol: ProtocolId, body: &[u8], case: &str) {
    let value: serde_json::Value = serde_json::from_slice(body).unwrap();
    assert_eq!(value["stream"], true, "{case}");
    assert_textish_target_request(protocol, &value, case);
}

fn assert_textish_target_request(protocol: ProtocolId, value: &serde_json::Value, case: &str) {
    match protocol {
        ProtocolId::Chat => assert_eq!(value["messages"][0]["content"], "hello", "{case}"),
        ProtocolId::Messages => {
            assert_eq!(
                value["messages"][0]["content"][0]["text"], "hello",
                "{case}"
            )
        }
        ProtocolId::Responses => {
            assert_eq!(value["input"][0]["content"][0]["text"], "hello", "{case}")
        }
        _ => unreachable!(),
    }
}

fn decode_stream(protocol: ProtocolId, input: &[u8]) -> (StreamState, Option<Termination>) {
    let mut framer = SseFramer::new();
    let mut frames = Vec::new();
    framer.feed(input, &mut frames).unwrap();
    framer.finish(&mut frames).unwrap();

    let codec = codec(protocol);
    let mut state = StreamState::new();
    let mut termination = None;
    for frame in frames {
        let decoded = codec.decode_stream_frame(&frame, &state).unwrap();
        state.apply_all(decoded.events).unwrap();
        if decoded.termination.is_some() {
            termination = decoded.termination;
        }
    }
    (state, termination)
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

fn find_tool_use(output: &AssistantOutput) -> Option<&ToolUse> {
    output
        .choices
        .iter()
        .flat_map(|choice| &choice.parts)
        .find_map(|part| match part {
            Part::ToolUse(tool_use) => Some(tool_use),
            _ => None,
        })
}
