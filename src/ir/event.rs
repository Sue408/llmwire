//! 规范化流式事件。
use crate::ids::OpaqueKind;
use crate::Error;

use super::{Finish, Opaque, ToolId, ToolUseKind};

/// 工具调用 part 开始时的元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolStart {
    /// 供应商原始索引。
    pub source_index: Option<u32>,
    /// 工具调用 ID。
    pub id: ToolId,
    /// 工具名。
    pub name: Box<str>,
    /// 工具调用类型。
    pub kind: ToolUseKind,
}

/// usage 增量更新；未知字段保持 `None`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UsagePatch {
    /// 输入 token。
    pub input: Option<u64>,
    /// 输出 token。
    pub output: Option<u64>,
    /// 缓存命中 token。
    pub cached: Option<u64>,
    /// 缓存创建 token。
    pub cache_creation: Option<u64>,
    /// reasoning token。
    pub reasoning: Option<u64>,
}

/// 协议无关的流式事件。
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Event {
    /// 消息开始。
    MessageStart { id: Box<str>, model: Box<str> },
    /// part 开始。
    PartStart {
        index: usize,
        kind: PartKind,
        tool: Option<ToolStart>,
    },
    /// part 增量。
    PartDelta { index: usize, delta: Delta },
    /// part 结束。
    PartStop { index: usize },
    /// usage 增量。
    UsagePatch(UsagePatch),
    /// 流式完成。
    Finish(Finish),
    /// 流内错误。
    Error(Box<Error>),
}

/// part 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PartKind {
    /// 文本。
    Text,
    /// thinking。
    Thinking,
    /// 工具调用。
    ToolUse,
    /// 工具结果。
    ToolResult,
    /// 不透明内容。
    Opaque(OpaqueKind),
}

/// 流式 part 增量。
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Delta {
    /// 文本增量。
    Text(Box<str>),
    /// thinking 文本增量。
    Thinking(Box<str>),
    /// 工具参数 JSON 片段。
    ToolArguments(Box<str>),
    /// 不透明字节增量。
    Opaque(Opaque),
}

/// 流终止原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Termination {
    /// 收到协议显式终止事件。
    Explicit,
    /// 上游正常关闭，但语义上仍有完整状态。
    CleanClose,
    /// 客户端主动中止。
    ClientAbort,
    /// 超时。
    Timeout,
    /// 网络或分帧错误。
    NetworkError,
}
