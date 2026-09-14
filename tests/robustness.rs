use llmwire::codec::{Chat, Messages, ProtocolCodec, Responses};
use llmwire::framing::{encode_frame, SseFrame, SseFramer};
use llmwire::ir::{AssistantOutput, Conversation, Part, ToolUse};
use llmwire::{converter, resolve, Error, ProtocolId, Termination};
use proptest::prelude::*;
use std::panic::{catch_unwind, AssertUnwindSafe};

const PROTOCOLS: [ProtocolId; 3] = [
    ProtocolId::Chat,
    ProtocolId::Messages,
    ProtocolId::Responses,
];

fn arb_frame() -> impl Strategy<Value = SseFrame> {
    let event = proptest::option::of("[A-Za-z0-9_-]{1,16}");
    let line = "[A-Za-z0-9 ,.;:{}@#!?/_-]{0,24}";
    let data =
        proptest::option::of(prop::collection::vec(line, 1..4).prop_map(|lines| lines.join("\n")));

    (event, data)
        .prop_map(|(event, data)| SseFrame { event, data })
        .prop_filter("frame must carry event or data", |frame| {
            frame.event.is_some() || frame.data.is_some()
        })
}

fn chunks<'a>(bytes: &'a [u8], cuts: &[usize]) -> Vec<&'a [u8]> {
    let mut points = vec![0, bytes.len()];
    points.extend(cuts.iter().map(|cut| cut % (bytes.len() + 1)));
    points.sort_unstable();
    points.dedup();

    points
        .windows(2)
        .filter_map(|window| {
            let start = window[0];
            let end = window[1];
            (start < end).then_some(&bytes[start..end])
        })
        .collect()
}

fn decode_request(protocol: ProtocolId, body: &[u8]) -> Result<Conversation, Error> {
    match protocol {
        ProtocolId::Chat => Chat.decode_request(body),
        ProtocolId::Messages => Messages.decode_request(body),
        ProtocolId::Responses => Responses.decode_request(body),
        _ => unreachable!(),
    }
}

fn decode_response(protocol: ProtocolId, body: &[u8]) -> Result<AssistantOutput, Error> {
    match protocol {
        ProtocolId::Chat => Chat.decode_response(body),
        ProtocolId::Messages => Messages.decode_response(body),
        ProtocolId::Responses => Responses.decode_response(body),
        _ => unreachable!(),
    }
}

fn request_body(protocol: ProtocolId, text: &str, stream: bool) -> Vec<u8> {
    let value = match protocol {
        ProtocolId::Chat => serde_json::json!({
            "model": "property-model",
            "stream": stream,
            "messages": [{"role": "user", "content": text}],
            "max_completion_tokens": 64
        }),
        ProtocolId::Messages => serde_json::json!({
            "model": "property-model",
            "stream": stream,
            "max_tokens": 64,
            "messages": [{"role": "user", "content": text}]
        }),
        ProtocolId::Responses => serde_json::json!({
            "model": "property-model",
            "stream": stream,
            "input": text,
            "max_output_tokens": 64
        }),
        _ => unreachable!(),
    };
    value.to_string().into_bytes()
}

fn text_response(protocol: ProtocolId, text: &str) -> Vec<u8> {
    let value = match protocol {
        ProtocolId::Chat => serde_json::json!({
            "id": "chatcmpl-property",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": text},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        }),
        ProtocolId::Messages => serde_json::json!({
            "id": "msg_property",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": text}],
            "model": "property-model",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 1, "output_tokens": 1}
        }),
        ProtocolId::Responses => serde_json::json!({
            "id": "resp_property",
            "object": "response",
            "status": "completed",
            "model": "property-model",
            "output": [{
                "type": "message",
                "id": "msg_property",
                "role": "assistant",
                "content": [{"type": "output_text", "text": text}]
            }],
            "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}
        }),
        _ => unreachable!(),
    };
    value.to_string().into_bytes()
}

fn tool_response(
    protocol: ProtocolId,
    tool_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Vec<u8> {
    let arguments_text = arguments.to_string();
    let value = match protocol {
        ProtocolId::Chat => serde_json::json!({
            "id": "chatcmpl-tool-property",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": tool_id,
                        "type": "function",
                        "function": {"name": tool_name, "arguments": arguments_text}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        }),
        ProtocolId::Messages => serde_json::json!({
            "id": "msg_tool_property",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "tool_use", "id": tool_id, "name": tool_name, "input": arguments}],
            "model": "property-model",
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 1, "output_tokens": 1}
        }),
        ProtocolId::Responses => serde_json::json!({
            "id": "resp_tool_property",
            "object": "response",
            "status": "completed",
            "model": "property-model",
            "output": [{
                "type": "function_call",
                "call_id": tool_id,
                "name": tool_name,
                "arguments": arguments_text
            }],
            "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}
        }),
        _ => unreachable!(),
    };
    value.to_string().into_bytes()
}

