//! 协议与不透明内容标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProtocolId {
    /// OpenAI Chat Completions。
    Chat,
    /// Anthropic Messages。
    Messages,
    /// OpenAI Responses。
    Responses,
}

/// 不透明内容的来源类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OpaqueKind {
    /// Anthropic `redacted_thinking`。
    AnthropicRedactedThinking,
    /// Anthropic thinking signature。
    AnthropicThinkingSignature,
    /// OpenAI Responses encrypted reasoning。
    ResponsesEncryptedReasoning,
    /// Gemini thought signature。
    GeminiThoughtSignature,
    /// 其他供应商专用不透明内容。
    ProviderSpecific(&'static str),
}
