use std::pin::Pin;

use futures_core::Stream;

use super::{
    error::ProviderError,
    identifiers::{ModelId, ProviderId},
    message::Message,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffortLevel {
    None,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
    Custom { value: String, label: String },
}

impl EffortLevel {
    pub const ALL: [Self; 5] = [Self::Low, Self::Medium, Self::High, Self::XHigh, Self::Max];

    #[must_use]
    pub fn from_wire(value: &str) -> Option<Self> {
        let value = value.trim().to_ascii_lowercase();
        if value.is_empty() {
            return None;
        }

        Some(match value.as_str() {
            "none" => Self::None,
            "minimal" => Self::Minimal,
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "xhigh" => Self::XHigh,
            "max" => Self::Max,
            _ => Self::Custom {
                label: display_label(&value),
                value,
            },
        })
    }

    #[must_use]
    pub const fn wire_value(&self) -> &str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
            Self::Custom { value, .. } => value.as_str(),
        }
    }

    #[must_use]
    pub const fn label(&self) -> &str {
        match self {
            Self::None => "None",
            Self::Minimal => "Minimal",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::XHigh => "XHigh",
            Self::Max => "Max",
            Self::Custom { label, .. } => label.as_str(),
        }
    }
}

fn display_label(value: &str) -> String {
    value
        .split(['_', '-', ' '])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters.next().map_or_else(String::new, |first| {
                format!("{}{}", first.to_ascii_uppercase(), characters.as_str())
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
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
    Started,
    TextDelta(String),
    Completed(GenerationOutcome),
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
    fn effort_levels_preserve_provider_specific_wire_values() {
        let values = [
            "none",
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh",
            "max",
            "thinking_budget",
        ];
        let efforts = values
            .iter()
            .map(|value| EffortLevel::from_wire(value).expect("effort should be valid"))
            .collect::<Vec<_>>();

        assert_eq!(efforts[0], EffortLevel::None);
        assert_eq!(efforts[1], EffortLevel::Minimal);
        assert_eq!(efforts[5], EffortLevel::XHigh);
        assert_eq!(efforts[6].label(), "Max");
        assert_eq!(efforts[7].wire_value(), "thinking_budget");
        assert_eq!(efforts[7].label(), "Thinking Budget");
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
