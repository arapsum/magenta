use std::sync::Arc;

use magenta_core::{
    Attachment, ChatProvider, Conversation, ConversationId, GenerationConfig, GenerationRequest,
    GenerationStream, Message, MessageId, MessageRole, MessageStatus,
};

use crate::SendMessageError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendTarget {
    New { conversation_id: ConversationId },
    Existing(Conversation),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageIds {
    pub user: MessageId,
    pub assistant: MessageId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendMessageInput {
    pub target: SendTarget,
    pub history: Vec<Message>,
    pub ids: MessageIds,
    pub prompt: String,
    pub attachments: Vec<Attachment>,
    pub generation: GenerationConfig,
}

pub struct PendingGeneration {
    pub conversation: Conversation,
    pub user_message: Message,
    pub assistant_message: Message,
    pub stream: GenerationStream,
}

pub struct SendMessage {
    provider: Arc<dyn ChatProvider>,
}

impl SendMessage {
    #[must_use]
    pub fn new(provider: Arc<dyn ChatProvider>) -> Self {
        Self { provider }
    }

    /// Prepares the user and assistant messages and starts provider streaming.
    ///
    /// # Errors
    ///
    /// Returns an error when the prompt is empty, reserved IDs collide with
    /// history, or history belongs to another conversation.
    pub fn execute(&self, input: SendMessageInput) -> Result<PendingGeneration, SendMessageError> {
        let prompt = input.prompt.trim().to_owned();
        if prompt.is_empty() {
            return Err(SendMessageError::EmptyPrompt);
        }

        validate_message_ids(&input.history, input.ids)?;
        let conversation = conversation_for(input.target, prompt.as_str(), input.generation);
        validate_history(conversation.id, &input.history)?;

        let user_message = Message {
            id: input.ids.user,
            conversation_id: conversation.id,
            role: MessageRole::User,
            content: prompt,
            status: MessageStatus::Complete,
            attachments: input.attachments,
            generation_outcome: None,
        };
        let assistant_message = Message {
            id: input.ids.assistant,
            conversation_id: conversation.id,
            role: MessageRole::Assistant,
            content: String::new(),
            status: MessageStatus::Streaming,
            attachments: Vec::new(),
            generation_outcome: None,
        };

        let messages = input
            .history
            .into_iter()
            .filter(|message| message.status == MessageStatus::Complete)
            .chain(std::iter::once(user_message.clone()))
            .collect();
        let request = GenerationRequest {
            generation: conversation.generation.clone(),
            messages,
        };

        Ok(PendingGeneration {
            conversation,
            user_message,
            assistant_message,
            stream: self.provider.stream(request),
        })
    }
}

fn conversation_for(
    target: SendTarget,
    prompt: &str,
    generation: GenerationConfig,
) -> Conversation {
    match target {
        SendTarget::New { conversation_id } => Conversation {
            id: conversation_id,
            title: title_from_prompt(prompt),
            generation,
        },
        SendTarget::Existing(mut conversation) => {
            conversation.generation = generation;
            conversation
        }
    }
}

fn validate_message_ids(history: &[Message], ids: MessageIds) -> Result<(), SendMessageError> {
    if ids.user == ids.assistant {
        return Err(SendMessageError::DuplicateMessageIds);
    }

    if history
        .iter()
        .any(|message| message.id == ids.user || message.id == ids.assistant)
    {
        return Err(SendMessageError::MessageIdAlreadyUsed);
    }

    Ok(())
}

fn validate_history(
    conversation_id: ConversationId,
    history: &[Message],
) -> Result<(), SendMessageError> {
    history
        .iter()
        .find(|message| message.conversation_id != conversation_id)
        .map_or(Ok(()), |message| {
            Err(SendMessageError::HistoryConversationMismatch {
                expected: conversation_id,
                actual: message.conversation_id,
                message: message.id,
            })
        })
}

fn title_from_prompt(prompt: &str) -> String {
    let title = prompt
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut title = title;
    if title.chars().count() > 46 {
        title = title.chars().take(43).collect();
        title.push_str("...");
    }
    if title.is_empty() {
        "New conversation".to_owned()
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use futures_util::stream;
    use magenta_core::{
        EffortLevel, FinishReason, GenerationEvent, GenerationOutcome, GenerationStream, ModelId,
        ProviderId,
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

    fn generation() -> GenerationConfig {
        GenerationConfig::new(
            ProviderId::new("anthropic"),
            ModelId::new("sonnet"),
            EffortLevel::Medium,
        )
    }

    fn message(
        id: u64,
        conversation_id: ConversationId,
        role: MessageRole,
        content: &str,
        status: MessageStatus,
    ) -> Message {
        Message {
            id: MessageId::new(id),
            conversation_id,
            role,
            content: content.to_owned(),
            status,
            attachments: Vec::new(),
            generation_outcome: None,
        }
    }

    fn input(
        target: SendTarget,
        history: Vec<Message>,
        prompt: &str,
        ids: MessageIds,
    ) -> SendMessageInput {
        SendMessageInput {
            target,
            history,
            ids,
            prompt: prompt.to_owned(),
            attachments: Vec::new(),
            generation: generation(),
        }
    }

    #[test]
    fn new_conversation_prepares_messages_and_filters_incomplete_history() {
        let provider = Arc::new(RecordingProvider::default());
        let workflow = SendMessage::new(provider.clone());
        let conversation_id = ConversationId::new(7);
        let pending = workflow
            .execute(input(
                SendTarget::New { conversation_id },
                vec![
                    message(
                        1,
                        conversation_id,
                        MessageRole::User,
                        "first",
                        MessageStatus::Complete,
                    ),
                    message(
                        2,
                        conversation_id,
                        MessageRole::Assistant,
                        "answer",
                        MessageStatus::Complete,
                    ),
                    message(
                        3,
                        conversation_id,
                        MessageRole::Assistant,
                        "stopped",
                        MessageStatus::Stopped,
                    ),
                ],
                "  second prompt  ",
                MessageIds {
                    user: MessageId::new(10),
                    assistant: MessageId::new(11),
                },
            ))
            .expect("the send workflow should prepare a valid request");

        assert_eq!(pending.conversation.title, "second prompt");
        assert_eq!(pending.user_message.content, "second prompt");
        assert_eq!(pending.user_message.status, MessageStatus::Complete);
        assert_eq!(pending.assistant_message.status, MessageStatus::Streaming);

        let requests = provider
            .requests
            .lock()
            .expect("the provider request lock should not be poisoned");
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .messages
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            vec![MessageId::new(1), MessageId::new(2), MessageId::new(10)]
        );
    }

    #[test]
    fn existing_conversation_uses_the_submitted_generation_configuration() {
        let provider = Arc::new(RecordingProvider::default());
        let workflow = SendMessage::new(provider);
        let conversation_id = ConversationId::new(8);
        let mut submitted_generation = generation();
        submitted_generation.effort = EffortLevel::High;
        let pending = workflow
            .execute(SendMessageInput {
                target: SendTarget::Existing(Conversation {
                    id: conversation_id,
                    title: "Existing".to_owned(),
                    generation: generation(),
                }),
                history: vec![message(
                    1,
                    conversation_id,
                    MessageRole::User,
                    "first",
                    MessageStatus::Complete,
                )],
                ids: MessageIds {
                    user: MessageId::new(10),
                    assistant: MessageId::new(11),
                },
                prompt: "follow up".to_owned(),
                attachments: Vec::new(),
                generation: submitted_generation.clone(),
            })
            .expect("the existing conversation should be accepted");

        assert_eq!(pending.conversation.generation, submitted_generation);
    }

    #[test]
    fn invalid_inputs_fail_before_the_provider_is_called() {
        let provider = Arc::new(RecordingProvider::default());
        let workflow = SendMessage::new(provider.clone());
        let conversation_id = ConversationId::new(9);
        let target = SendTarget::Existing(Conversation {
            id: conversation_id,
            title: "Existing".to_owned(),
            generation: generation(),
        });

        assert!(matches!(
            workflow.execute(input(
                target.clone(),
                Vec::new(),
                "   ",
                MessageIds {
                    user: MessageId::new(10),
                    assistant: MessageId::new(11),
                },
            )),
            Err(SendMessageError::EmptyPrompt)
        ));
        assert!(matches!(
            workflow.execute(input(
                target.clone(),
                Vec::new(),
                "valid",
                MessageIds {
                    user: MessageId::new(10),
                    assistant: MessageId::new(10),
                },
            )),
            Err(SendMessageError::DuplicateMessageIds)
        ));
        let result = workflow.execute(input(
            target,
            vec![message(
                1,
                ConversationId::new(99),
                MessageRole::User,
                "foreign",
                MessageStatus::Complete,
            )],
            "valid",
            MessageIds {
                user: MessageId::new(10),
                assistant: MessageId::new(11),
            },
        ));
        let Err(SendMessageError::HistoryConversationMismatch {
            expected,
            actual,
            message,
        }) = result
        else {
            panic!("the foreign history should be rejected");
        };
        assert_eq!(expected, conversation_id);
        assert_eq!(actual, ConversationId::new(99));
        assert_eq!(message, MessageId::new(1));

        assert!(
            provider
                .requests
                .lock()
                .expect("the provider request lock should not be poisoned")
                .is_empty()
        );
    }
}
