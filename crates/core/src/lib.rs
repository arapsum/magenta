//! Provider-independent conversation values shared by Magenta's UI and
//! future storage/provider adapters.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConversationId(pub u64);

impl ConversationId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MessageId(pub u64);

impl MessageId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelId(pub String);

impl ModelId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProviderId(pub String);

impl ProviderId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageStatus {
    Complete,
    Streaming,
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversation {
    pub id: ConversationId,
    pub title: String,
    pub generation: GenerationConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: MessageId,
    pub conversation_id: ConversationId,
    pub role: MessageRole,
    pub content: String,
    pub status: MessageStatus,
    pub attachments: Vec<Attachment>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_constructed_without_ui_dependencies() {
        assert_eq!(ConversationId::new(7), ConversationId(7));
        assert_eq!(MessageId::new(11), MessageId(11));
        assert_eq!(ModelId::new("sonnet").0, "sonnet");
        assert_eq!(ProviderId::new("anthropic").0, "anthropic");
    }

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
    fn message_values_preserve_role_status_and_attachments() {
        let message = Message {
            id: MessageId::new(1),
            conversation_id: ConversationId::new(2),
            role: MessageRole::User,
            content: "Show me the plan".to_owned(),
            status: MessageStatus::Complete,
            attachments: vec![Attachment {
                name: "brief.png".to_owned(),
                path: PathBuf::from("brief.png"),
            }],
        };

        assert_eq!(message.role, MessageRole::User);
        assert_eq!(message.status, MessageStatus::Complete);
        assert_eq!(message.attachments.len(), 1);
    }
}
