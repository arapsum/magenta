use std::time::Duration;

use magenta_core::{
    ConversationId, EffortLevel, GenerationConfig, Message, MessageId, MessageRole, MessageStatus,
    ModelId, ProviderId,
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
        instructions: None,
    }
}

fn user_message(content: &str) -> Message {
    Message {
        id: MessageId::new(1),
        conversation_id: ConversationId::new(1),
        role: MessageRole::User,
        command_id: None,
        content: content.to_owned(),
        status: MessageStatus::Complete,
        attachments: Vec::new(),
        generation_outcome: None,
        failure: None,
        assistant_trace: Default::default(),
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
