//! 采样与推理参数。
/// 与具体协议无关的采样参数；缺失值保持 `None`。
#[derive(Debug, Clone, Default)]
pub struct Sampling {
    /// temperature。
    pub temperature: Option<f32>,
    /// nucleus sampling。
    pub top_p: Option<f32>,
    /// top-k。
    pub top_k: Option<u32>,
    /// 输出 token 上限。
    pub max_output_tokens: Option<u32>,
    /// 停止序列。
    pub stop: Vec<Box<str>>,
    /// 随机种子。
    pub seed: Option<i64>,
    /// 候选数量。
    pub n: Option<u32>,
    /// presence penalty。
    pub presence_penalty: Option<f32>,
    /// frequency penalty。
    pub frequency_penalty: Option<f32>,
}

/// 推理强度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReasoningEffort {
    /// 最小推理。
    Minimal,
    /// 低强度。
    Low,
    /// 中强度。
    Medium,
    /// 高强度。
    High,
    /// 显式关闭。
    None,
}

/// 推理控制参数。
#[derive(Debug, Clone, Default)]
pub struct Reasoning {
    /// 是否启用推理。
    pub enabled: bool,
    /// 推理强度。
    pub effort: Option<ReasoningEffort>,
    /// thinking token 预算。
    pub budget_tokens: Option<u64>,
}
