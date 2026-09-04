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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationOutcome {
    pub finish_reason: FinishReason,
    pub usage: Option<TokenUsage>,
}

impl GenerationOutcome {
    #[must_use]
    pub const fn new(finish_reason: FinishReason, usage: Option<TokenUsage>) -> Self {
        Self {
            finish_reason,
            usage,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolUse,
    Other(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// A provider-owned stream whose work must stop when the stream is dropped.
///
/// GPUI views rely on this contract to cancel generation by dropping the task
/// that owns the stream.
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

    #[test]
    fn generation_outcome_keeps_finish_reason_and_usage_together() {
        let outcome = GenerationOutcome::new(
            FinishReason::Length,
            Some(TokenUsage {
                input_tokens: 21,
                output_tokens: 34,
            }),
        );

        assert_eq!(outcome.finish_reason, FinishReason::Length);
        assert_eq!(
            outcome.usage,
            Some(TokenUsage {
                input_tokens: 21,
                output_tokens: 34,
            })
        );
    }
}
