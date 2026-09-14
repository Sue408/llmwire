#[derive(Debug, Clone, Default)]
pub struct Sampling {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub stop: Vec<Box<str>>,
    pub seed: Option<i64>,
    pub n: Option<u32>,
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
    None,
}

#[derive(Debug, Clone, Default)]
pub struct Reasoning {
    pub enabled: bool,
    pub effort: Option<ReasoningEffort>,
    pub budget_tokens: Option<u64>,
}
