use llmwire::codec::{ProtocolCodec, Responses};
use llmwire::framing::{encode_frame, SseFrame};
use llmwire::ids::OpaqueKind;
use llmwire::ir::{Delta, Event, Part, PartKind, StreamState, Termination, ToolId};
use serde_json::json;

fn frame(kind: &str, value: serde_json::Value) -> SseFrame {
    SseFrame {
        event: Some(kind.to_owned()),
        data: Some(value.to_string()),
    }
}

fn apply_and_encode(event: Event, state: &mut StreamState) -> Vec<SseFrame> {
    state.apply(event.clone()).unwrap();
    Responses.encode_stream_event(&event, state).unwrap()
}

fn encoded(frames: Vec<SseFrame>) -> String {
    let mut bytes = Vec::new();
    for frame in frames {
        bytes.extend_from_slice(&encode_frame(&frame));
    }
    String::from_utf8(bytes).unwrap()
}

#[test]
fn decodes_text_lifecycle_and_completes_explicitly() {
    let mut state = StreamState::new();

    let created = Responses
        .decode_stream_frame(
            &frame(
                "response.created",
                json!({"type":"response.created","response":{"id":"resp_1","model":"gpt-test"}}),
            ),
            &state,
        )
        .unwrap();
    assert!(matches!(
        created.events.as_slice(),
        [Event::MessageStart { id, model }] if id.as_ref() == "resp_1" && model.as_ref() == "gpt-test"
    ));
    state.apply_all(created.events.clone()).unwrap();

    let added = Responses
        .decode_stream_frame(
            &frame(
                "response.output_item.added",
                json!({
                    "type":"response.output_item.added",
                    "output_index":0,
                    "item":{"type":"message","id":"msg_1","content":[]}
                }),
            ),
            &state,
        )
        .unwrap();
    assert!(matches!(
        added.events.as_slice(),
        [Event::PartStart {
            kind: PartKind::Text,
            ..
        }]
    ));
    state.apply_all(added.events.clone()).unwrap();

    let content = Responses
        .decode_stream_frame(
            &frame(
                "response.content_part.added",
                json!({
                    "type":"response.content_part.added",
                    "output_index":0,
                    "content_index":0,
                    "part":{"type":"output_text","text":""}
                }),
            ),
            &state,
        )
        .unwrap();
    assert!(content.events.is_empty());

    for text in ["hel", "lo"] {
        let decoded = Responses
            .decode_stream_frame(
                &frame(
                    "response.output_text.delta",
                    json!({
                        "type":"response.output_text.delta",
                        "output_index":0,
                        "content_index":0,
                        "delta":text
                    }),
                ),
                &state,
            )
            .unwrap();
        state.apply_all(decoded.events.clone()).unwrap();
    }

    let stopped = Responses
        .decode_stream_frame(
            &frame(
                "response.output_text.done",
                json!({
                    "type":"response.output_text.done",
                    "output_index":0,
                    "content_index":0,
                    "text":"hello"
                }),
            ),
            &state,
        )
        .unwrap();
    assert!(matches!(
        stopped.events.as_slice(),
        [Event::PartStop { .. }]
    ));
    state.apply_all(stopped.events.clone()).unwrap();

    let completed = Responses
        .decode_stream_frame(
            &frame(
                "response.completed",
                json!({
                    "type":"response.completed",
                    "response":{
                        "id":"resp_1",
                        "status":"completed",
                        "model":"gpt-test",
                        "output":[{
                            "type":"message",
                            "id":"msg_1",
                            "role":"assistant",
                            "status":"completed",
                            "content":[{"type":"output_text","text":"hello"}]
                        }],
                        "usage":{"input_tokens":10,"output_tokens":5}
                    }
                }),
            ),
            &state,
        )
        .unwrap();
    assert_eq!(completed.termination, Some(Termination::Explicit));
    assert!(completed
        .events
        .iter()
        .any(|event| matches!(event, Event::Finish(_))));
    state.apply_all(completed.events.clone()).unwrap();

    let output = state.assistant_output().unwrap();
    assert!(matches!(
        &output.choices[0].parts[..],
        [Part::Text(text)] if text == "hello"
    ));
    assert_eq!(output.usage.input, Some(10));
    assert_eq!(output.usage.output, Some(5));
}

