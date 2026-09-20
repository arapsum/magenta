use super::*;
use crate::{AssistantTrace, ConversationId, MessageId, MessageStatus};

fn message(id: u64, role: MessageRole, content: &str) -> Message {
    Message {
        id: MessageId(id),
        conversation_id: ConversationId(1),
        role,
        command_id: None,
        content: content.to_owned(),
        status: MessageStatus::Complete,
        attachments: Vec::new(),
        generation_outcome: None,
        failure: None,
        assistant_trace: AssistantTrace::default(),
    }
}

#[test]
fn retains_recent_complete_turns_without_orphaning_assistants() {
    let messages = vec![
        message(1, MessageRole::User, &"a".repeat(120)),
        message(2, MessageRole::Assistant, &"b".repeat(120)),
        message(3, MessageRole::User, "latest"),
    ];
    let (selected, report) = select_context(
        &messages,
        GenerationLimits {
            context_window_tokens: 350,
            max_output_tokens: 20,
        },
        0,
    )
    .expect("latest turn fits");
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].content, "latest");
    assert_eq!(report.omitted_messages, 2);
}

#[test]
fn rejects_a_newest_turn_that_cannot_fit() {
    let result = select_context(
        &[message(1, MessageRole::User, &"x".repeat(300))],
        GenerationLimits {
            context_window_tokens: 300,
            max_output_tokens: 20,
        },
        0,
    );
    assert_eq!(result, Err(ContextTooLarge));
}
