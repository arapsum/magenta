use std::pin::Pin;

use futures_core::Stream;
use serde::{Deserialize, Serialize};

use super::{
    AssistantTextPhase,
    error::ProviderError,
    identifiers::{ModelId, ProviderId},
    message::Message,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationConfig {
    pub provider: ProviderId,
    pub model: ModelId,
    pub effort: EffortLevel,
    #[serde(default)]
    pub limits: GenerationLimits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationLimits {
    pub context_window_tokens: u64,
    pub max_output_tokens: u64,
}

impl Default for GenerationLimits {
    fn default() -> Self {
        Self {
            context_window_tokens: 128_000,
            max_output_tokens: 16_384,
        }
    }
}

impl GenerationConfig {
    #[must_use]
    pub const fn new(provider: ProviderId, model: ModelId, effort: EffortLevel) -> Self {
        Self {
            provider,
            model,
            effort,
            limits: GenerationLimits {
                context_window_tokens: 128_000,
                max_output_tokens: 16_384,
            },
        }
    }

    #[must_use]
    pub const fn with_limits(mut self, limits: GenerationLimits) -> Self {
        self.limits = limits;
        self
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
    TextDeltaWithPhase {
        delta: String,
        phase: AssistantTextPhase,
    },
    ReasoningSummaryStarted {
        key: String,
        title: String,
    },
    ReasoningSummaryDelta {
        key: String,
        delta: String,
    },
    ReasoningSummaryCompleted {
        key: String,
        text: String,
    },
    Completed(GenerationOutcome),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolUse,
    Other(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[path = "../test/generation.rs"]
mod tests;