#[test]
fn decodes_parallel_function_calls_by_output_index() {
    let mut state = StreamState::new();

    for (output_index, call_id, name) in [(0_u32, "call_a", "lookup"), (1_u32, "call_b", "weather")]
    {
        let start = Responses
            .decode_stream_frame(
                &frame(
                    "response.output_item.added",
                    json!({
                        "type":"response.output_item.added",
                        "output_index":output_index,
                        "item":{
                            "type":"function_call",
                            "id":format!("fc_{output_index}"),
                            "call_id":call_id,
                            "name":name,
                            "arguments":""
                        }
                    }),
                ),
                &state,
            )
            .unwrap();
        state.apply_all(start.events).unwrap();
    }

    for (output_index, delta) in [
        (0_u32, r#"{"a":"#),
        (1_u32, r#"{"city":"#),
        (0_u32, "1}"),
        (1_u32, "\"Paris\"}"),
    ] {
        let decoded = Responses
            .decode_stream_frame(
                &frame(
                    "response.function_call_arguments.delta",
                    json!({
                        "type":"response.function_call_arguments.delta",
                        "output_index":output_index,
                        "delta":delta
                    }),
                ),
                &state,
            )
            .unwrap();
        state.apply_all(decoded.events).unwrap();
    }

    for output_index in [0_u32, 1_u32] {
        let done = Responses
            .decode_stream_frame(
                &frame(
                    "response.function_call_arguments.done",
                    json!({
                        "type":"response.function_call_arguments.done",
                        "output_index":output_index,
                        "arguments": if output_index == 0 { r#"{"a":1}"# } else { r#"{"city":"Paris"}"# }
                    }),
                ),
                &state,
            )
            .unwrap();
        state.apply_all(done.events).unwrap();
    }

    let completed = Responses
        .decode_stream_frame(
            &frame(
                "response.completed",
                json!({
                    "type":"response.completed",
                    "response":{
                        "id":"resp_1",
                        "status":"completed",
                        "model":"gpt-test",
                        "output":[
                            {"type":"function_call","id":"fc_0","call_id":"call_a","name":"lookup","arguments":"{\"a\":1}"},
                            {"type":"function_call","id":"fc_1","call_id":"call_b","name":"weather","arguments":"{\"city\":\"Paris\"}"}
                        ]
                    }
                }),
            ),
            &state,
        )
        .unwrap();
    state.apply_all(completed.events).unwrap();

    let output = state.assistant_output().unwrap();
    let tools = output.choices[0]
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::ToolUse(tool) => Some(tool),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].id, ToolId("call_a".into()));
    assert_eq!(tools[0].arguments.raw(), r#"{"a":1}"#);
    assert_eq!(tools[1].id, ToolId("call_b".into()));
    assert_eq!(tools[1].arguments.raw(), r#"{"city":"Paris"}"#);
}

#[test]
fn keeps_content_index_distinct_inside_one_message() {
    let mut state = StreamState::new();
    let added = Responses
        .decode_stream_frame(
            &frame(
                "response.output_item.added",
                json!({
                    "type":"response.output_item.added",
                    "output_index":0,
                    "item":{"type":"message","id":"msg_1","content":[]}
                }),
            ),
            &state,
        )
        .unwrap();
    state.apply_all(added.events).unwrap();

    for (content_index, text) in [(0_u32, "first"), (1_u32, "second")] {
        let added = Responses
            .decode_stream_frame(
                &frame(
                    "response.content_part.added",
                    json!({
                        "type":"response.content_part.added",
                        "output_index":0,
                        "content_index":content_index,
                        "part":{"type":"output_text","text":""}
                    }),
                ),
                &state,
            )
            .unwrap();
        state.apply_all(added.events).unwrap();

        let delta = Responses
            .decode_stream_frame(
                &frame(
                    "response.output_text.delta",
                    json!({
                        "type":"response.output_text.delta",
                        "output_index":0,
                        "content_index":content_index,
                        "delta":text
                    }),
                ),
                &state,
            )
            .unwrap();
        state.apply_all(delta.events).unwrap();

        let stopped = Responses
            .decode_stream_frame(
                &frame(
                    "response.output_text.done",
                    json!({
                        "type":"response.output_text.done",
                        "output_index":0,
                        "content_index":content_index,
                        "text":text
                    }),
                ),
                &state,
            )
            .unwrap();
        state.apply_all(stopped.events).unwrap();
    }

    let finish = Responses
        .decode_stream_frame(
            &frame(
                "response.completed",
                json!({
                    "type":"response.completed",
                    "response":{
                        "id":"resp_1",
                        "status":"completed",
                        "model":"gpt-test",
                        "output":[{
                            "type":"message",
                            "id":"msg_1",
                            "role":"assistant",
                            "content":[{"type":"output_text","text":"first"},{"type":"output_text","text":"second"}]
                        }]
                    }
                }),
            ),
            &state,
        )
        .unwrap();
    state.apply_all(finish.events).unwrap();

    let output = state.assistant_output().unwrap();
    let parts = &output.choices[0].parts;
    assert!(matches!(parts[0], Part::Text(ref text) if text == "first"));
    assert!(matches!(parts[1], Part::Text(ref text) if text == "second"));
}

#[test]
fn output_item_done_preserves_encrypted_reasoning() {
    let mut state = StreamState::new();
    let decoded = Responses
        .decode_stream_frame(
            &frame(
                "response.output_item.done",
                json!({
                    "type":"response.output_item.done",
                    "output_index":0,
                    "item":{
                        "type":"reasoning",
                        "id":"rs_1",
                        "summary":[],
                        "encrypted_content":"opaque-bytes",
                        "status":"completed"
                    }
                }),
            ),
            &state,
        )
        .unwrap();
    assert!(matches!(
        decoded.events.as_slice(),
        [
            Event::PartStart { .. },
            Event::PartDelta { .. },
            Event::PartStop { .. }
        ]
    ));
    state.apply_all(decoded.events).unwrap();

    let output = state.assistant_output().unwrap();
    let opaque = output.choices[0]
        .parts
        .iter()
        .find_map(|part| match part {
            Part::Opaque(opaque) => Some(opaque),
            _ => None,
        })
        .unwrap();
    assert_eq!(opaque.kind, OpaqueKind::ResponsesEncryptedReasoning);
    assert_eq!(opaque.bytes.as_ref(), b"opaque-bytes");
}
#[test]
fn encodes_completed_without_done_marker() {
    let mut state = StreamState::new();
    let mut output = Vec::new();
    for event in [
        Event::MessageStart {
            id: "resp_1".into(),
            model: "gpt-test".into(),
        },
        Event::PartStart {
            index: 0,
            kind: PartKind::Text,
            tool: None,
        },
        Event::PartDelta {
            index: 0,
            delta: Delta::Text("hi".into()),
        },
        Event::PartStop { index: 0 },
        Event::Finish(llmwire::ir::Finish {
            canonical: llmwire::ir::StopReason::EndTurn,
            provider_raw: "completed".into(),
        }),
    ] {
        output.extend_from_slice(encoded(apply_and_encode(event, &mut state)).as_bytes());
    }

    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("event: response.completed"));
    assert!(text.contains("\"type\":\"response.completed\""));
    assert!(!text.contains("[DONE]"));
    assert!(!text.contains("data: [DONE]"));
}

#[test]
fn failure_and_done_marker_are_explicit() {
    let state = StreamState::new();
    let failed = Responses
        .decode_stream_frame(
            &frame(
                "response.failed",
                json!({
                    "type":"response.failed",
                    "response":{"error":{"message":"boom"}}
                }),
            ),
            &state,
        )
        .unwrap();
    assert_eq!(failed.termination, Some(Termination::Explicit));
    assert!(matches!(failed.events.as_slice(), [Event::Error(_)]));

    let done = SseFrame {
        event: None,
        data: Some("[DONE]".to_owned()),
    };
    let error = Responses.decode_stream_frame(&done, &state).unwrap_err();
    assert!(matches!(error, llmwire::Error::Protocol(_)));
}
