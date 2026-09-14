use std::fmt;

use crate::ids::OpaqueKind;

use super::{ToolResult, ToolUse};

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Part {
    Text(String),
    Image(ImageRef),
    ToolUse(ToolUse),
    ToolResult(ToolResult),
    Thinking(Thinking),
    Opaque(Opaque),
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ImageRef {
    Url(Box<str>),
    Base64 {
        media_type: Box<str>,
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
#[derive(Clone)]
pub struct Opaque {
    pub kind: OpaqueKind,
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

#[derive(Debug, Clone)]
pub struct Thinking {
    pub text: String,
    pub signature: Option<Opaque>,
}
