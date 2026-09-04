use std::path::PathBuf;

use super::{ConversationId, GenerationOutcome, MessageId};

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
pub struct Attachment {
    pub name: String,
    pub path: PathBuf,
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
            }],
            generation_outcome: None,
        };

        assert_eq!(message.role, MessageRole::User);
        assert_eq!(message.status, MessageStatus::Complete);
        assert_eq!(message.attachments.len(), 1);
    }
}
