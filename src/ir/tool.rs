//! 工具定义、调用与工具结果。
use std::sync::OnceLock;

use super::Part;

/// 字节保真的工具调用 ID。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolId(pub Box<str>);

/// 工具定义。
#[derive(Debug, Clone)]
pub struct ToolDef {
    /// 工具名。
    pub name: Box<str>,
    /// 工具描述。
    pub description: Option<Box<str>>,
    /// JSON Schema 参数定义，保留原始 JSON 文本。
    pub parameters: RawJson,
    /// 是否启用 strict schema。
    pub strict: Option<bool>,
}

/// 模型发起的工具调用。
#[derive(Debug, Clone)]
pub struct ToolUse {
    /// 工具调用 ID。
    pub id: ToolId,
    /// 工具名。
    pub name: Box<str>,
    /// 原始参数 JSON。
    pub arguments: RawJson,
    /// 工具调用类型。
    pub kind: ToolUseKind,
}

/// 回传给模型的工具执行结果。
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// 对应的工具调用 ID。
    pub tool_use_id: ToolId,
    /// 结果内容。
    pub content: ToolResultContent,
}

/// 工具结果内容。
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ToolResultContent {
    /// 纯文本结果。
    Text(Box<str>),
    /// 多 part 结果。
    Parts(Vec<Part>),
    /// 原始 JSON 对象结果。
    Object(RawJson),
}

/// 工具调用类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolUseKind {
    /// 客户端执行。
    Client,
    /// 服务端执行。
    Server,
    /// 远程工具。
    Remote {
        /// 远程服务名。
        server: Option<Box<str>>,
    },
}

/// 工具选择策略。
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum ToolChoice {
    /// 自动选择。
    #[default]
    Auto,
    /// 必须调用工具。
    Required,
    /// 禁止调用工具。
    None,
    /// 指定工具名。
    Named(Box<str>),
    /// 其他供应商表示。
    Other(RawJson),
}

/// 保留原始 JSON 文本与惰性解析结果的容器。
#[derive(Debug, Clone)]
pub struct RawJson {
    raw: Box<str>,
    parsed: OnceLock<Option<serde_json::Value>>,
}

impl RawJson {
    /// 从原始 JSON 文本构造。
    pub fn from_raw(raw: impl Into<Box<str>>) -> Self {
        Self {
            raw: raw.into(),
            parsed: OnceLock::new(),
        }
    }

    /// 返回原始 JSON 文本。
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// 惰性解析并返回 JSON 值；原始文本非法时返回 `None`。
    pub fn parsed(&self) -> Option<&serde_json::Value> {
        self.parsed
            .get_or_init(|| serde_json::from_str(&self.raw).ok())
            .as_ref()
    }
}
