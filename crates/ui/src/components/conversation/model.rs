use magenta_application::MessageIds;
use magenta_core::{
    Conversation, ConversationId, EffortLevel, Message, MessageId, MessageRole, MessageStatus,
};

use crate::components::{
    prompt_input::ChatModel,
    sidebar::{ConversationPeriod, ConversationSummary},
};

#[derive(Clone, Debug)]
pub struct DemoThread {
    pub conversation: Conversation,
    pub messages: Vec<Message>,
}

#[derive(Debug)]
pub struct DemoCatalog {
    threads: Vec<DemoThread>,
    next_conversation_id: u64,
    next_message_id: u64,
}

impl DemoCatalog {
    #[must_use]
    pub fn new() -> Self {
        let summaries = crate::components::sidebar::demo_conversations();
        let mut next_message_id = 1_000;
        let threads = summaries
            .into_iter()
            .enumerate()
            .map(|(index, summary)| {
                let model = demo_model(index);
                let effort = demo_effort(index);
                let conversation = Conversation {
                    id: summary.id,
                    title: summary.title,
                    generation: model.generation_config(effort),
                };
                let messages = fixture_messages(
                    conversation.id,
                    conversation.title.as_str(),
                    index,
                    &mut next_message_id,
                );
                DemoThread {
                    conversation,
                    messages,
                }
            })
            .collect();

        Self {
            threads,
            next_conversation_id: 100,
            next_message_id,
        }
    }

    #[must_use]
    pub fn thread(&self, id: ConversationId) -> Option<&DemoThread> {
        self.threads
            .iter()
            .find(|thread| thread.conversation.id == id)
    }

    pub const fn reserve_conversation_id(&mut self) -> ConversationId {
        let id = ConversationId::new(self.next_conversation_id);
        self.next_conversation_id = self.next_conversation_id.saturating_add(1);
        id
    }

    pub const fn reserve_message_ids(&mut self) -> MessageIds {
        let ids = MessageIds {
            user: MessageId::new(self.next_message_id),
            assistant: MessageId::new(self.next_message_id.saturating_add(1)),
        };
        self.next_message_id = self.next_message_id.saturating_add(2);
        ids
    }

    pub const fn reserve_message_id(&mut self) -> MessageId {
        let id = MessageId::new(self.next_message_id);
        self.next_message_id = self.next_message_id.saturating_add(1);
        id
    }

    pub fn replace_thread(&mut self, thread: DemoThread) {
        if let Some(existing) = self
            .threads
            .iter_mut()
            .find(|existing| existing.conversation.id == thread.conversation.id)
        {
            *existing = thread;
        } else {
            self.threads.push(thread);
        }
    }
}

impl Default for DemoCatalog {
    fn default() -> Self {
        Self::new()
    }
}

#[must_use]
pub fn summary_for(conversation: &Conversation) -> ConversationSummary {
    ConversationSummary {
        id: conversation.id,
        title: conversation.title.clone(),
        period: ConversationPeriod::Today,
        pinned: false,
    }
}

#[must_use]
pub fn chat_model_for(conversation: &Conversation) -> ChatModel {
    ChatModel::from_generation(&conversation.generation)
}

fn fixture_messages(
    conversation_id: ConversationId,
    title: &str,
    index: usize,
    next_message_id: &mut u64,
) -> Vec<Message> {
    let mut messages = Vec::with_capacity(4);
    push_fixture_message(
        &mut messages,
        conversation_id,
        next_message_id,
        MessageRole::User,
        format!("Can you help me explore **{title}**?"),
    );
    push_fixture_message(
        &mut messages,
        conversation_id,
        next_message_id,
        MessageRole::Assistant,
        fixture_response(title, index),
    );
    push_fixture_message(
        &mut messages,
        conversation_id,
        next_message_id,
        MessageRole::User,
        "What would you make concrete first?".to_owned(),
    );
    push_fixture_message(
        &mut messages,
        conversation_id,
        next_message_id,
        MessageRole::Assistant,
        "I would make the smallest useful path visible first, then leave a clean seam for the next capability. The demo thread is intentionally local, so this is a safe place to explore the interaction.".to_owned(),
    );
    messages
}

fn push_fixture_message(
    messages: &mut Vec<Message>,
    conversation_id: ConversationId,
    next_message_id: &mut u64,
    role: MessageRole,
    content: String,
) {
    messages.push(Message {
        id: MessageId::new(*next_message_id),
        conversation_id,
        role,
        content,
        status: MessageStatus::Complete,
        attachments: Vec::new(),
        generation_outcome: None,
    });
    *next_message_id = (*next_message_id).saturating_add(1);
}

fn fixture_response(title: &str, index: usize) -> String {
    match index % 3 {
        0 => format!(
            "## A measured approach\n\nFor **{title}**, I would keep the first pass deliberately observable:\n\n- start with one user action;\n- show the result in the same surface;\n- keep the next step easy to undo.\n\nThat gives us a useful baseline before we optimize the path."
        ),
        1 => format!(
            "The useful boundary around **{title}** is the state transition, not the visual wrapper. I would model it like this:\n\n```rust\nlet result = operation.run(input).await?;\nview.apply(result, cx);\n```\n\nThe UI remains easy to test, and the implementation can change behind the operation later."
        ),
        _ => format!(
            "For **{title}**, I would separate the work into three quiet stages:\n\n1. **Intent** — what the user asked for.\n2. **Progress** — what Magenta is doing now.\n3. **Result** — what can be revisited or copied.\n\nThat rhythm keeps a complex workflow legible without making the conversation feel like a dashboard."
        ),
    }
}

const fn demo_model(index: usize) -> ChatModel {
    match index % 3 {
        0 => ChatModel::Sonnet,
        1 => ChatModel::Gpt,
        _ => ChatModel::GeminiPro,
    }
}

const fn demo_effort(index: usize) -> EffortLevel {
    match index % 3 {
        0 => EffortLevel::High,
        1 => EffortLevel::Medium,
        _ => EffortLevel::Low,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_a_thread_for_every_sidebar_summary() {
        let catalog = DemoCatalog::new();
        let summaries = crate::components::sidebar::demo_conversations();

        assert_eq!(catalog.threads.len(), summaries.len());
        assert!(summaries.iter().all(|summary| {
            catalog
                .thread(summary.id)
                .is_some_and(|thread| !thread.messages.is_empty())
        }));
    }

    #[test]
    fn reserved_ids_advance_without_reuse() {
        let mut catalog = DemoCatalog::new();
        let first_conversation = catalog.reserve_conversation_id();
        let second_conversation = catalog.reserve_conversation_id();
        let first_messages = catalog.reserve_message_ids();
        let replacement_message = catalog.reserve_message_id();
        let second_messages = catalog.reserve_message_ids();

        assert_eq!(first_conversation, ConversationId::new(100));
        assert_eq!(second_conversation, ConversationId::new(101));
        assert_eq!(first_messages.user, MessageId::new(1_040));
        assert_eq!(first_messages.assistant, MessageId::new(1_041));
        assert_eq!(replacement_message, MessageId::new(1_042));
        assert_eq!(second_messages.user, MessageId::new(1_043));
        assert_eq!(second_messages.assistant, MessageId::new(1_044));
    }
}
