use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{
    AgentActivity, ConversationId, GenerationOutcome, MessageId, ProviderError,
    ProviderErrorDiagnostic, ProviderErrorKind, ProviderId,
};

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
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentDraft {
    pub name: String,
    pub source_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub path: PathBuf,
    pub mime_type: String,
    pub byte_size: u64,
    pub managed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MessageFailureCategory {
    Authentication,
    Permission,
    RateLimit,
    Connection,
    Service,
    InvalidRequest,
    Context,
    IncompleteResponse,
    AgentLimit,
    RepeatedToolCalls,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MessageFailureDetail {
    HttpStatus {
        status: u16,
    },
    AgentLimits {
        observed_rounds: usize,
        permitted_rounds: usize,
        observed_tool_calls: usize,
        permitted_tool_calls: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageFailure {
    pub category: MessageFailureCategory,
    pub reference_code: String,
    pub provider: ProviderId,
    pub detail: Option<MessageFailureDetail>,
}

impl MessageFailure {
    #[must_use]
    pub fn from_provider_error(error: &ProviderError) -> Self {
        let category = match error.kind {
            ProviderErrorKind::AuthenticationRequired => MessageFailureCategory::Authentication,
            ProviderErrorKind::PermissionDenied => MessageFailureCategory::Permission,
            ProviderErrorKind::RateLimited => MessageFailureCategory::RateLimit,
            ProviderErrorKind::Transport => MessageFailureCategory::Connection,
            ProviderErrorKind::ServiceUnavailable => MessageFailureCategory::Service,
            ProviderErrorKind::InvalidRequest => MessageFailureCategory::InvalidRequest,
            ProviderErrorKind::AgentLimitReached => MessageFailureCategory::AgentLimit,
            ProviderErrorKind::RepeatedToolCalls => MessageFailureCategory::RepeatedToolCalls,
            ProviderErrorKind::IncompleteResponse => MessageFailureCategory::IncompleteResponse,
            ProviderErrorKind::Protocol | ProviderErrorKind::Other => {
                MessageFailureCategory::Unknown
            }
        };
        let reference_code = match category {
            MessageFailureCategory::Authentication => "MAG-GEN-AUTH",
            MessageFailureCategory::Permission => "MAG-GEN-PERMISSION",
            MessageFailureCategory::RateLimit => "MAG-GEN-RATE-LIMIT",
            MessageFailureCategory::Connection => "MAG-GEN-CONNECTION",
            MessageFailureCategory::Service => "MAG-GEN-SERVICE",
            MessageFailureCategory::InvalidRequest => "MAG-GEN-INVALID-REQUEST",
            MessageFailureCategory::Context => "MAG-GEN-CONTEXT",
            MessageFailureCategory::IncompleteResponse => "MAG-GEN-INCOMPLETE",
            MessageFailureCategory::AgentLimit => "MAG-GEN-AGENT-LIMIT",
            MessageFailureCategory::RepeatedToolCalls => "MAG-GEN-REPEATED-ACTIONS",
            MessageFailureCategory::Unknown => "MAG-GEN-UNKNOWN",
        };
        let detail = error.diagnostic.map(|diagnostic| match diagnostic {
            ProviderErrorDiagnostic::HttpStatus(status) => {
                MessageFailureDetail::HttpStatus { status }
            }
            ProviderErrorDiagnostic::AgentLimits {
                observed_rounds,
                permitted_rounds,
                observed_tool_calls,
                permitted_tool_calls,
            } => MessageFailureDetail::AgentLimits {
                observed_rounds,
                permitted_rounds,
                observed_tool_calls,
                permitted_tool_calls,
            },
        });
        Self {
            category,
            reference_code: reference_code.to_owned(),
            provider: error.provider.clone(),
            detail,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: MessageId,
    pub conversation_id: ConversationId,
    pub role: MessageRole,
    pub content: String,
    pub status: MessageStatus,
    pub attachments: Vec<Attachment>,
    pub generation_outcome: Option<GenerationOutcome>,
    pub failure: Option<MessageFailure>,
    pub agent_activities: Vec<AgentActivity>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
                mime_type: "image/png".to_owned(),
                byte_size: 42,
                managed: true,
            }],
            generation_outcome: None,
            failure: None,
            agent_activities: Vec::new(),
        };

        assert_eq!(message.role, MessageRole::User);
        assert_eq!(message.status, MessageStatus::Complete);
        assert_eq!(message.attachments.len(), 1);
    }

    #[test]
    fn failures_keep_only_allowlisted_provider_diagnostics() {
        let error = ProviderError::with_kind_and_diagnostic(
            ProviderId::new("openai"),
            ProviderErrorKind::AgentLimitReached,
            ProviderErrorDiagnostic::AgentLimits {
                observed_rounds: 65,
                permitted_rounds: 64,
                observed_tool_calls: 257,
                permitted_tool_calls: 256,
            },
            std::io::Error::other("response body contains token=secret and a prompt"),
        );

        let failure = MessageFailure::from_provider_error(&error);
        assert_eq!(failure.reference_code, "MAG-GEN-AGENT-LIMIT");
        assert_eq!(failure.provider, ProviderId::new("openai"));
        assert_eq!(
            failure.detail,
            Some(MessageFailureDetail::AgentLimits {
                observed_rounds: 65,
                permitted_rounds: 64,
                observed_tool_calls: 257,
                permitted_tool_calls: 256,
            })
        );
        let persisted = serde_json::to_string(&failure).unwrap();
        assert!(!persisted.contains("token=secret"));
        assert!(!persisted.contains("response body"));
    }
}
