use std::time::Duration;

use futures_util::stream;
use magenta_core::{
    ChatProvider, FinishReason, GenerationEvent, GenerationOutcome, GenerationRequest,
    GenerationStream, ProviderError,
};
use smol::Timer;

use super::demo_response::{fake_response, latest_user_prompt, response_chunks};

const INITIAL_DELAY: Duration = Duration::from_millis(260);
const CHUNK_DELAY: Duration = Duration::from_millis(38);

#[derive(Clone, Copy, Debug)]
pub struct DemoProvider {
    initial_delay: Duration,
    chunk_delay: Duration,
}

impl Default for DemoProvider {
    fn default() -> Self {
        Self {
            initial_delay: INITIAL_DELAY,
            chunk_delay: CHUNK_DELAY,
        }
    }
}

impl DemoProvider {
    #[must_use]
    pub const fn new(initial_delay: Duration, chunk_delay: Duration) -> Self {
        Self {
            initial_delay,
            chunk_delay,
        }
    }
}

impl ChatProvider for DemoProvider {
    fn stream(&self, request: GenerationRequest) -> GenerationStream {
        let provider = request.generation.provider.clone();
        let events = latest_user_prompt(&request.messages).map_or_else(
            || {
                vec![Err(ProviderError::new(
                    provider,
                    DemoProviderError::MissingUserMessage,
                ))]
            },
            |prompt| {
                let mut events = vec![Ok(GenerationEvent::Started)];
                events.extend(
                    response_chunks(&fake_response(&prompt))
                        .into_iter()
                        .map(|chunk| Ok(GenerationEvent::TextDelta(chunk))),
                );
                events.push(Ok(GenerationEvent::Completed(GenerationOutcome::new(
                    FinishReason::Stop,
                    None,
                ))));
                events
            },
        );
        let initial_delay = self.initial_delay;
        let chunk_delay = self.chunk_delay;

        Box::pin(stream::unfold(
            (events.into_iter(), true),
            move |(mut events, first)| async move {
                let event = events.next()?;
                Timer::after(if first { initial_delay } else { chunk_delay }).await;
                Some((event, (events, false)))
            },
        ))
    }
}

#[derive(Debug, thiserror::Error)]
enum DemoProviderError {
    #[error("the generation request did not contain a user message")]
    MissingUserMessage,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use magenta_core::{
        ConversationId, EffortLevel, GenerationConfig, Message, MessageId, MessageRole,
        MessageStatus, ModelId, ProviderId,
    };

    use super::*;
    use crate::contract::{assert_failure_contract, assert_success_contract};

    fn request(messages: Vec<Message>) -> GenerationRequest {
        GenerationRequest {
            generation: GenerationConfig::new(
                ProviderId::new("anthropic"),
                ModelId::new("sonnet"),
                EffortLevel::Medium,
            ),
            messages,
        }
    }

    fn user_message(content: &str) -> Message {
        Message {
            id: MessageId::new(1),
            conversation_id: ConversationId::new(1),
            role: MessageRole::User,
            content: content.to_owned(),
            status: MessageStatus::Complete,
            attachments: Vec::new(),
            generation_outcome: None,
        }
    }

    #[test]
    fn demo_stream_reassembles_the_response_and_completes_once() {
        let prompt = "keep the provider boundary narrow";
        let provider = DemoProvider::new(Duration::ZERO, Duration::ZERO);
        let outcome = assert_success_contract(
            &provider,
            request(vec![user_message(prompt)]),
            &fake_response(prompt),
        );

        assert_eq!(outcome, GenerationOutcome::new(FinishReason::Stop, None));
    }

    #[test]
    fn demo_stream_reports_a_typed_error_without_user_context() {
        let provider = DemoProvider::new(Duration::ZERO, Duration::ZERO);
        let error = assert_failure_contract(&provider, request(Vec::new()));
        assert_eq!(error.provider, ProviderId::new("anthropic"));
        assert_eq!(
            error.source.to_string(),
            "the generation request did not contain a user message"
        );
    }
}
