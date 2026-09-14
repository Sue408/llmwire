//! `llmwire` 是一个双向、默认无状态、可嵌入任意 host 的 LLM wire protocol 转换核。
//!
//! `Converter` 接收客户端或上游的原始字节，完成分帧、协议解析、IR 转换与目标协议序列化。
//! 核心不负责 HTTP、认证、重试、连接管理或 async runtime。
//!
//! # 快速开始
//!
//! ```no_run
//! use llmwire::{converter, resolve, ProtocolId};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut converter = converter(
//!     ProtocolId::Chat,
//!     ProtocolId::Messages,
//!     resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-model"),
//! )?;
//!
//! let client_request = br#"{
//!     "model": "claude-model",
//!     "messages": [{"role": "user", "content": "hello"}],
//!     "max_completion_tokens": 32
//! }"#;
//!
//! let mut upstream_request = Vec::new();
//! converter.request(client_request, &mut upstream_request)?;
//!
//! // Host 将 upstream_request 发给 Messages 后端，再把响应体交给 response。
//! // let backend_response = host::post(&upstream_request)?;
//! // let mut client_response = Vec::new();
//! // converter.response(&backend_response, &mut client_response)?;
//! # Ok(())
//! # }
//! ```
//!
//! 非流式调用顺序为 `request`、`response`、`take_report`。流式调用顺序为
//! `request`、重复 `feed`、`finish`、`take_report`。
//!
//! 完整的接入说明、限制与示例见仓库根目录的 `README.md`。
#![forbid(unsafe_code)]

pub mod caps;
pub mod codec;
pub mod converter;
pub mod error;
pub mod framing;
pub mod host;
pub mod ids;
pub mod ir;
pub mod report;

pub use caps::{resolve, Capabilities, Mode, ParamSet, ThinkingPolicy, ToolIdPolicy};
pub use converter::{converter, Converter};
pub use error::Error;
pub use host::{Host, ModelProfile, StaticHost, UnknownModelPolicy};
pub use ids::{OpaqueKind, ProtocolId};
pub use ir::Termination;
pub use report::{Report, Severity, Unmapped, UnmappedReason, Warning};
