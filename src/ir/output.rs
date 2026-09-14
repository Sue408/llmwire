use super::{Part, Usage};

#[derive(Debug, Clone, Default)]
pub struct AssistantOutput {
    pub choices: Vec<Choice>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Default)]
pub struct Choice {
    pub index: u32,
    pub parts: Vec<Part>,
    pub finish: Finish,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum StopReason {
    #[default]
    EndTurn,
    MaxTokens,
    StopSequence,
    ToolUse,
    ContentFilter,
    Pause,
    Cancelled,
    Other(Box<str>),
}

#[derive(Debug, Clone, Default)]
pub struct Finish {
    pub canonical: StopReason,
    pub provider_raw: Box<str>,
}
