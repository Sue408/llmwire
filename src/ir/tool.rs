use std::sync::OnceLock;

use super::Part;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolId(pub Box<str>);

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: Box<str>,
    pub description: Option<Box<str>>,
    pub parameters: RawJson,
    pub strict: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ToolUse {
    pub id: ToolId,
    pub name: Box<str>,
    pub arguments: RawJson,
    pub kind: ToolUseKind,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_use_id: ToolId,
    pub content: ToolResultContent,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ToolResultContent {
    Text(Box<str>),
    Parts(Vec<Part>),
    Object(RawJson),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolUseKind {
    Client,
    Server,
    Remote { server: Option<Box<str>> },
}

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum ToolChoice {
    #[default]
    Auto,
    Required,
    None,
    Named(Box<str>),
    Other(RawJson),
}

#[derive(Debug, Clone)]
pub struct RawJson {
    raw: Box<str>,
    parsed: OnceLock<Option<serde_json::Value>>,
}

impl RawJson {
    pub fn from_raw(raw: impl Into<Box<str>>) -> Self {
        Self {
            raw: raw.into(),
            parsed: OnceLock::new(),
        }
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn parsed(&self) -> Option<&serde_json::Value> {
        self.parsed
            .get_or_init(|| serde_json::from_str(&self.raw).ok())
            .as_ref()
    }
}
