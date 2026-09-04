use std::sync::Arc;

use magenta_core::{
    ChatProvider, Conversation, GenerationRequest, GenerationStream, Message, MessageId,
    MessageRole, MessageStatus, ProviderId,
};

use crate::RegenerateMessageError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegenerateMessageInput {
    pub conversation: Conversation,
    pub messages: Vec<Message>,
    pub target_message_id: MessageId,
    pub assistant_message_id: MessageId,
}

pub struct PendingRegeneration {
    pub target_message_id: MessageId,
    pub assistant_message: Message,
    pub provider_id: ProviderId,
    pub stream: GenerationStream,
}

pub struct RegenerateMessage {
    provider: Arc<dyn ChatProvider>,
}

impl RegenerateMessage {
    #[must_use]
    pub fn new(provider: Arc<dyn ChatProvider>) -> Self {
        Self { provider }
    }

    /// Validates a completed assistant response and prepares its replacement.
    ///
    /// # Errors
    ///
    /// Returns an error when history does not belong to the conversation, the
    /// target is absent or ineligible, the replacement ID is already used, or
    /// no completed user context precedes the target.
    pub fn execute(
        &self,
        input: RegenerateMessageInput,
    ) -> Result<PendingRegeneration, RegenerateMessageError> {
        validate_history(input.conversation.id, &input.messages)?;

        if input
            .messages
            .iter()
            .any(|message| message.id == input.assistant_message_id)
        {
            return Err(RegenerateMessageError::MessageIdAlreadyUsed {
                message: input.assistant_message_id,
            });
        }

        let target_index = input
            .messages
            .iter()
            .position(|message| message.id == input.target_message_id)
            .ok_or(RegenerateMessageError::TargetNotFound {
                target: input.target_message_id,
            })?;
        let target = &input.messages[target_index];
        if target.role != MessageRole::Assistant {
            return Err(RegenerateMessageError::TargetNotAssistant {
                target: input.target_message_id,
            });
        }
        if target.status == MessageStatus::Streaming {
            return Err(RegenerateMessageError::TargetStillStreaming {
                target: input.target_message_id,
            });
        }

        let context = input.messages[..target_index]
            .iter()
            .filter(|message| message.status == MessageStatus::Complete)
            .cloned()
            .collect::<Vec<_>>();
        if !context
            .iter()
            .any(|message| message.role == MessageRole::User)
        {
            return Err(RegenerateMessageError::MissingUserContext {
                target: input.target_message_id,
            });
        }

        let provider_id = input.conversation.generation.provider.clone();
        let assistant_message = Message {
            id: input.assistant_message_id,
            conversation_id: input.conversation.id,
            role: MessageRole::Assistant,
            content: String::new(),
            status: MessageStatus::Streaming,
            attachments: Vec::new(),
            generation_outcome: None,
        };
        let stream = self.provider.stream(GenerationRequest {
            generation: input.conversation.generation,
            messages: context,
        });

        Ok(PendingRegeneration {
            target_message_id: input.target_message_id,
            assistant_message,
            provider_id,
            stream,
        })
    }
}

