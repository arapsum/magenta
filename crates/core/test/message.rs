use super::*;
use crate::AssistantTrace;

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
        assistant_trace: AssistantTrace::default(),
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