fn text_stream(protocol: ProtocolId) -> &'static [u8] {
    match protocol {
        ProtocolId::Chat => concat!(
            "data: {\"id\":\"chatcmpl-property-stream\",\"model\":\"property-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"world\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-property-stream\",\"model\":\"property-model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        )
        .as_bytes(),
        ProtocolId::Messages => concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_property_stream\",\"model\":\"property-model\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"world\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        )
        .as_bytes(),
        ProtocolId::Responses => concat!(
            "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_property_stream\",\"model\":\"property-model\"}}\n\n",
            "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"message\",\"id\":\"msg_property_stream\"}}\n\n",
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"world\"}\n\n",
            "event: response.output_text.done\ndata: {\"type\":\"response.output_text.done\",\"output_index\":0,\"content_index\":0,\"text\":\"world\"}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_property_stream\",\"status\":\"completed\",\"model\":\"property-model\",\"output\":[]}}\n\n",
        )
        .as_bytes(),
        _ => unreachable!(),
    }
}

fn run_stream_one_shot(src: ProtocolId, dst: ProtocolId, stream: &[u8]) -> (Vec<u8>, Termination) {
    let mut converter = converter(src, dst, resolve(src, dst, "property-model")).unwrap();
    let mut request_out = Vec::new();
    converter
        .request(&request_body(src, "hi", true), &mut request_out)
        .unwrap();

    let mut out = Vec::new();
    converter.feed(stream, &mut out).unwrap();
    let mut tail = Vec::new();
    let termination = converter.finish(&mut tail).unwrap();
    out.extend_from_slice(&tail);
    (out, termination)
}

fn run_stream_chunked(
    src: ProtocolId,
    dst: ProtocolId,
    stream: &[u8],
    cuts: &[usize],
) -> (Vec<u8>, Termination) {
    let mut converter = converter(src, dst, resolve(src, dst, "property-model")).unwrap();
    let mut request_out = Vec::new();
    converter
        .request(&request_body(src, "hi", true), &mut request_out)
        .unwrap();

    let mut out = Vec::new();
    for chunk in chunks(stream, cuts) {
        converter.feed(chunk, &mut out).unwrap();
    }
    let mut tail = Vec::new();
    let termination = converter.finish(&mut tail).unwrap();
    out.extend_from_slice(&tail);
    (out, termination)
}

fn response_text(output: &AssistantOutput) -> Option<&str> {
    output
        .choices
        .iter()
        .flat_map(|choice| &choice.parts)
        .find_map(|part| match part {
            Part::Text(text) => Some(text.as_str()),
            _ => None,
        })
}

