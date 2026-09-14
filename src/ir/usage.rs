//! token 用量。
/// token 用量；输入和输出未知时保持 `None`，不能伪造为零。
#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    /// 输入 token。
    pub input: Option<u64>,
    /// 输出 token。
    pub output: Option<u64>,
    /// 缓存命中 token。
    pub cached: u64,
    /// 缓存创建 token。
    pub cache_creation: u64,
    /// reasoning token。
    pub reasoning: u64,
}
