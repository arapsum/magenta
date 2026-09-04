use magenta_core::{
    Attachment, Conversation, ConversationId, EffortLevel, Message, MessageId, MessageRole,
    MessageStatus, ModelId,
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
                    model: model_id(model),
                    effort,
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

    pub fn create_conversation(
        &mut self,
        prompt: &str,
        model: ChatModel,
        effort: EffortLevel,
    ) -> Conversation {
        let conversation = Conversation {
            id: ConversationId::new(self.next_conversation_id),
            title: title_from_prompt(prompt),
            model: model_id(model),
            effort,
        };
        self.next_conversation_id = self.next_conversation_id.saturating_add(1);
        self.threads.push(DemoThread {
            conversation: conversation.clone(),
            messages: Vec::new(),
        });
        conversation
    }

    pub const fn new_message(
        &mut self,
        conversation_id: ConversationId,
        role: MessageRole,
        content: String,
        status: MessageStatus,
        attachments: Vec<Attachment>,
    ) -> Message {
        let message = Message {
            id: MessageId::new(self.next_message_id),
            conversation_id,
            role,
            content,
            status,
            attachments,
        };
        self.next_message_id = self.next_message_id.saturating_add(1);
        message
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
    ChatModel::from_id(conversation.model.0.as_str())
}

#[must_use]
pub fn fake_response(prompt: &str) -> String {
    let fingerprint = prompt
        .bytes()
        .fold(0usize, |sum, byte| sum.wrapping_add(usize::from(byte)));

    match fingerprint % 3 {
        0 => format!(
            "## A focused direction\n\nI would turn **{prompt}** into a small, observable workflow. Start with the user-visible state, keep the boundary narrow, and let the implementation grow from a real interaction.\n\n- Name the state the user can see.\n- Keep the first action reversible.\n- Measure the result before adding another layer.\n\nThat gives the next decision a clear place to land."
        ),
        1 => format!(
            "Here is a practical shape for **{prompt}**:\n\n1. Capture the intent in one value.\n2. Render the current state immediately.\n3. Move slow work behind a cancellable task.\n4. Persist only after the result is complete.\n\nThe important part is the seam between the UI and the work behind it."
        ),
        _ => format!(
            "I would keep **{prompt}** deliberately small at first. The UI can model the workflow with a typed operation, then a provider or storage adapter can replace the local fixture later.\n\n```rust\nstruct Operation {{\n    input: String,\n    state: OperationState,\n}}\n```\n\nThat shape keeps the interface testable while the remote boundary is still evolving."
        ),
    }
}

#[must_use]
pub fn response_chunks(response: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut last_boundary = 0;

    for (index, character) in response.char_indices() {
        let end = index + character.len_utf8();
        if character.is_whitespace() {
            last_boundary = end;
        }
        if end.saturating_sub(start) >= 20 && last_boundary > start {
            let boundary = last_boundary;
            chunks.push(response[start..boundary].to_owned());
            start = boundary;
            last_boundary = boundary;
        }
    }

    if start < response.len() {
        chunks.push(response[start..].to_owned());
    }

    chunks
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

fn model_id(model: ChatModel) -> ModelId {
    ModelId::new(model.id())
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
    fn title_from_prompt_is_single_line_and_bounded() {
        let title = title_from_prompt(
            "  A very long first line that should become a concise conversation title for the sidebar\nsecond line",
        );

        assert_eq!(title, "A very long first line that should become a...");
        assert!(title.chars().count() <= 46);
    }

    #[test]
    fn response_chunks_preserve_markdown_and_unicode() {
        let response = "A calm café with **bold** detail.";
        let chunks = response_chunks(response);

        assert!(!chunks.is_empty());
        assert_eq!(chunks.concat(), response);
    }

    #[test]
    fn fake_responses_are_deterministic_and_rich() {
        let response = fake_response("streaming responses in GPUI");

        assert_eq!(response, fake_response("streaming responses in GPUI"));
        assert!(response.contains("**streaming responses in GPUI**"));
    }
}
