use std::time::Duration;

use futures_util::stream;
use magenta_core::{
    ChatProvider, GenerationEvent, GenerationRequest, GenerationStream, ProviderError,
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
                let mut events = response_chunks(&fake_response(&prompt))
                    .into_iter()
                    .map(|chunk| Ok(GenerationEvent::TextDelta(chunk)))
                    .collect::<Vec<_>>();
                events.push(Ok(GenerationEvent::Completed));
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

    use futures_util::StreamExt as _;
    use magenta_core::{
        ConversationId, EffortLevel, GenerationConfig, Message, MessageId, MessageRole,
        MessageStatus, ModelId, ProviderId,
    };

    use super::*;

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
        }
    }

    #[test]
    fn demo_stream_reassembles_the_response_and_completes_once() {
        let prompt = "keep the provider boundary narrow";
        let provider = DemoProvider::new(Duration::ZERO, Duration::ZERO);
        let events = smol::block_on(
            provider
                .stream(request(vec![user_message(prompt)]))
                .collect::<Vec<_>>(),
        );
        let mut response = String::new();
        let mut completions = 0;

        for event in events {
            match event.expect("the demo provider should succeed") {
                GenerationEvent::TextDelta(chunk) => response.push_str(&chunk),
                GenerationEvent::Completed => completions += 1,
            }
        }

        assert_eq!(response, fake_response(prompt));
        assert_eq!(completions, 1);
    }

    #[test]
    fn demo_stream_reports_a_typed_error_without_user_context() {
        let provider = DemoProvider::new(Duration::ZERO, Duration::ZERO);
        let events = smol::block_on(provider.stream(request(Vec::new())).collect::<Vec<_>>());

        let error = events
            .into_iter()
            .next()
            .expect("the demo provider should emit an error")
            .expect_err("an empty request should fail");
        assert_eq!(error.provider, ProviderId::new("anthropic"));
        assert_eq!(
            error.source.to_string(),
            "the generation request did not contain a user message"
        );
    }
}
