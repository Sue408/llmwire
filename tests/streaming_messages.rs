mod streaming_messages {
    use llmwire::codec::{Chat, Messages, ProtocolCodec, StreamDecode};
    use llmwire::framing::{SseFrame, SseFramer};
    use llmwire::ids::OpaqueKind;
    use llmwire::ir::{Event, Part, StopReason, StreamState, Termination, ToolUseKind};

    fn frames(input: &[u8]) -> Vec<SseFrame> {
        let mut framer = SseFramer::new();
        let mut frames = Vec::new();
        framer.feed(input, &mut frames).unwrap();
        framer.finish(&mut frames).unwrap();
        frames
    }

    fn event_frame(event: &str, data: &str) -> String {
        format!("event: {event}\ndata: {data}\n\n")
    }

    fn decode_with(
        codec: &dyn ProtocolCodec,
        input: &[u8],
    ) -> (StreamState, Vec<Event>, Option<Termination>) {
        let mut state = StreamState::new();
        let mut all_events = Vec::new();
        let mut termination = None;

        for frame in frames(input) {
            let StreamDecode {
                events,
                termination: frame_termination,
            } = codec.decode_stream_frame(&frame, &state).unwrap();
            state.apply_all(events.clone()).unwrap();
            all_events.extend(events);
            if frame_termination.is_some() {
                termination = frame_termination;
            }
        }

        (state, all_events, termination)
    }

    #[test]
    fn decodes_text_and_merges_usage() {
        let input = [
            event_frame(
                "message_start",
                r#"{"type":"message_start","message":{"id":"msg_1","model":"claude-test","usage":{"input_tokens":10,"cache_read_input_tokens":2,"cache_creation_input_tokens":1}}}"#,
            ),
            event_frame(
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            ),
            event_frame(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hel"}}"#,
            ),
            event_frame(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}}"#,
            ),
            event_frame(
                "content_block_stop",
                r#"{"type":"content_block_stop","index":0}"#,
            ),
            event_frame(
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":4}}"#,
            ),
            event_frame("message_stop", r#"{"type":"message_stop"}"#),
        ]
        .concat();

        let (state, events, termination) = decode_with(&Messages, input.as_bytes());

        assert_eq!(termination, Some(Termination::Explicit));
        assert!(matches!(events[0], Event::MessageStart { .. }));
        assert!(matches!(events[1], Event::UsagePatch(_)));
        let output = state.assistant_output().unwrap();
        assert!(matches!(
            &output.choices[0].parts[0],
            Part::Text(text) if text == "hello"
        ));
        assert_eq!(output.choices[0].finish.canonical, StopReason::EndTurn);
        assert_eq!(state.usage().input, Some(13));
        assert_eq!(state.usage().cached, 2);
        assert_eq!(state.usage().cache_creation, 1);
        assert_eq!(state.usage().output, Some(4));
    }

    #[test]
    fn decodes_server_tool_use_with_partial_json() {
        let input = [
            event_frame(
                "message_start",
                r#"{"type":"message_start","message":{"id":"msg_1","model":"claude-test"}}"#,
            ),
            event_frame(
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"srvtoolu_01","name":"web_search","input":{}}}"#,
            ),
            event_frame(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"q\":\""}}"#,
            ),
            event_frame(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"rust\"}"}}"#,
            ),
            event_frame(
                "content_block_stop",
                r#"{"type":"content_block_stop","index":0}"#,
            ),
            event_frame(
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            ),
            event_frame("message_stop", r#"{"type":"message_stop"}"#),
        ]
        .concat();

        let (state, _, _) = decode_with(&Messages, input.as_bytes());
        let output = state.assistant_output().unwrap();
        let Part::ToolUse(tool) = &output.choices[0].parts[0] else {
            panic!("expected tool use");
        };
        assert_eq!(tool.id.0.as_ref(), "srvtoolu_01");
        assert_eq!(tool.kind, ToolUseKind::Server);
        assert_eq!(tool.arguments.raw(), r#"{"q":"rust"}"#);
    }

    #[test]
    fn decodes_thinking_signature_as_opaque() {
        let input = [
            event_frame(
                "message_start",
                r#"{"type":"message_start","message":{"id":"msg_1","model":"claude-test"}}"#,
            ),
            event_frame(
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            ),
            event_frame(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}"#,
            ),
            event_frame(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig_123"}}"#,
            ),
            event_frame(
                "content_block_stop",
                r#"{"type":"content_block_stop","index":0}"#,
            ),
            event_frame(
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
            ),
            event_frame("message_stop", r#"{"type":"message_stop"}"#),
        ]
        .concat();

        let (state, _, _) = decode_with(&Messages, input.as_bytes());
        let output = state.assistant_output().unwrap();
        let Part::Thinking(thinking) = &output.choices[0].parts[0] else {
            panic!("expected thinking");
        };
        assert_eq!(thinking.text, "hmm");
        let signature = thinking.signature.as_ref().unwrap();
        assert_eq!(signature.kind, OpaqueKind::AnthropicThinkingSignature);
        assert_eq!(signature.bytes.as_ref(), b"sig_123");
    }

    #[test]
    fn ignores_ping_and_stops_explicitly() {
        let input = [
            event_frame(
                "message_start",
                r#"{"type":"message_start","message":{"id":"msg_1","model":"claude-test"}}"#,
            ),
            event_frame("ping", r#"{"type":"ping"}"#),
            event_frame(
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
            ),
            event_frame("message_stop", r#"{"type":"message_stop"}"#),
        ]
        .concat();

        let (state, events, termination) = decode_with(&Messages, input.as_bytes());
        assert_eq!(termination, Some(Termination::Explicit));
        assert_eq!(events.len(), 2);
        assert!(state.is_finished());
    }

    #[test]
    fn encodes_tool_start_with_event_name() {
        let event = Event::PartStart {
            index: 0,
            kind: llmwire::ir::PartKind::ToolUse,
            tool: Some(llmwire::ir::ToolStart {
                source_index: Some(0),
                id: llmwire::ir::ToolId("toolu_01".into()),
                name: "lookup".into(),
                kind: ToolUseKind::Client,
            }),
        };
        let frames = Messages
            .encode_stream_event(&event, &StreamState::new())
            .unwrap();

        assert_eq!(frames[0].event.as_deref(), Some("content_block_start"));
        let json: serde_json::Value =
            serde_json::from_str(frames[0].data.as_deref().unwrap()).unwrap();
        assert_eq!(json["content_block"]["type"], "tool_use");
        assert_eq!(json["content_block"]["id"], "toolu_01");
    }

    #[test]
    fn message_start_marks_estimated_input_usage() {
        let event = Event::MessageStart {
            id: "msg_1".into(),
            model: "claude-test".into(),
        };

        let frames = Messages
            .encode_stream_event(&event, &StreamState::new())
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(frames[0].data.as_deref().unwrap()).unwrap();

        assert_eq!(json["message"]["usage"]["input_tokens"], 0);
        assert_eq!(json["message"]["usage"]["estimated"], true);
    }
    #[test]
    fn converts_chat_text_stream_to_messages() {
        let chat_input = [
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"model-a\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hel\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"model-a\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"model-a\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        ]
        .concat();
        let (_, source_events, _) = decode_with(&Chat, chat_input.as_bytes());

        let mut target_state = StreamState::new();
        let mut termination = None;
        for event in source_events {
            for frame in Messages.encode_stream_event(&event, &target_state).unwrap() {
                let decoded = Messages.decode_stream_frame(&frame, &target_state).unwrap();
                target_state.apply_all(decoded.events).unwrap();
                if decoded.termination.is_some() {
                    termination = decoded.termination;
                }
            }
        }

        assert_eq!(termination, Some(Termination::Explicit));
        let output = target_state.assistant_output().unwrap();
        assert!(matches!(
            &output.choices[0].parts[0],
            Part::Text(text) if text == "hello"
        ));
    }
}
