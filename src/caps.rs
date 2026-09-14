use bitflags::bitflags;

use crate::ids::ProtocolId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Mode {
    NativePassthrough,
    #[default]
    Converted,
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ThinkingPolicy {
    Passthrough,
    Adapt,
    Strip,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ToolIdPolicy {
    #[default]
    Preserve,
}

bitflags! {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub mode: Mode,
    pub thinking: ThinkingPolicy,
    pub tool_id: ToolIdPolicy,
    pub passthrough_cache_control: bool,
    pub passthrough_betas: bool,
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
    #[must_use]
    pub fn supports(self, params: ParamSet) -> bool {
        self.supported.contains(params)
    }
}

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