fn request_text(conversation: &Conversation) -> Option<&str> {
    conversation
        .turns
        .iter()
        .rev()
        .flat_map(|turn| &turn.parts)
        .find_map(|part| match part {
            Part::Text(text) => Some(text.as_str()),
            _ => None,
        })
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

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        max_shrink_iters: 2048,
        .. ProptestConfig::default()
    })]

    #[test]
    fn sse_framer_never_panics_on_arbitrary_bytes(
        chunks in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..256), 0..16)
    ) {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut framer = SseFramer::new();
            let mut out = Vec::new();
            for chunk in chunks {
                let _ = framer.feed(&chunk, &mut out);
            }
            let _ = framer.finish(&mut out);
        }));

        prop_assert!(result.is_ok());
    }

    #[test]
    fn sse_frames_survive_arbitrary_chunk_splits(
        frames in prop::collection::vec(arb_frame(), 1..8),
        cuts in prop::collection::vec(any::<usize>(), 0..24)
    ) {
        let bytes: Vec<u8> = frames.iter().flat_map(encode_frame).collect();
        let mut framer = SseFramer::new();
        let mut out = Vec::new();

        for chunk in chunks(&bytes, &cuts) {
            framer.feed(chunk, &mut out).unwrap();
        }
        framer.finish(&mut out).unwrap();

        prop_assert_eq!(out, frames);
    }

    #[test]
    fn converters_never_panic_on_arbitrary_request_and_response_bytes(
        body in prop::collection::vec(any::<u8>(), 0..1536)
    ) {
        for src in PROTOCOLS {
            for dst in PROTOCOLS {
                let request_result = catch_unwind(AssertUnwindSafe(|| {
                    let mut converter = converter(src, dst, resolve(src, dst, "property-model")).unwrap();
                    let mut out = Vec::new();
                    let _ = converter.request(&body, &mut out);
                }));
                prop_assert!(request_result.is_ok());

                let response_result = catch_unwind(AssertUnwindSafe(|| {
                    let mut converter = converter(src, dst, resolve(src, dst, "property-model")).unwrap();
                    let mut out = Vec::new();
                    let _ = converter.response(&body, &mut out);
                }));
                prop_assert!(response_result.is_ok());
            }
        }
    }

    #[test]
    fn converters_never_panic_on_arbitrary_stream_bytes(
        body in prop::collection::vec(any::<u8>(), 0..1536)
    ) {
        for src in PROTOCOLS {
            for dst in PROTOCOLS {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    let mut converter = converter(src, dst, resolve(src, dst, "property-model")).unwrap();
                    let mut request_out = Vec::new();
                    converter
                        .request(&request_body(src, "hi", true), &mut request_out)
                        .unwrap();

                    let mut out = Vec::new();
                    converter.feed(&body, &mut out).unwrap();
                    let mut tail = Vec::new();
                    let _ = converter.finish(&mut tail).unwrap();
                }));
                prop_assert!(result.is_ok());
            }
        }
    }

    #[test]
    fn converter_text_equivalence_covers_all_directions(
        user in "[A-Za-z0-9 .,:;!?_-]{1,64}",
        assistant in "[A-Za-z0-9 .,:;!?_-]{1,64}",
    ) {
        for src in PROTOCOLS {
            for dst in PROTOCOLS {
                let mut converter = converter(src, dst, resolve(src, dst, "property-model")).unwrap();
                let mut outbound = Vec::new();
                converter
                    .request(&request_body(src, &user, false), &mut outbound)
                    .unwrap();

                let target_request = decode_request(dst, &outbound).unwrap();
                prop_assert_eq!(request_text(&target_request), Some(user.as_str()));

                let mut inbound = Vec::new();
                converter
                    .response(&text_response(dst, &assistant), &mut inbound)
                    .unwrap();
                let source_response = decode_response(src, &inbound).unwrap();
                prop_assert_eq!(response_text(&source_response), Some(assistant.as_str()));
            }
        }
    }

    #[test]
    fn converter_tool_equivalence_covers_all_directions(
        tool_id in "call_[A-Za-z0-9_-]{1,16}",
        tool_name in "[a-z][a-z0-9_]{0,15}",
        arg_value in "[A-Za-z0-9 .,:;!?_-]{0,48}",
    ) {
        let arguments = serde_json::json!({"value": arg_value});
        for src in PROTOCOLS {
            for dst in PROTOCOLS {
                let mut converter = converter(src, dst, resolve(src, dst, "property-model")).unwrap();
                let mut outbound = Vec::new();
                converter
                    .request(&request_body(src, "Use tool", false), &mut outbound)
                    .unwrap();

                let mut inbound = Vec::new();
                converter
                    .response(&tool_response(dst, &tool_id, &tool_name, &arguments), &mut inbound)
                    .unwrap();
                let source_response = decode_response(src, &inbound).unwrap();
                let tool_use = find_tool_use(&source_response).unwrap();

                prop_assert_eq!(tool_use.id.0.as_ref(), tool_id.as_str());
                prop_assert_eq!(tool_use.name.as_ref(), tool_name.as_str());
                let decoded_arguments: serde_json::Value =
                    serde_json::from_str(tool_use.arguments.raw()).unwrap();
                prop_assert_eq!(decoded_arguments, arguments.clone());
            }
        }
    }

    #[test]
    fn converter_stream_output_is_invariant_under_arbitrary_chunk_splits(
        cuts in prop::collection::vec(any::<usize>(), 0..32)
    ) {
        for src in PROTOCOLS {
            for dst in PROTOCOLS {
                let stream = text_stream(dst);
                let baseline = run_stream_one_shot(src, dst, stream);
                let chunked = run_stream_chunked(src, dst, stream, &cuts);
                prop_assert_eq!(chunked, baseline);
            }
        }
    }
}
