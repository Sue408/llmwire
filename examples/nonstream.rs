use llmwire::{converter, resolve, ProtocolId};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut converter = converter(
        ProtocolId::Chat,
        ProtocolId::Messages,
        resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-test"),
    )?;

    let client_request = br#"{
        "model": "claude-test",
        "messages": [{"role": "user", "content": "hello"}],
        "max_completion_tokens": 32
    }"#;

    let mut upstream_request = Vec::new();
    converter.request(client_request, &mut upstream_request)?;

    let backend_response = br#"{
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "world"}],
        "model": "claude-test",
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 3, "output_tokens": 2}
    }"#;

    let mut client_response = Vec::new();
    converter.response(backend_response, &mut client_response)?;

    println!(
        "upstream request:\n{}",
        String::from_utf8_lossy(&upstream_request)
    );
    println!(
        "client response:\n{}",
        String::from_utf8_lossy(&client_response)
    );

    let report = converter.take_report();
    println!(
        "report: {} unmapped, {} warnings",
        report.unmapped.len(),
        report.warnings.len()
    );

    Ok(())
}
