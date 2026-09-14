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
pub use part::{ImageRef, Opaque, Part, Thinking};
pub use sampling::{Reasoning, ReasoningEffort, Sampling};
pub use state::StreamState;
pub use tool::{
    RawJson, ToolChoice, ToolDef, ToolId, ToolResult, ToolResultContent, ToolUse, ToolUseKind,
};
pub use usage::Usage;

#[derive(Debug, Clone, Default)]
pub struct Conversation {
    pub system: Vec<Part>,
    pub turns: Vec<Turn>,
    pub tools: Vec<ToolDef>,
    pub tool_choice: ToolChoice,
    pub sampling: Sampling,
    pub reasoning: Reasoning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub struct Turn {
    pub role: Role,
    pub parts: Vec<Part>,
}
