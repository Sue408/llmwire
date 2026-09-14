//! 规范化模型输出。
use super::{Part, Usage};

/// 一次模型响应的规范化输出。
#[derive(Debug, Clone, Default)]
pub struct AssistantOutput {
    /// 候选输出。
    pub choices: Vec<Choice>,
    /// usage 信息，未知值保持未知。
    pub usage: Usage,
}

/// 单个候选输出。
#[derive(Debug, Clone, Default)]
pub struct Choice {
    /// 候选索引。
    pub index: u32,
    /// 有序输出 part。
    pub parts: Vec<Part>,
    /// 完成原因。
    pub finish: Finish,
}

/// 规范化停止原因。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum StopReason {
    /// 正常结束。
    #[default]
    EndTurn,
    /// 达到 token 上限。
    MaxTokens,
    /// 命中停止序列。
    StopSequence,
    /// 请求工具调用。
    ToolUse,
    /// 内容过滤。
    ContentFilter,
    /// 暂停。
    Pause,
    /// 取消。
    Cancelled,
    /// 其他供应商原因。
    Other(Box<str>),
}

/// 规范化停止原因与供应商原始值。
#[derive(Debug, Clone, Default)]
pub struct Finish {
    /// 规范化原因。
    pub canonical: StopReason,
    /// 供应商原始 finish reason。
    pub provider_raw: Box<str>,
}
