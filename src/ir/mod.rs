//! 协议无关的中间表示。
//!
//! IR 保留跨协议语义、流式事件与不透明内容；其稳定规则见 `docs/spec/IR.md`。
mod event;
mod output;
mod part;
mod sampling;
mod state;
mod tool;
mod usage;

pub use crate::ids::OpaqueKind;
pub use event::{Delta, Event, PartKind, Termination, ToolStart, UsagePatch};
pub use output::{AssistantOutput, Choice, Finish, StopReason};
pub use part::{ImageRef, ImageSource, Opaque, Part, Thinking};
pub use sampling::{Reasoning, ReasoningEffort, Sampling};
pub use state::StreamState;
pub use tool::{
    RawJson, ToolChoice, ToolDef, ToolId, ToolResult, ToolResultContent, ToolUse, ToolUseKind,
};
pub use usage::Usage;

/// 一次请求的规范化对话。
#[derive(Debug, Clone, Default)]
pub struct Conversation {
    /// 顶层 system 内容。
    pub system: Vec<Part>,
    /// 按顺序排列的 user / assistant 回合。
    pub turns: Vec<Turn>,
    /// 可用工具定义。
    pub tools: Vec<ToolDef>,
    /// 工具选择策略。
    pub tool_choice: ToolChoice,
    /// 采样参数。
    pub sampling: Sampling,
    /// 推理参数。
    pub reasoning: Reasoning,
}

/// IR 对话角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// 用户或工具结果回合。
    User,
    /// 模型输出回合。
    Assistant,
}

/// 一个角色回合，内容始终表达为有序 part。
#[derive(Debug, Clone)]
pub struct Turn {
    /// 回合角色。
    pub role: Role,
    /// 有序内容块。
    pub parts: Vec<Part>,
}
