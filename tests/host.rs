use llmwire::caps::{ParamSet, ThinkingPolicy, ToolIdPolicy};
use llmwire::host::{Host, ModelProfile, StaticHost, UnknownModelPolicy};
use llmwire::{Error, Mode, ProtocolId};

#[test]
fn host_unknown_model_falls_back_to_protocol_defaults() {
    let host = StaticHost::new();
    let caps = host
        .capabilities(ProtocolId::Chat, ProtocolId::Messages, "unknown")
        .unwrap();

    assert_eq!(caps.mode, Mode::Converted);
    assert_eq!(caps.thinking, ThinkingPolicy::Passthrough);
    assert!(caps.passthrough_cache_control);
}

#[test]
fn host_unknown_model_rejection_is_explicit() {
    let host = StaticHost::with_unknown_policy(UnknownModelPolicy::Reject);
    let error = host
        .capabilities(ProtocolId::Chat, ProtocolId::Messages, "unknown")
        .unwrap_err();

    assert!(matches!(error, Error::Unsupported(_)));
}

#[test]
fn host_model_profile_overrides_policy_and_intersects_supported_params() {
    let host = StaticHost::new().with_profile(
        "local-model",
        ModelProfile {
            thinking: Some(ThinkingPolicy::Strip),
            tool_id: Some(ToolIdPolicy::Preserve),
            passthrough_cache_control: Some(false),
            passthrough_betas: Some(false),
            supported: Some(ParamSet::TEMPERATURE | ParamSet::TOP_K | ParamSet::N),
            max_output_tokens: Some(2048),
        },
    );

    let caps = host
        .capabilities(ProtocolId::Messages, ProtocolId::Messages, "local-model")
        .unwrap();

    assert_eq!(caps.mode, Mode::NativePassthrough);
    assert_eq!(caps.thinking, ThinkingPolicy::Strip);
    assert!(!caps.passthrough_cache_control);
    assert!(!caps.passthrough_betas);
    assert!(caps.supports(ParamSet::TEMPERATURE));
    assert!(caps.supports(ParamSet::TOP_K));
    assert!(!caps.supports(ParamSet::N));
    assert_eq!(host.max_output_tokens("local-model"), Some(2048));
    assert_eq!(host.max_output_tokens("unknown"), None);
}
