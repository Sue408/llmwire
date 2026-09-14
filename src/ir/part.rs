//! 对话内容 part 与图片输入类型。
use std::fmt;

use crate::ids::OpaqueKind;

use super::{ToolResult, ToolUse};

/// 对话中的有序内容块。
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Part {
    /// 纯文本。
    Text(String),
    /// 图片输入。
    Image(ImageRef),
    /// 模型请求调用工具。
    ToolUse(ToolUse),
    /// 工具执行结果。
    ToolResult(ToolResult),
    /// Anthropic 明文 thinking 与可选签名。
    Thinking(Thinking),
    /// 无法规范化但必须字节保真的内容。
    Opaque(Opaque),
}

/// 图片引用及其可选细节提示。
#[derive(Debug, Clone)]
pub struct ImageRef {
    /// 图片来源。
    pub source: ImageSource,
    /// OpenAI 风格 detail 值；目标不支持时会上报。
    pub detail: Option<Box<str>>,
}

/// 图片内容来源。
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ImageSource {
    /// 目标服务端自行获取的 `http(s)` URL。
    RemoteUrl(Box<str>),
    /// 内联 Base64 payload，`data` 不包含 data URI 前缀。
    Base64 {
        /// MIME 类型。
        media_type: Box<str>,
        /// 纯 Base64 payload。
        data: Box<str>,
    },
}

#[doc = "```compile_fail"]
#[doc = "fn assert_display<T: std::fmt::Display>(_: T) {}"]
#[doc = "assert_display(llmwire::ir::Opaque { kind: llmwire::ids::OpaqueKind::ProviderSpecific(\"test\"), bytes: Box::<[u8]>::from(Vec::new()) });"]
#[doc = "```"]
#[doc = "```compile_fail"]
#[doc = "let value = llmwire::ir::Opaque { kind: llmwire::ids::OpaqueKind::ProviderSpecific(\"test\"), bytes: Box::<[u8]>::from(Vec::new()) };"]
#[doc = "value.as_str();"]
#[doc = "```"]
/// 必须字节保真但不由 IR 解释的不透明内容。
///
/// 类型不提供 `Display` 或 `as_str`；`Debug` 只显示 kind 与长度。
#[derive(Clone)]
pub struct Opaque {
    /// 来源类型。
    pub kind: OpaqueKind,
    /// 原始字节。
    pub bytes: Box<[u8]>,
}

impl fmt::Debug for Opaque {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Opaque")
            .field("kind", &self.kind)
            .field("len", &self.bytes.len())
            .finish()
    }
}

/// Anthropic 明文 thinking。
#[derive(Debug, Clone)]
pub struct Thinking {
    /// thinking 文本。
    pub text: String,
    /// 可选签名。
    pub signature: Option<Opaque>,
}
