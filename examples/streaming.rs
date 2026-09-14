use llmwire::{converter, resolve, ProtocolId};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut converter = converter(
        ProtocolId::Chat,
        ProtocolId::Messages,
        resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-test"),
    )?;

    let client_request = br#"{
        "model": "claude-test",
        "stream": true,
        "messages": [{"role": "user", "content": "hello"}],
        "max_completion_tokens": 32
    }"#;

    let mut upstream_request = Vec::new();
    converter.request(client_request, &mut upstream_request)?;

    let upstream_chunk = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-test\"}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );

    let mut client_stream = Vec::new();
    converter.feed(upstream_chunk.as_bytes(), &mut client_stream)?;

    let mut tail = Vec::new();
    let termination = converter.finish(&mut tail)?;
    client_stream.extend_from_slice(&tail);

    println!(
        "upstream request:\n{}",
        String::from_utf8_lossy(&upstream_request)
    );
    println!(
        "client stream:\n{}",
        String::from_utf8_lossy(&client_stream)
    );
    println!("termination: {termination:?}");

    let report = converter.take_report();
    println!(
        "report: {} unmapped, {} warnings",
        report.unmapped.len(),
        report.warnings.len()
    );

    Ok(())
}
