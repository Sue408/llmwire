//! 路由能力与参数策略。
//!
//! 能力描述的是“客户端协议到后端协议”这条路由允许采用的转换策略，
//! 不是模型能力清单本身。
use bitflags::bitflags;

use crate::ids::ProtocolId;

/// 转换模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Mode {
    /// 源协议与目标协议一致，按原生透传路径处理。
    NativePassthrough,
    /// 在源协议与目标协议之间做语义转换。
    #[default]
    Converted,
    /// 无法无损表达时立即失败，而不是静默降级。
    Strict,
}

/// thinking 与签名内容的处理策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ThinkingPolicy {
    /// 原样保留。
    Passthrough,
    /// 在目标协议中转换为等价的 thinking 表达。
    Adapt,
    /// 目标不支持时显式剥离并上报。
    Strip,
    /// 目标不支持时直接拒绝。
    Reject,
}

/// 工具调用 ID 的处理策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ToolIdPolicy {
    /// 不修改、不截断、不重新生成。
    #[default]
    Preserve,
}

bitflags! {
    /// 目标路由支持的采样与请求参数集合。
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct ParamSet: u32 {
        const TEMPERATURE = 1 << 0;
        const TOP_P = 1 << 1;
        const TOP_K = 1 << 2;
        const MAX_OUTPUT_TOKENS = 1 << 3;
        const STOP = 1 << 4;
        const SEED = 1 << 5;
        const N = 1 << 6;
        const PRESENCE_PENALTY = 1 << 7;
        const FREQUENCY_PENALTY = 1 << 8;
        const REASONING = 1 << 9;
        const TOOLS = 1 << 10;
        const TOOL_CHOICE = 1 << 11;
        const CACHE_CONTROL = 1 << 12;
        const BETAS = 1 << 13;
    }
}

/// 一条协议路由允许采用的转换策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// 转换模式。
    pub mode: Mode,
    /// thinking 与签名内容策略。
    pub thinking: ThinkingPolicy,
    /// 工具调用 ID 策略。
    pub tool_id: ToolIdPolicy,
    /// 是否允许透传 `cache_control`。
    pub passthrough_cache_control: bool,
    /// 是否允许透传 Anthropic beta 头对应的请求字段。
    pub passthrough_betas: bool,
    /// 目标路由支持的参数集合。
    pub supported: ParamSet,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            mode: Mode::Converted,
            thinking: ThinkingPolicy::Strip,
            tool_id: ToolIdPolicy::Preserve,
            passthrough_cache_control: false,
            passthrough_betas: false,
            supported: supported_params(ProtocolId::Chat),
        }
    }
}

impl Capabilities {
    /// 判断当前路由是否支持指定的参数集合。
    #[must_use]
    pub fn supports(self, params: ParamSet) -> bool {
        self.supported.contains(params)
    }
}

/// 根据入站协议与后端协议生成默认能力策略。
///
/// 相同协议得到 [`Mode::NativePassthrough`]，跨协议得到 [`Mode::Converted`]。
/// 当前 `model` 参数保留给 host 侧扩展；需要按模型覆盖时使用 [`crate::StaticHost`]。
#[must_use]
pub fn resolve(inbound: ProtocolId, backend: ProtocolId, _model: &str) -> Capabilities {
    let mode = if inbound == backend {
        Mode::NativePassthrough
    } else {
        Mode::Converted
    };

    Capabilities {
        mode,
        thinking: thinking_policy(inbound, backend),
        tool_id: ToolIdPolicy::Preserve,
        passthrough_cache_control: backend == ProtocolId::Messages,
        passthrough_betas: backend == ProtocolId::Messages,
        supported: supported_params(backend),
    }
}

fn thinking_policy(inbound: ProtocolId, backend: ProtocolId) -> ThinkingPolicy {
    if inbound == backend {
        return ThinkingPolicy::Passthrough;
    }

    match backend {
        ProtocolId::Messages | ProtocolId::Responses => ThinkingPolicy::Passthrough,
        ProtocolId::Chat => ThinkingPolicy::Strip,
    }
}

fn supported_params(protocol: ProtocolId) -> ParamSet {
    match protocol {
        ProtocolId::Chat => {
            ParamSet::TEMPERATURE
                | ParamSet::TOP_P
                | ParamSet::MAX_OUTPUT_TOKENS
                | ParamSet::STOP
                | ParamSet::SEED
                | ParamSet::N
                | ParamSet::PRESENCE_PENALTY
                | ParamSet::FREQUENCY_PENALTY
                | ParamSet::REASONING
                | ParamSet::TOOLS
                | ParamSet::TOOL_CHOICE
        }
        ProtocolId::Messages => {
            ParamSet::TEMPERATURE
                | ParamSet::TOP_P
                | ParamSet::TOP_K
                | ParamSet::MAX_OUTPUT_TOKENS
                | ParamSet::STOP
                | ParamSet::REASONING
                | ParamSet::TOOLS
                | ParamSet::TOOL_CHOICE
                | ParamSet::CACHE_CONTROL
                | ParamSet::BETAS
        }
        ProtocolId::Responses => {
            ParamSet::TEMPERATURE
                | ParamSet::TOP_P
                | ParamSet::MAX_OUTPUT_TOKENS
                | ParamSet::REASONING
                | ParamSet::TOOLS
                | ParamSet::TOOL_CHOICE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROTOCOLS: [ProtocolId; 3] = [
        ProtocolId::Chat,
        ProtocolId::Messages,
        ProtocolId::Responses,
    ];

    #[test]
    fn resolve_covers_all_protocol_directions() {
        for inbound in PROTOCOLS {
            for backend in PROTOCOLS {
                let caps = resolve(inbound, backend, "test-model");
                assert_eq!(
                    caps.mode,
                    if inbound == backend {
                        Mode::NativePassthrough
                    } else {
                        Mode::Converted
                    }
                );
                assert_eq!(caps.tool_id, ToolIdPolicy::Preserve);
            }
        }
    }

    #[test]
    fn resolve_uses_target_thinking_and_cache_control_capabilities() {
        assert_eq!(
            resolve(ProtocolId::Chat, ProtocolId::Messages, "m").thinking,
            ThinkingPolicy::Passthrough
        );
        assert_eq!(
            resolve(ProtocolId::Messages, ProtocolId::Responses, "m").thinking,
            ThinkingPolicy::Passthrough
        );
        assert_eq!(
            resolve(ProtocolId::Responses, ProtocolId::Chat, "m").thinking,
            ThinkingPolicy::Strip
        );

        let messages = resolve(ProtocolId::Chat, ProtocolId::Messages, "m");
        assert!(messages.passthrough_cache_control);
        assert!(messages.passthrough_betas);

        let chat = resolve(ProtocolId::Messages, ProtocolId::Chat, "m");
        assert!(!chat.passthrough_cache_control);
        assert!(!chat.passthrough_betas);
    }

    #[test]
    fn resolve_param_sets_follow_target_protocol() {
        let chat = resolve(ProtocolId::Messages, ProtocolId::Chat, "m");
        assert!(chat.supports(ParamSet::N));
        assert!(!chat.supports(ParamSet::TOP_K));

        let messages = resolve(ProtocolId::Chat, ProtocolId::Messages, "m");
        assert!(messages.supports(ParamSet::TOP_K));
        assert!(!messages.supports(ParamSet::N));

        let responses = resolve(ProtocolId::Messages, ProtocolId::Responses, "m");
        assert!(responses.supports(ParamSet::REASONING));
        assert!(!responses.supports(ParamSet::STOP));
    }
}
