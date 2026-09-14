mod converter {
    use llmwire::codec::Chat;
    use llmwire::codec::ProtocolCodec;
    use llmwire::{converter, resolve, Error, ProtocolId, Termination};

    fn streaming_chat_request() -> Vec<u8> {
        serde_json::json!({
            "model": "openai-test",
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}],
            "max_completion_tokens": 64
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn request_and_response_non_stream_roundtrip() {
        let mut converter = converter(
            ProtocolId::Chat,
            ProtocolId::Messages,
            resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-test"),
        )
        .unwrap();

        let request = serde_json::json!({
            "model": "gpt-test",
            "messages": [{"role": "user", "content": "hello"}],
            "max_completion_tokens": 32
        })
        .to_string()
        .into_bytes();
        let mut outbound = Vec::new();
        converter.request(&request, &mut outbound).unwrap();

        let outbound_json: serde_json::Value = serde_json::from_slice(&outbound).unwrap();
        assert_eq!(outbound_json["messages"][0]["role"], "user");
        assert_eq!(outbound_json["max_tokens"], 32);

        let response = br#"{
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "world"}],
            "model": "claude-test",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 3, "output_tokens": 2}
        }"#;
        let mut client_body = Vec::new();
        converter.response(response, &mut client_body).unwrap();

        let client_json: serde_json::Value = serde_json::from_slice(&client_body).unwrap();
        assert_eq!(client_json["choices"][0]["message"]["content"], "world");
        assert_eq!(client_json["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn streams_chat_to_messages_and_finishes_explicitly() {
        let mut converter = converter(
            ProtocolId::Chat,
            ProtocolId::Messages,
            resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-test"),
        )
        .unwrap();

        let mut outbound = Vec::new();
        converter
            .request(&streaming_chat_request(), &mut outbound)
            .unwrap();
        let outbound_json: serde_json::Value = serde_json::from_slice(&outbound).unwrap();
        assert_eq!(outbound_json["stream"], true);

        let input = concat!(
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hel\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-test\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let mut stream_out = Vec::new();
        converter.feed(input.as_bytes(), &mut stream_out).unwrap();
        let text = String::from_utf8(stream_out).unwrap();

        assert!(text.contains("event: message_start"));
        assert!(text.contains("event: content_block_start"));
        assert!(text.contains("event: content_block_delta"));
        assert!(text.contains("event: message_delta"));
        assert!(text.contains("event: message_stop"));

        let mut tail = Vec::new();
        assert_eq!(converter.finish(&mut tail).unwrap(), Termination::Explicit);
        assert!(tail.is_empty());
    }

    #[test]
    fn stream_error_uses_channel_b_and_report() {
        let mut converter = converter(
            ProtocolId::Chat,
            ProtocolId::Messages,
            resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-test"),
        )
        .unwrap();
        let mut request_out = Vec::new();
        converter
            .request(&streaming_chat_request(), &mut request_out)
            .unwrap();

        let mut stream_out = Vec::new();
        converter
            .feed(b"data: {not-json}\n\n", &mut stream_out)
            .unwrap();

        let text = String::from_utf8(stream_out).unwrap();
        assert!(text.contains("event: error"));
        assert!(text.contains("invalid input"));

        let report = converter.take_report();
        assert!(!report.warnings.is_empty());
        assert!(converter.take_report().warnings.is_empty());
    }

    #[test]
    fn finish_without_terminal_is_not_success() {
        let mut converter = converter(
            ProtocolId::Chat,
            ProtocolId::Messages,
            resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-test"),
        )
        .unwrap();
        let mut request_out = Vec::new();
        converter
            .request(&streaming_chat_request(), &mut request_out)
            .unwrap();

        let mut stream_out = Vec::new();
        converter
            .feed(
                b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"x\"},\"finish_reason\":null}]}\n\n",
                &mut stream_out,
            )
            .unwrap();

        let mut tail = Vec::new();
        assert_eq!(
            converter.finish(&mut tail).unwrap(),
            Termination::ClientAbort
        );
    }

    #[test]
    fn rejects_response_body_for_streaming_request() {
        let mut converter = converter(
            ProtocolId::Chat,
            ProtocolId::Messages,
            resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-test"),
        )
        .unwrap();
        let mut request_out = Vec::new();
        converter
            .request(&streaming_chat_request(), &mut request_out)
            .unwrap();

        let mut response_out = Vec::new();
        let error = converter
            .response(br#"{"content":[]}"#, &mut response_out)
            .unwrap_err();
        assert!(matches!(error, Error::Protocol(_)));
    }

    #[test]
    fn streams_chat_to_responses_without_done_marker() {
        let mut converter = converter(
            ProtocolId::Chat,
            ProtocolId::Responses,
            resolve(ProtocolId::Chat, ProtocolId::Responses, "gpt-test"),
        )
        .unwrap();
        let mut request_out = Vec::new();
        converter
            .request(&streaming_chat_request(), &mut request_out)
            .unwrap();

        let input = concat!(
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hel\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-test\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let mut stream_out = Vec::new();
        converter.feed(input.as_bytes(), &mut stream_out).unwrap();
        let text = String::from_utf8(stream_out).unwrap();

        assert!(text.contains("event: response.output_item.added"));
        assert!(text.contains("event: response.output_text.delta"));
        assert!(text.contains("event: response.completed"));
        assert!(!text.contains("[DONE]"));

        let mut tail = Vec::new();
        assert_eq!(converter.finish(&mut tail).unwrap(), Termination::Explicit);
    }
    #[test]
    fn responses_protocol_non_stream_is_supported() {
        let mut converter = converter(
            ProtocolId::Chat,
            ProtocolId::Responses,
            resolve(ProtocolId::Chat, ProtocolId::Responses, "gpt-test"),
        )
        .unwrap();

        let request = serde_json::json!({
            "model": "gpt-test",
            "messages": [{"role": "user", "content": "hello"}],
            "max_completion_tokens": 32
        })
        .to_string()
        .into_bytes();
        let mut outbound = Vec::new();
        converter.request(&request, &mut outbound).unwrap();
        let outbound_json: serde_json::Value = serde_json::from_slice(&outbound).unwrap();
        assert_eq!(outbound_json["input"][0]["type"], "message");
        assert_eq!(
            outbound_json["input"][0]["content"][0]["type"],
            "input_text"
        );

        let response = br#"{
            "id":"resp_1",
            "object":"response",
            "status":"completed",
            "model":"gpt-test",
            "output":[{
                "type":"message",
                "id":"msg_1",
                "role":"assistant",
                "status":"completed",
                "content":[{"type":"output_text","text":"hi","annotations":[]}]
            }]
        }"#;
        let mut inbound = Vec::new();
        converter.response(response, &mut inbound).unwrap();
        let inbound_json: serde_json::Value = serde_json::from_slice(&inbound).unwrap();
        assert_eq!(inbound_json["choices"][0]["message"]["content"], "hi");
    }

    #[test]
    fn facade_stays_byte_oriented_for_codec_roundtrip() {
        let request = serde_json::json!({
            "model": "gpt-test",
            "messages": [{"role": "user", "content": "hello"}]
        })
        .to_string()
        .into_bytes();
        let decoded = Chat.decode_request(&request).unwrap();
        let encoded = Chat.encode_request(&decoded).unwrap();
        assert!(!encoded.is_empty());
    }
}
