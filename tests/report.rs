use llmwire::codec::{Chat, Messages, ProtocolCodec};
use llmwire::{converter, resolve, Mode, OpaqueKind, ProtocolId, Report, Severity, UnmappedReason};

#[test]
fn report_entries_keep_source_and_severity() {
    let mut report = Report::new();
    report.unmapped(
        "sampling.top_k",
        UnmappedReason::UnsupportedByTarget,
        Severity::Degraded,
    );
    report.warn("request.max_tokens", "deprecated alias", Severity::Degraded);

    assert_eq!(report.unmapped.len(), 1);
    assert_eq!(report.unmapped[0].field.as_ref(), "sampling.top_k");
    assert_eq!(report.unmapped[0].severity, Severity::Degraded);
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].field.as_ref(), "request.max_tokens");
    assert!(!report.is_empty());
}

#[test]
fn report_opaque_records_only_kind_and_len() {
    let mut report = Report::new();
    report.opaque(
        "content[0].opaque",
        OpaqueKind::AnthropicThinkingSignature,
        32,
        Severity::Degraded,
    );

    let warning = &report.warnings[0];
    assert_eq!(warning.field.as_ref(), "content[0].opaque");
    assert!(warning.message.contains("AnthropicThinkingSignature"));
    assert!(warning.message.contains("len=32"));
}

#[test]
fn report_tracks_fatal_entries() {
    let mut report = Report::new();
    assert!(!report.has_fatal());
    report.unmapped(
        "request.previous_response_id",
        UnmappedReason::PolicyBlocked,
        Severity::Fatal,
    );
    assert!(report.has_fatal());
}

#[test]
fn report_chat_deprecated_max_tokens_alias() {
    let mut converter = converter(
        ProtocolId::Chat,
        ProtocolId::Messages,
        resolve(ProtocolId::Chat, ProtocolId::Messages, "model"),
    )
    .unwrap();
    let request = serde_json::json!({
        "model": "model",
        "messages": [{"role": "user", "content": "hello"}],
        "max_tokens": 32
    })
    .to_string()
    .into_bytes();
    let mut out = Vec::new();
    converter.request(&request, &mut out).unwrap();

    let report = converter.take_report();
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.field.as_ref() == "request.max_tokens"));
}

#[test]
fn report_messages_default_max_tokens() {
    let mut report = Report::new();
    let encoded = Messages
        .encode_request_with_report(&llmwire::ir::Conversation::default(), &mut report)
        .unwrap();

    assert!(!encoded.is_empty());
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.field.as_ref() == "request.max_tokens"));
}

#[test]
fn report_unsupported_parameter_drop() {
    let mut converter = converter(
        ProtocolId::Messages,
        ProtocolId::Chat,
        resolve(ProtocolId::Messages, ProtocolId::Chat, "model"),
    )
    .unwrap();
    let request = serde_json::json!({
        "model": "model",
        "max_tokens": 32,
        "top_k": 20,
        "messages": [{"role": "user", "content": "hello"}]
    })
    .to_string()
    .into_bytes();
    let mut out = Vec::new();
    converter.request(&request, &mut out).unwrap();

    let report = converter.take_report();
    assert!(report
        .unmapped
        .iter()
        .any(|entry| entry.field.as_ref() == "sampling.top_k"
            && entry.severity == Severity::Degraded));
}

#[test]
fn report_strict_mode_fails_after_recording_fatal_drop() {
    let mut caps = resolve(ProtocolId::Messages, ProtocolId::Chat, "model");
    caps.mode = Mode::Strict;
    let mut converter = converter(ProtocolId::Messages, ProtocolId::Chat, caps).unwrap();
    let request = serde_json::json!({
        "model": "model",
        "max_tokens": 32,
        "top_k": 20,
        "messages": [{"role": "user", "content": "hello"}]
    })
    .to_string()
    .into_bytes();
    let mut out = Vec::new();

    assert!(converter.request(&request, &mut out).is_err());
    let report = converter.take_report();
    assert!(report.has_fatal());
    assert_eq!(report.unmapped[0].field.as_ref(), "sampling.top_k");
}

#[test]
fn report_stateful_responses_rejection_has_path() {
    let mut converter = converter(
        ProtocolId::Responses,
        ProtocolId::Chat,
        resolve(ProtocolId::Responses, ProtocolId::Chat, "model"),
    )
    .unwrap();
    let request = serde_json::json!({
        "model": "model",
        "store": true,
        "input": "hello"
    })
    .to_string()
    .into_bytes();
    let mut out = Vec::new();

    assert!(converter.request(&request, &mut out).is_err());
    let report = converter.take_report();
    assert_eq!(report.unmapped[0].field.as_ref(), "request.store");
    assert_eq!(report.unmapped[0].reason, UnmappedReason::PolicyBlocked);
    assert_eq!(report.unmapped[0].severity, Severity::Fatal);
}

#[test]
fn report_chat_unknown_request_field() {
    let mut report = Report::new();
    let request = br#"{
        "model": "model",
        "messages": [{"role": "user", "content": "hello"}],
        "metadata": {"trace": "abc"}
    }"#;

    Chat.decode_request_with_report(request, &mut report)
        .unwrap();

    assert!(report
        .unmapped
        .iter()
        .any(|entry| entry.field.as_ref() == "request.metadata"
            && entry.reason == UnmappedReason::UnsupportedByTarget));
}

#[test]
fn report_chat_private_finish_reason() {
    let mut report = Report::new();
    let response = br#"{
        "id": "chatcmpl-1",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "hello"},
            "finish_reason": "vendor_stop"
        }]
    }"#;

    let output = Chat
        .decode_response_with_report(response, &mut report)
        .unwrap();

    assert!(matches!(
        output.choices[0].finish.canonical,
        llmwire::ir::StopReason::Other(ref value) if value.as_ref() == "vendor_stop"
    ));
    assert!(report
        .unmapped
        .iter()
        .any(|entry| entry.field.as_ref() == "choices[0].finish_reason"));
}
