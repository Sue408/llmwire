#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cached: u64,
    pub cache_creation: u64,
    pub reasoning: u64,
}
