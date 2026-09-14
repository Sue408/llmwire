mod state {
    use llmwire::ids::OpaqueKind;
    use llmwire::ir::{
        Delta, Event, Part, PartKind, StreamState, ToolId, ToolStart, ToolUseKind, UsagePatch,
    };
    use llmwire::Error;

    fn tool_start(id: &str, name: &str) -> ToolStart {
        ToolStart {
            source_index: None,
            id: ToolId(id.into()),
            name: name.into(),
            kind: ToolUseKind::Client,
        }
    }

    #[test]
    fn rejects_delta_before_part_start() {
        let mut state = StreamState::new();

        let error = state
            .apply(Event::PartDelta {
                index: 0,
                delta: Delta::Text("tail".into()),
            })
            .unwrap_err();

        assert!(matches!(error, Error::Protocol(_)));
    }

    #[test]
    fn collects_parallel_tool_arguments_by_index() {
        let mut state = StreamState::new();
        state
            .apply_all([
                Event::PartStart {
                    index: 3,
                    kind: PartKind::ToolUse,
                    tool: Some(tool_start("call_b", "beta")),
                },
                Event::PartStart {
                    index: 1,
                    kind: PartKind::ToolUse,
                    tool: Some(tool_start("call_a", "alpha")),
                },
                Event::PartDelta {
                    index: 3,
                    delta: Delta::ToolArguments("{\"b\":".into()),
                },
                Event::PartDelta {
                    index: 1,
                    delta: Delta::ToolArguments("{\"a\":1".into()),
                },
                Event::PartDelta {
                    index: 3,
                    delta: Delta::ToolArguments("2}".into()),
                },
                Event::PartDelta {
                    index: 1,
                    delta: Delta::ToolArguments("}".into()),
                },
                Event::PartStop { index: 3 },
                Event::PartStop { index: 1 },
            ])
            .unwrap();

        let output = state.assistant_output().unwrap();
        assert_eq!(output.choices[0].parts.len(), 2);
        let Part::ToolUse(first) = &output.choices[0].parts[0] else {
            panic!("expected tool use");
        };
        let Part::ToolUse(second) = &output.choices[0].parts[1] else {
            panic!("expected tool use");
        };
        assert_eq!(first.id.0.as_ref(), "call_a");
        assert_eq!(first.arguments.raw(), "{\"a\":1}");
        assert_eq!(second.id.0.as_ref(), "call_b");
        assert_eq!(second.arguments.raw(), "{\"b\":2}");
    }

    #[test]
    fn opaque_delta_requires_open_matching_block() {
        let mut state = StreamState::new();
        state
            .apply(Event::PartStart {
                index: 0,
                kind: PartKind::Opaque(OpaqueKind::AnthropicRedactedThinking),
                tool: None,
            })
            .unwrap();
        let error = state
            .apply(Event::PartDelta {
                index: 0,
                delta: Delta::Opaque(llmwire::ir::Opaque {
                    kind: OpaqueKind::AnthropicThinkingSignature,
                    bytes: b"sig".to_vec().into_boxed_slice(),
                }),
            })
            .unwrap_err();
        assert!(matches!(error, Error::Protocol(_)));
    }

    #[test]
    fn thinking_accepts_only_thinking_signature() {
        let mut state = StreamState::new();
        state
            .apply(Event::PartStart {
                index: 0,
                kind: PartKind::Thinking,
                tool: None,
            })
            .unwrap();
        state
            .apply(Event::PartDelta {
                index: 0,
                delta: Delta::Opaque(llmwire::ir::Opaque {
                    kind: OpaqueKind::AnthropicThinkingSignature,
                    bytes: b"sig".to_vec().into_boxed_slice(),
                }),
            })
            .unwrap();
        state.apply(Event::PartStop { index: 0 }).unwrap();

        let Part::Thinking(thinking) = state.part(0).unwrap() else {
            panic!("expected thinking");
        };
        assert_eq!(thinking.signature.as_ref().unwrap().bytes.as_ref(), b"sig");
    }

    #[test]
    fn finish_closes_open_blocks_before_output() {
        let mut state = StreamState::new();
        state
            .apply_all([
                Event::PartStart {
                    index: 0,
                    kind: PartKind::Text,
                    tool: None,
                },
                Event::PartDelta {
                    index: 0,
                    delta: Delta::Text("hello".into()),
                },
                Event::Finish(Default::default()),
            ])
            .unwrap();

        assert!(state.is_finished());
        assert!(matches!(
            &state.assistant_output().unwrap().choices[0].parts[0],
            Part::Text(text) if text == "hello"
        ));
    }

    #[test]
    fn usage_patch_does_not_overwrite_known_values() {
        let mut state = StreamState::new();
        state
            .apply(Event::UsagePatch(UsagePatch {
                input: Some(10),
                output: None,
                ..UsagePatch::default()
            }))
            .unwrap();
        state
            .apply(Event::UsagePatch(UsagePatch {
                input: Some(99),
                output: Some(5),
                cached: Some(2),
                ..UsagePatch::default()
            }))
            .unwrap();

        let usage = state.usage();
        assert_eq!(usage.input, Some(10));
        assert_eq!(usage.output, Some(5));
        assert_eq!(usage.cached, 2);
    }

    #[test]
    fn rejects_tool_start_without_metadata() {
        let mut state = StreamState::new();
        let error = state
            .apply(Event::PartStart {
                index: 0,
                kind: PartKind::ToolUse,
                tool: None,
            })
            .unwrap_err();

        assert!(matches!(error, Error::Protocol(_)));
    }
}
