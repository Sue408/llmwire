#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProtocolId {
    Chat,
    Messages,
    Responses,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OpaqueKind {
    AnthropicRedactedThinking,
    AnthropicThinkingSignature,
    ResponsesEncryptedReasoning,
    GeminiThoughtSignature,
    ProviderSpecific(&'static str),
}