fn validate_history(
    conversation_id: magenta_core::ConversationId,
    history: &[Message],
) -> Result<(), RegenerateMessageError> {
    history
        .iter()
        .find(|message| message.conversation_id != conversation_id)
        .map_or(Ok(()), |message| {
            Err(RegenerateMessageError::HistoryConversationMismatch {
                expected: conversation_id,
                actual: message.conversation_id,
                message: message.id,
            })
        })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures_util::stream;
    use magenta_core::{
        ConversationId, EffortLevel, FinishReason, GenerationConfig, GenerationEvent,
        GenerationOutcome, ModelId, ProviderId,
    };

    use super::*;

    #[derive(Default)]
    struct RecordingProvider {
        requests: Mutex<Vec<GenerationRequest>>,
    }

    impl ChatProvider for RecordingProvider {
        fn stream(&self, request: GenerationRequest) -> GenerationStream {
            self.requests
                .lock()
                .expect("the provider request lock should not be poisoned")
                .push(request);
            Box::pin(stream::iter([
                Ok(GenerationEvent::Started),
                Ok(GenerationEvent::Completed(GenerationOutcome::new(
                    FinishReason::Stop,
                    None,
                ))),
            ]))
        }
    }

    fn conversation() -> Conversation {
        Conversation {
            id: ConversationId::new(7),
            title: "Focused boundary".to_owned(),
            generation: GenerationConfig::new(
                ProviderId::new("anthropic"),
                ModelId::new("sonnet"),
                EffortLevel::High,
            ),
        }
    }

    fn message(id: u64, role: MessageRole, status: MessageStatus) -> Message {
        Message {
            id: MessageId::new(id),
            conversation_id: ConversationId::new(7),
            role,
            content: format!("message {id}"),
            status,
            attachments: Vec::new(),
            generation_outcome: None,
        }
    }

    fn input(messages: Vec<Message>) -> RegenerateMessageInput {
        RegenerateMessageInput {
            conversation: conversation(),
            messages,
            target_message_id: MessageId::new(2),
            assistant_message_id: MessageId::new(9),
        }
    }

    #[test]
    fn prepares_replacement_from_complete_context_before_target() {
        let provider = Arc::new(RecordingProvider::default());
        let workflow = RegenerateMessage::new(provider.clone());
        let pending = workflow
            .execute(input(vec![
                message(1, MessageRole::User, MessageStatus::Complete),
                message(2, MessageRole::Assistant, MessageStatus::Complete),
                message(3, MessageRole::User, MessageStatus::Complete),
            ]))
            .expect("a completed assistant response should regenerate");

        assert_eq!(pending.target_message_id, MessageId::new(2));
        assert_eq!(pending.assistant_message.id, MessageId::new(9));
        assert_eq!(pending.assistant_message.status, MessageStatus::Streaming);
        assert_eq!(pending.provider_id, ProviderId::new("anthropic"));
        let requests = provider
            .requests
            .lock()
            .expect("the provider request lock should not be poisoned");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].messages.len(), 1);
        assert_eq!(requests[0].messages[0].id, MessageId::new(1));
    }

    #[test]
    fn rejects_every_invalid_target_without_invoking_provider() {
        let provider = Arc::new(RecordingProvider::default());
        let workflow = RegenerateMessage::new(provider.clone());

        let cases = [
            (
                input(vec![message(1, MessageRole::User, MessageStatus::Complete)]),
                RegenerateMessageError::TargetNotFound {
                    target: MessageId::new(2),
                },
            ),
            (
                input(vec![
                    message(1, MessageRole::User, MessageStatus::Complete),
                    message(2, MessageRole::User, MessageStatus::Complete),
                ]),
                RegenerateMessageError::TargetNotAssistant {
                    target: MessageId::new(2),
                },
            ),
            (
                input(vec![
                    message(1, MessageRole::User, MessageStatus::Complete),
                    message(2, MessageRole::Assistant, MessageStatus::Streaming),
                ]),
                RegenerateMessageError::TargetStillStreaming {
                    target: MessageId::new(2),
                },
            ),
            (
                input(vec![
                    message(1, MessageRole::User, MessageStatus::Complete),
                    message(2, MessageRole::Assistant, MessageStatus::Complete),
                    message(9, MessageRole::Assistant, MessageStatus::Complete),
                ]),
                RegenerateMessageError::MessageIdAlreadyUsed {
                    message: MessageId::new(9),
                },
            ),
            (
                input(vec![message(
                    2,
                    MessageRole::Assistant,
                    MessageStatus::Complete,
                )]),
                RegenerateMessageError::MissingUserContext {
                    target: MessageId::new(2),
                },
            ),
        ];

        for (input, expected) in cases {
            let result = workflow.execute(input);
            assert!(matches!(result, Err(ref error) if error == &expected));
        }

        let mut foreign = input(vec![
            message(1, MessageRole::User, MessageStatus::Complete),
            message(2, MessageRole::Assistant, MessageStatus::Complete),
        ]);
        foreign.messages[0].conversation_id = ConversationId::new(99);
        assert!(matches!(
            workflow.execute(foreign),
            Err(RegenerateMessageError::HistoryConversationMismatch {
                expected,
                actual,
                message,
            }) if expected == ConversationId::new(7)
                && actual == ConversationId::new(99)
                && message == MessageId::new(1)
        ));

        assert!(
            provider
                .requests
                .lock()
                .expect("the provider request lock should not be poisoned")
                .is_empty()
        );
    }
}
