use std::pin::Pin;

use futures_core::Stream;

use super::{
    error::ProviderError,
    identifiers::{ModelId, ProviderId},
    message::Message,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffortLevel {
    Low,
    Medium,
    High,
}

impl EffortLevel {
    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationConfig {
    pub provider: ProviderId,
    pub model: ModelId,
    pub effort: EffortLevel,
}

impl GenerationConfig {
    #[must_use]
    pub const fn new(provider: ProviderId, model: ModelId, effort: EffortLevel) -> Self {
        Self {
            provider,
            model,
            effort,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationRequest {
    pub generation: GenerationConfig,
    pub messages: Vec<Message>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenerationEvent {
    TextDelta(String),
    Completed,
}

pub type GenerationStream =
    Pin<Box<dyn Stream<Item = Result<GenerationEvent, ProviderError>> + Send + 'static>>;

pub trait ChatProvider: Send + Sync {
    fn stream(&self, request: GenerationRequest) -> GenerationStream;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_configuration_keeps_provider_model_and_effort_together() {
        let configuration = GenerationConfig::new(
            ProviderId::new("anthropic"),
            ModelId::new("sonnet"),
            EffortLevel::High,
        );

        assert_eq!(configuration.provider, ProviderId::new("anthropic"));
        assert_eq!(configuration.model, ModelId::new("sonnet"));
        assert_eq!(configuration.effort, EffortLevel::High);
    }
}
