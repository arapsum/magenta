use gpui_kit::{Context, FollowMode, Window, px};
use magenta_core::{
    AssistantTraceStatus, GenerationStream, Message, MessageId, MessageStatus, ProviderId,
};

use super::super::{
    ConversationContext, ConversationView, GenerationProgress, trace_generation_terminal,
};

#[allow(dead_code)]
impl ConversationView {
    pub(crate) fn start_generation(
        &mut self,
        user_message: Message,
        assistant_message: Message,
        provider_id: ProviderId,
        stream: GenerationStream,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.cancel_generation(cx);
        let user = Self::rendered_message(user_message, cx);
        if let Some(conversation) = &self.conversation {
            self.origins
                .insert(assistant_message.id, conversation.generation.clone());
        }
        let assistant = Self::rendered_message(assistant_message, cx);
        let old_count = self.messages.len();
        let assistant_id = assistant.message.id;

        self.messages.push(user);
        self.messages.push(assistant);
        if self.trim_oldest_to_limit(cx) == 0 {
            self.list_state.splice(old_count..old_count, 2);
        } else {
            self.list_state
                .reset_with_uniform_height(self.messages.len(), px(96.));
        }
        self.list_state.set_follow_mode(FollowMode::Tail);
        self.list_state.scroll_to_end();
        self.begin_stream(assistant_id, provider_id, stream, window, cx);
        cx.notify();
    }

    pub(crate) fn regenerate(
        &mut self,
        assistant_id: MessageId,
        assistant_message: Message,
        provider_id: ProviderId,
        stream: GenerationStream,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(index) = self
            .messages
            .iter()
            .position(|message| message.message.id == assistant_id)
        else {
            return;
        };

        self.cancel_generation(cx);
        if let Some(conversation) = &self.conversation {
            self.origins
                .insert(assistant_message.id, conversation.generation.clone());
        }
        let new_assistant_id = assistant_message.id;
        self.messages[index] = Self::rendered_message(assistant_message, cx);
        self.list_state.remeasure_items(index..index + 1);
        self.list_state.set_follow_mode(FollowMode::Tail);
        self.list_state.scroll_to_end();
        self.begin_stream(new_assistant_id, provider_id, stream, window, cx);
        cx.notify();
    }

    pub(crate) fn start_retry(
        &mut self,
        pending: magenta_application::PendingRetry,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let magenta_application::PendingRetry {
            conversation,
            assistant_message,
            provider_id,
            stream,
            assistant_sequence,
            context_report,
        } = pending;

        self.cancel_generation(cx);
        self.conversation = Some(conversation.clone());
        self.origins
            .insert(assistant_message.id, conversation.generation);
        let assistant = Self::rendered_message(assistant_message, cx);
        let old_count = self.messages.len();
        let assistant_id = assistant.message.id;

        self.messages.push(assistant);
        self.set_pending_metadata(
            MessageId(0),
            magenta_core::MessageSequence(0),
            assistant_id,
            assistant_sequence,
            context_report.omitted_messages,
            cx,
        );
        if self.trim_oldest_to_limit(cx) == 0 {
            self.list_state.splice(old_count..old_count, 1);
        } else {
            self.list_state
                .reset_with_uniform_height(self.messages.len(), px(96.));
        }
        self.list_state.set_follow_mode(FollowMode::Tail);
        self.list_state.scroll_to_end();
        self.begin_stream(assistant_id, provider_id, stream, window, cx);
        cx.notify();
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<'_, Self>) {
        self.cancel_generation(cx);
    }

    pub(crate) fn interrupt_for_shutdown(
        &mut self,
        cx: &mut ConversationContext<'_>,
    ) -> Option<Message> {
        let id = self.streaming_message.take()?;
        self.generation = self.generation.wrapping_add(1);
        self.generation_task.take();
        self.clear_agent_state();
        let progress = self.clear_generation_progress();
        self.mark_trace_terminal(
            id,
            AssistantTraceStatus::Stopped,
            progress.as_ref().map(GenerationProgress::elapsed),
        );
        if let Some(progress) = progress.as_ref() {
            trace_generation_terminal(progress, "interrupted");
        }
        let message = self
            .messages
            .iter_mut()
            .find(|message| message.message.id == id)?;
        message.message.status = MessageStatus::Stopped;
        cx.notify();
        Some(message.message.clone())
    }

    pub(crate) fn set_pending_metadata(
        &mut self,
        user_id: MessageId,
        user_sequence: magenta_core::MessageSequence,
        assistant_id: MessageId,
        assistant_sequence: magenta_core::MessageSequence,
        omitted_context_messages: usize,
        cx: &mut Context<'_, Self>,
    ) {
        for rendered in &mut self.messages {
            if rendered.message.id == user_id {
                rendered.sequence = Some(user_sequence);
            } else if rendered.message.id == assistant_id {
                rendered.sequence = Some(assistant_sequence);
                rendered.omitted_context_messages = omitted_context_messages;
            }
        }
        self.newer_cursor = Some(assistant_sequence);
        cx.notify();
    }
}
