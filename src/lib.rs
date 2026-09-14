#![forbid(unsafe_code)]

pub mod error;
pub mod ids;
pub mod ir;

pub use error::Error;
pub use ids::{OpaqueKind, ProtocolId};
