mod streaming_chat {
    use llmwire::codec::{Chat, ProtocolCodec, StreamDecode};
    use llmwire::framing::SseFramer;
    use llmwire::ir::{Event, Part, StopReason, StreamState, Termination};
    use llmwire::Error;

    fn frames(input: &[u8]) -> Vec<llmwire::framing::SseFrame> {
        let mut framer = SseFramer::new();
        let mut frames = Vec::new();
        framer.feed(input, &mut frames).unwrap();
        framer.finish(&mut frames).unwrap();
        frames
    }

    fn decode(input: &[u8]) -> (StreamState, Vec<Event>, Option<Termination>) {
        let mut state = StreamState::new();
        let mut all_events = Vec::new();
        let mut termination = None;

        for frame in frames(input) {
            let StreamDecode {
                events,
                termination: frame_termination,
            } = Chat.decode_stream_frame(&frame, &state).unwrap();
            state.apply_all(events.clone()).unwrap();
            all_events.extend(events);
            if frame_termination.is_some() {
                termination = frame_termination;
            }
        }

        (state, all_events, termination)
    }

    fn chat_chunk(json: &str) -> String {
        format!("data: {json}\n\n")
    }

    #[test]
    fn decodes_text_chunks_into_canonical_events() {
        let input = [
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}"#,
            ),
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[{"index":0,"delta":{"content":"hel"},"finish_reason":null}]}"#,
            ),
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[{"index":0,"delta":{"content":"lo"},"finish_reason":null}]}"#,
            ),
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            ),
            chat_chunk("[DONE]"),
        ]
        .concat();

        let (state, events, termination) = decode(input.as_bytes());

        assert_eq!(termination, Some(Termination::Explicit));
        assert!(matches!(events[0], Event::MessageStart { .. }));
        assert!(matches!(
            events[1],
            Event::PartStart {
                kind: llmwire::ir::PartKind::Text,
                ..
            }
        ));
        let output = state.assistant_output().unwrap();
        assert!(matches!(
            &output.choices[0].parts[0],
            Part::Text(text) if text == "hello"
        ));
        assert_eq!(output.choices[0].finish.canonical, StopReason::EndTurn);
    }

    #[test]
    fn empty_content_still_starts_text_part() {
        let input = [
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[{"index":0,"delta":{"content":""},"finish_reason":null}]}"#,
            ),
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            ),
        ]
        .concat();

        let (state, _, _) = decode(input.as_bytes());
        let output = state.assistant_output().unwrap();

        assert!(matches!(
            &output.choices[0].parts[0],
            Part::Text(text) if text.is_empty()
        ));
    }

    #[test]
    fn decodes_parallel_tool_calls_by_index() {
        let input = [
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}"#,
            ),
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_b","type":"function","function":{"name":"beta","arguments":"{\"b\":"}}]},"finish_reason":null}]}"#,
            ),
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"alpha","arguments":"{\"a\":1"}}]},"finish_reason":null}]}"#,
            ),
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":"2}"}}]},"finish_reason":null}]}"#,
            ),
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"}"}}]},"finish_reason":null}]}"#,
            ),
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
            ),
            chat_chunk("[DONE]"),
        ]
        .concat();

        let (state, _, termination) = decode(input.as_bytes());
        assert_eq!(termination, Some(Termination::Explicit));

        let output = state.assistant_output().unwrap();
        let Part::ToolUse(first) = &output.choices[0].parts[0] else {
            panic!("expected first tool use");
        };
        let Part::ToolUse(second) = &output.choices[0].parts[1] else {
            panic!("expected second tool use");
        };
        assert_eq!(first.id.0.as_ref(), "call_a");
        assert_eq!(first.arguments.raw(), r#"{"a":1}"#);
        assert_eq!(second.id.0.as_ref(), "call_b");
        assert_eq!(second.arguments.raw(), r#"{"b":2}"#);
        assert_eq!(output.choices[0].finish.canonical, StopReason::ToolUse);
    }

    #[test]
    fn merges_usage_chunk() {
        let input = [
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":null}]}"#,
            ),
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            ),
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":4,"prompt_tokens_details":{"cached_tokens":2},"completion_tokens_details":{"reasoning_tokens":1}}}"#,
            ),
            chat_chunk("[DONE]"),
        ]
        .concat();

        let (state, _, _) = decode(input.as_bytes());
        let usage = state.usage();

        assert_eq!(usage.input, Some(10));
        assert_eq!(usage.output, Some(4));
        assert_eq!(usage.cached, 2);
        assert_eq!(usage.reasoning, 1);
    }

    #[test]
    fn encodes_tool_start_metadata() {
        let state = StreamState::new();
        let event = Event::PartStart {
            index: 0,
            kind: llmwire::ir::PartKind::ToolUse,
            tool: Some(llmwire::ir::ToolStart {
                source_index: Some(0),
                id: llmwire::ir::ToolId("call_01".into()),
                name: "lookup".into(),
                kind: llmwire::ir::ToolUseKind::Client,
            }),
        };

        let frames = Chat.encode_stream_event(&event, &state).unwrap();
        let data = frames[0].data.as_deref().unwrap();
        let json: serde_json::Value = serde_json::from_str(data).unwrap();

        assert_eq!(
            json["choices"][0]["delta"]["tool_calls"][0]["id"],
            "call_01"
        );
        assert_eq!(
            json["choices"][0]["delta"]["tool_calls"][0]["function"]["name"],
            "lookup"
        );
    }

    #[test]
    fn rejects_unsupported_multiple_choices_explicitly() {
        let state = StreamState::new();
        let frame = frames(
            chat_chunk(
                r#"{"id":"chatcmpl-1","model":"model-a","choices":[{"index":1,"delta":{"content":"x"},"finish_reason":null}]}"#,
            )
            .as_bytes(),
        )
        .remove(0);

        let error = Chat.decode_stream_frame(&frame, &state).unwrap_err();
        assert!(matches!(error, Error::Unsupported(_)));
    }
}
