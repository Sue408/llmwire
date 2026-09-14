use std::collections::BTreeMap;

use crate::caps::{resolve, Capabilities, ParamSet, ThinkingPolicy, ToolIdPolicy};
use crate::{Error, ProtocolId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum UnknownModelPolicy {
    #[default]
    FallbackToProtocol,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelProfile {
    pub thinking: Option<ThinkingPolicy>,
    pub tool_id: Option<ToolIdPolicy>,
    pub passthrough_cache_control: Option<bool>,
    pub passthrough_betas: Option<bool>,
    pub supported: Option<ParamSet>,
    pub max_output_tokens: Option<u32>,
}

pub trait Host {
    fn capabilities(
        &self,
        inbound: ProtocolId,
        backend: ProtocolId,
        model: &str,
    ) -> Result<Capabilities, Error>;

    fn max_output_tokens(&self, model: &str) -> Option<u32>;
}

#[derive(Debug, Clone, Default)]
pub struct StaticHost {
    unknown: UnknownModelPolicy,
    models: BTreeMap<Box<str>, ModelProfile>,
}

impl StaticHost {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_unknown_policy(unknown: UnknownModelPolicy) -> Self {
        Self {
            unknown,
            models: BTreeMap::new(),
        }
    }

    pub fn insert_profile(&mut self, model: impl Into<Box<str>>, profile: ModelProfile) {
        self.models.insert(model.into(), profile);
    }

    #[must_use]
    pub fn with_profile(mut self, model: impl Into<Box<str>>, profile: ModelProfile) -> Self {
        self.insert_profile(model, profile);
        self
    }

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
