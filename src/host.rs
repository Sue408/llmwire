//! Host 侧模型与能力配置端口。
use std::collections::BTreeMap;

use crate::caps::{resolve, Capabilities, ParamSet, ThinkingPolicy, ToolIdPolicy};
use crate::{Error, ProtocolId};

/// 未配置模型时采用的能力策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum UnknownModelPolicy {
    /// 回退到协议级默认能力。
    #[default]
    FallbackToProtocol,
    /// 返回 [`Error::Unsupported`]。
    Reject,
}

/// 单个模型的能力覆盖项。
///
/// 未填写的字段回退到协议级默认值，`supported` 会与默认集合取交集。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelProfile {
    /// thinking 与签名内容策略。
    pub thinking: Option<ThinkingPolicy>,
    /// 工具调用 ID 策略。
    pub tool_id: Option<ToolIdPolicy>,
    /// 是否允许透传 `cache_control`。
    pub passthrough_cache_control: Option<bool>,
    /// 是否允许透传 beta 字段。
    pub passthrough_betas: Option<bool>,
    /// 模型支持的参数集合。
    pub supported: Option<ParamSet>,
    /// 模型输出 token 上限。
    pub max_output_tokens: Option<u32>,
}

/// 为转换器提供路由能力与模型限制。
pub trait Host {
    /// 返回指定入站协议、后端协议与模型组合的能力策略。
    fn capabilities(
        &self,
        inbound: ProtocolId,
        backend: ProtocolId,
        model: &str,
    ) -> Result<Capabilities, Error>;

    /// 返回模型的输出 token 上限。
    fn max_output_tokens(&self, model: &str) -> Option<u32>;
}

/// 由固定模型表实现的 [`Host`]。
#[derive(Debug, Clone, Default)]
pub struct StaticHost {
    unknown: UnknownModelPolicy,
    models: BTreeMap<Box<str>, ModelProfile>,
}

impl StaticHost {
    /// 创建空的 host，未知模型默认回退到协议能力。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 创建空 host，并指定未知模型策略。
    #[must_use]
    pub fn with_unknown_policy(unknown: UnknownModelPolicy) -> Self {
        Self {
            unknown,
            models: BTreeMap::new(),
        }
    }

    /// 插入或替换一个模型配置。
    pub fn insert_profile(&mut self, model: impl Into<Box<str>>, profile: ModelProfile) {
        self.models.insert(model.into(), profile);
    }

    /// 以 builder 风格插入一个模型配置。
    #[must_use]
    pub fn with_profile(mut self, model: impl Into<Box<str>>, profile: ModelProfile) -> Self {
        self.insert_profile(model, profile);
        self
    }

    /// 查询模型配置。
    #[must_use]
    pub fn profile(&self, model: &str) -> Option<&ModelProfile> {
        self.models.get(model)
    }
}

impl Host for StaticHost {
    fn capabilities(
        &self,
        inbound: ProtocolId,
        backend: ProtocolId,
        model: &str,
    ) -> Result<Capabilities, Error> {
        let default = resolve(inbound, backend, model);
        let Some(profile) = self.models.get(model) else {
            return match self.unknown {
                UnknownModelPolicy::FallbackToProtocol => Ok(default),
                UnknownModelPolicy::Reject => {
                    Err(Error::Unsupported(format!("unknown model {model}")))
                }
            };
        };

        Ok(Capabilities {
            thinking: profile.thinking.unwrap_or(default.thinking),
            tool_id: profile.tool_id.unwrap_or(default.tool_id),
            passthrough_cache_control: profile
                .passthrough_cache_control
                .unwrap_or(default.passthrough_cache_control),
            passthrough_betas: profile
                .passthrough_betas
                .unwrap_or(default.passthrough_betas),
            supported: profile
                .supported
                .map_or(default.supported, |supported| supported & default.supported),
            ..default
        })
    }

    fn max_output_tokens(&self, model: &str) -> Option<u32> {
        self.models
            .get(model)
            .and_then(|profile| profile.max_output_tokens)
    }
}
