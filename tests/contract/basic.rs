use llmwire::codec::{Chat, Messages, ProtocolCodec, Responses};
use llmwire::framing::{SseFrame, SseFramer};
use llmwire::ir::{Part, StopReason, StreamState, Termination};
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
fn chat_text_nonstream_and_stream_contract() {
    let response = br#"{
        "id":"chatcmpl-1",
        "object":"chat.completion",
        "choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}]
    }"#;
    let output = Chat.decode_response(response).unwrap();
    assert!(matches!(&output.choices[0].parts[..], [Part::Text(text)] if text == "hello"));

    let stream = concat!(
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"model-a\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"model-a\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (state, termination) = decode_stream(&Chat, stream.as_bytes()).unwrap();
    assert_eq!(termination, Some(Termination::Explicit));
    assert!(matches!(
        &state.assistant_output().unwrap().choices[0].parts[..],
        [Part::Text(text)] if text == "hello"
    ));
}

#[test]
fn messages_text_nonstream_and_stream_contract() {
    let response = br#"{
        "id":"msg_1",
        "type":"message",
        "role":"assistant",
        "content":[{"type":"text","text":"hello"}],
        "model":"claude-test",
        "stop_reason":"end_turn",
        "usage":{"input_tokens":1,"output_tokens":1}
    }"#;
    let output = Messages.decode_response(response).unwrap();
    assert!(matches!(&output.choices[0].parts[..], [Part::Text(text)] if text == "hello"));

    let stream = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-test\"}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );
    let (state, termination) = decode_stream(&Messages, stream.as_bytes()).unwrap();
    assert_eq!(termination, Some(Termination::Explicit));
    assert!(matches!(
        &state.assistant_output().unwrap().choices[0].parts[..],
        [Part::Text(text)] if text == "hello"
    ));
}

#[test]
fn responses_text_nonstream_and_stream_contract() {
    let response = br#"{
        "id":"resp_1",
        "object":"response",
        "status":"completed",
        "model":"gpt-test",
        "output":[{"type":"message","id":"msg_1","role":"assistant","content":[{"type":"output_text","text":"hello"}]}]
    }"#;
    let output = Responses.decode_response(response).unwrap();
    assert!(matches!(&output.choices[0].parts[..], [Part::Text(text)] if text == "hello"));

    let stream = concat!(
        "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\",\"model\":\"gpt-test\"}}\n\n",
        "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"message\",\"id\":\"msg_1\"}}\n\n",
        "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"hello\"}\n\n",
        "event: response.output_text.done\ndata: {\"type\":\"response.output_text.done\",\"output_index\":0,\"content_index\":0,\"text\":\"hello\"}\n\n",
        "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"model\":\"gpt-test\",\"output\":[]}}\n\n",
    );
    let (state, termination) = decode_stream(&Responses, stream.as_bytes()).unwrap();
    assert_eq!(termination, Some(Termination::Explicit));
    assert!(matches!(
        &state.assistant_output().unwrap().choices[0].parts[..],
        [Part::Text(text)] if text == "hello"
    ));
}

#[test]
fn chat_n_greater_than_one_nonstream_keeps_choices() {
    let response = br#"{
        "id":"chatcmpl-1",
        "choices":[
            {"index":0,"message":{"role":"assistant","content":"a"},"finish_reason":"stop"},
            {"index":1,"message":{"role":"assistant","content":"b"},"finish_reason":"stop"}
        ]
    }"#;
    let output = Chat.decode_response(response).unwrap();
    assert_eq!(output.choices.len(), 2);
    assert_eq!(output.choices[0].index, 0);
    assert_eq!(output.choices[1].index, 1);
}

#[test]
fn responses_status_branches_are_distinct() {
    let completed = Responses
        .decode_response(br#"{"status":"completed","output":[]}"#)
        .unwrap();
    assert_eq!(completed.choices[0].finish.canonical, StopReason::EndTurn);

    let incomplete = Responses
        .decode_response(
            br#"{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[]}"#,
        )
        .unwrap();
    assert_eq!(
        incomplete.choices[0].finish.canonical,
        StopReason::MaxTokens
    );

    let cancelled = Responses
        .decode_response(br#"{"status":"cancelled","output":[]}"#)
        .unwrap();
    assert_eq!(cancelled.choices[0].finish.canonical, StopReason::Cancelled);
}

#[test]
fn responses_output_text_does_not_override_output_items() {
    let output = Responses
        .decode_response(
            br#"{
                "status":"completed",
                "output_text":"convenience",
                "output":[{
                    "type":"message",
                    "role":"assistant",
                    "content":[{"type":"output_text","text":"canonical"}]
                }]
            }"#,
        )
        .unwrap();
    assert!(matches!(&output.choices[0].parts[..], [Part::Text(text)] if text == "canonical"));
}
