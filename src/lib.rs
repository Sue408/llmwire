#![forbid(unsafe_code)]

pub mod caps;
pub mod codec;
pub mod converter;
pub mod error;
pub mod framing;
pub mod ids;
pub mod ir;
pub mod report;

pub use caps::{resolve, Capabilities, Mode, ParamSet, ThinkingPolicy, ToolIdPolicy};
pub use converter::{converter, Converter};
pub use error::Error;
pub use ids::{OpaqueKind, ProtocolId};
pub use ir::Termination;
pub use report::{Report, Severity, Unmapped, UnmappedReason, Warning};
