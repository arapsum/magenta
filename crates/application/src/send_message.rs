use std::sync::Arc;

use futures_util::StreamExt as _;
use magenta_core::{
    AssistantTrace, AttachmentDraft, BeginTurn, ChatProvider, CommandCatalog, CommandId,
    Conversation, ConversationId, ConversationMode, ConversationStore, GenerationConfig,
    GenerationEvent, GenerationRequest, GenerationStream, Message, MessageId, MessageRole,
    MessageStatus,
};

use crate::trace::traced_generation_stream;
use crate::{
    CommandResolutionError, SendMessageError, TitleConversationError, apply_provider_prompt,
    resolve_normal, resolve_submission,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendTarget {
    New,
    Existing(ConversationId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendMessageInput {
    pub target: SendTarget,
    pub prompt: String,
    pub command_id: Option<CommandId>,
    pub attachments: Vec<AttachmentDraft>,
    pub generation: GenerationConfig,
    pub mode: ConversationMode,
    pub workspace_root: Option<std::path::PathBuf>,
}

pub struct PendingGeneration {
    pub conversation: Conversation,
    pub user_message: Message,
    pub assistant_message: Message,
    pub stream: GenerationStream,
    pub user_sequence: magenta_core::MessageSequence,
    pub assistant_sequence: magenta_core::MessageSequence,
    pub context_report: magenta_core::ContextBudgetReport,
}

#[derive(Clone)]
pub struct SendMessage {
    provider: Arc<dyn ChatProvider>,
    store: Arc<dyn ConversationStore>,
    command_catalog: Option<Arc<dyn CommandCatalog>>,
}

impl SendMessage {
    #[must_use]
    pub fn new(provider: Arc<dyn ChatProvider>, store: Arc<dyn ConversationStore>) -> Self {
        Self {
            provider,
            store,
            command_catalog: None,
        }
    }

    #[must_use]
    pub fn with_command_catalog(mut self, catalog: Arc<dyn CommandCatalog>) -> Self {
        self.command_catalog = Some(catalog);
        self
    }

    /// Commits the turn before starting the provider.
    ///
    /// # Errors
    /// Returns an error if the prompt is empty or persistence fails.
    pub async fn execute(
        &self,
        input: SendMessageInput,
    ) -> Result<PendingGeneration, SendMessageError> {
        let stored_prompt = input.prompt.trim().to_owned();
        let resolution = match input.command_id.as_ref() {
            Some(command_id) => {
                let catalog = self.command_catalog.as_deref().ok_or_else(|| {
                    SendMessageError::Command(CommandResolutionError::Unavailable {
                        command_id: command_id.clone(),
                    })
                })?;
                resolve_submission(
                    catalog,
                    &input.generation,
                    &input.mode,
                    Some(command_id),
                    &stored_prompt,
                    !input.attachments.is_empty(),
                )?
            }
            None => resolve_normal(&stored_prompt),
        };
        if stored_prompt.is_empty() && input.attachments.is_empty() && input.command_id.is_none() {
            return Err(SendMessageError::EmptyPrompt);
        }
        let title = title_from_prompt(&resolution.title_seed, &input.attachments);
        let prepared = self
            .store
            .begin_turn(BeginTurn {
                conversation_id: match input.target {
                    SendTarget::New => None,
                    SendTarget::Existing(id) => Some(id),
                },
                title,
                prompt: stored_prompt,
                command_id: resolution.command_id.clone(),
                attachments: input.attachments,
                generation: input.generation,
                mode: input.mode,
                workspace_root: input.workspace_root,
                request_overhead_tokens: resolution.request_overhead_tokens,
            })
            .await?;
        let stream = traced_generation_stream(
            self.store.clone(),
            prepared.assistant_message.id,
            prepared.assistant_message.assistant_trace.clone(),
            self.provider.stream(GenerationRequest {
                generation: prepared.conversation.generation.clone(),
                messages: apply_provider_prompt(prepared.context, Some(&resolution)),
                instructions: resolution.instructions,
            }),
        );
        Ok(PendingGeneration {
            conversation: prepared.conversation,
            user_message: prepared.user_message,
            assistant_message: prepared.assistant_message,
            stream,
            user_sequence: prepared.user_sequence,
            assistant_sequence: prepared.assistant_sequence,
            context_report: prepared.context_report,
        })
    }

    /// Generates and conditionally persists a concise title for a new conversation.
    ///
    /// The conditional write prevents a late model response from replacing a manual rename.
    ///
    /// # Errors
    /// Returns an error when generation fails, produces no usable title, or storage is unavailable.
    pub async fn generate_title(
        &self,
        conversation_id: ConversationId,
        opening_prompt: &str,
        current_title: String,
        generation: GenerationConfig,
    ) -> Result<Option<String>, TitleConversationError> {
        let opening_prompt = opening_prompt.chars().take(4_000).collect::<String>();
        let request = format!(
            "Name this conversation from the opening message below. Return only a concise, specific title of 3 to 7 words. Do not use quotation marks, markdown, or a trailing period.\n\nOpening message:\n{opening_prompt}"
        );
        let message = Message {
            id: MessageId(0),
            conversation_id,
            role: MessageRole::User,
            command_id: None,
            content: request,
            status: MessageStatus::Complete,
            attachments: Vec::new(),
            generation_outcome: None,
            failure: None,
            assistant_trace: AssistantTrace::default(),
        };
        let mut stream = self.provider.stream(GenerationRequest {
            generation,
            messages: vec![message],
            instructions: None,
        });
        let mut output = String::new();
        let mut completed = false;
        while let Some(event) = stream.next().await {
            match event? {
                GenerationEvent::Started
                | GenerationEvent::ReasoningSummaryStarted { .. }
                | GenerationEvent::ReasoningSummaryDelta { .. }
                | GenerationEvent::ReasoningSummaryCompleted { .. } => {}
                GenerationEvent::TextDelta(delta)
                | GenerationEvent::TextDeltaWithPhase { delta, .. } => output.push_str(&delta),
                GenerationEvent::Completed(_) => {
                    completed = true;
                    break;
                }
            }
        }
        if !completed {
            return Err(TitleConversationError::Incomplete);
        }
        let title = normalize_generated_title(&output).ok_or(TitleConversationError::Empty)?;
        let changed = self
            .store
            .rename_if_current(conversation_id, current_title, title.clone())
            .await?;
        Ok(changed.then_some(title))
    }
}

fn normalize_generated_title(value: &str) -> Option<String> {
    let mut title = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.len() >= 2 {
        let quoted = (title.starts_with('"') && title.ends_with('"'))
            || (title.starts_with('`') && title.ends_with('`'));
        if quoted {
            title.remove(0);
            title.pop();
            trim_in_place(&mut title);
        }
    }
    if title
        .get(..10)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("**title:**"))
    {
        title.drain(..10);
        trim_in_place(&mut title);
    }
    if title
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("title:"))
    {
        title.drain(..6);
        trim_in_place(&mut title);
    }
    while title.ends_with('.') {
        title.pop();
    }
    trim_in_place(&mut title);
    if title.chars().count() > 60 {
        title = format!("{}…", title.chars().take(59).collect::<String>());
    }
    (!title.is_empty()).then_some(title)
}

fn trim_in_place(value: &mut String) {
    let leading = value.len().saturating_sub(value.trim_start().len());
    value.drain(..leading);
    value.truncate(value.trim_end().len());
}

fn title_from_prompt(prompt: &str, attachments: &[AttachmentDraft]) -> String {
    let title = prompt
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        return attachments.first().map_or_else(
            || "New conversation".to_owned(),
            |attachment| format!("Image: {}", attachment.name),
        );
    }
    if title.chars().count() > 46 {
        format!("{}...", title.chars().take(43).collect::<String>())
    } else {
        title
    }
}

#[cfg(test)]
#[path = "../test/send_message.rs"]
mod tests;
