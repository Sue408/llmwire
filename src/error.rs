//! 转换核公开错误。
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// 当前协议或策略不支持该操作。
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// 输入不是合法的协议或数据格式。
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// 输入格式合法，但违反协议状态机或字段语义。
    #[error("protocol error: {0}")]
    Protocol(String),
    /// SSE 等分帧缓冲区超过配置上限。
    #[error("buffer limit exceeded")]
    BufferLimitExceeded,
}
