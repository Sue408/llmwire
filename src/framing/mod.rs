//! 传输帧的字节级分帧接口。
mod sse;

pub use sse::{encode_frame, SseFrame, SseFramer, DEFAULT_MAX_BUFFER_BYTES};
