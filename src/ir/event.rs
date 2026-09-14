use crate::ids::OpaqueKind;
use crate::Error;

use super::{Finish, Opaque, ToolId, ToolUseKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolStart {
    pub source_index: Option<u32>,
    pub id: ToolId,
    pub name: Box<str>,
    pub kind: ToolUseKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UsagePatch {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cached: Option<u64>,
    pub cache_creation: Option<u64>,
    pub reasoning: Option<u64>,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Event {
    MessageStart {
        id: Box<str>,
        model: Box<str>,
    },
    PartStart {
        index: usize,
        kind: PartKind,
        tool: Option<ToolStart>,
    },
    PartDelta {
        index: usize,
        delta: Delta,
    },
    PartStop {
        index: usize,
    },
    UsagePatch(UsagePatch),
    Finish(Finish),
    Error(Box<Error>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PartKind {
    Text,
    Thinking,
    ToolUse,
    ToolResult,
    Opaque(OpaqueKind),
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Delta {
    Text(Box<str>),
    Thinking(Box<str>),
    ToolArguments(Box<str>),
    Opaque(Opaque),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Termination {
    Explicit,
    CleanClose,
    ClientAbort,
    Timeout,
    NetworkError,
}
