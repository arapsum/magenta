use gpui::{Context, Window};
use magenta_application::{
    AgentSendTarget, PendingAgentGeneration, PendingGeneration, RegenerateMessageInput,
    RunWorkspaceAgentInput,
};
use magenta_core::{ConversationId, Message, MessageId};

use super::{AccountState, CloseState, MainView, Operation};
use crate::{MagentaError, components::conversation::ConversationThread};

impl MainView {
    pub(super) fn submit_agent(
        &mut self,
        request: &crate::components::prompt_input::PromptRequest,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(agent) = self.agent.clone() else {
            Self::present_storage_error(
                &MagentaError::SendMessage {
                    source: magenta_application::SendMessageError::WorkspaceUnavailable,
                },
                window,
                cx,
            );
            return;
        };
        let Some(workspace_root) = request.workspace_root.clone() else {
            Self::present_storage_error(
                &MagentaError::SendMessage {
                    source: magenta_application::SendMessageError::WorkspaceUnavailable,
                },
                window,
                cx,
            );
            return;
        };

        let should_generate_title = self.active_conversation.is_none();
        let input = RunWorkspaceAgentInput {
            target: self
                .active_conversation
                .map_or(AgentSendTarget::New, AgentSendTarget::Existing),
            prompt: request.prompt.to_string(),
            generation: request.generation.clone(),
            workspace_root,
        };
        let submitted = request.clone();
        self.operation = Operation::Preparing;
        self.update_composer_availability(cx);
        self.operation_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = agent.execute(input).await;
            _ = view.update_in(window, |main, window, cx| {
                main.operation_task = None;
                main.operation = Operation::Idle;
                match result {
                    Ok(pending) => {
                        let title_details = should_generate_title.then(|| {
                            (
                                pending.conversation.id,
                                submitted.prompt.to_string(),
                                pending.conversation.title.clone(),
                                pending.conversation.generation.clone(),
                            )
                        });
                        main.composer.update(cx, |composer, cx| {
                            composer.clear_submitted(&submitted, window, cx);
                        });
                        main.start_agent_pending(pending, window, cx);
                        if let Some((id, prompt, current, generation)) = title_details {
                            main.generate_conversation_title(
                                id, prompt, current, generation, window, cx,
                            );
                        }
                    }
                    Err(source) => {
                        Self::present_storage_error(
                            &MagentaError::SendMessage { source },
                            window,
                            cx,
                        );
                        main.continue_navigation(window, cx);
                    }
                }
                main.update_composer_availability(cx);
                cx.notify();
            });
        }));
    }

    pub(super) fn start_pending(
        &mut self,
        pending: PendingGeneration,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let user_id = pending.user_message.id;
        let assistant_id = pending.assistant_message.id;
        let user_sequence = pending.user_sequence;
        let assistant_sequence = pending.assistant_sequence;
        let omitted = pending.context_report.omitted_messages;
        let id = pending.conversation.id;
        let provider = pending.conversation.generation.provider.clone();
        let conversation = pending.conversation.clone();
        if self.active_conversation.is_none() {
            self.conversation.update(cx, |view, cx| {
                view.load(
                    ConversationThread {
                        conversation: pending.conversation,
                        messages: Vec::new(),
                    },
                    cx,
                );
            });
        } else {
            self.conversation.update(cx, |view, cx| {
                view.set_generation(conversation.generation.clone(), cx);
            });
        }
        self.active_conversation = Some(id);
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.set_active(Some(id), cx));
        self.conversation.update(cx, |view, cx| {
            view.start_generation(
                pending.user_message,
                pending.assistant_message,
                provider,
                pending.stream,
                window,
                cx,
            );
            view.set_pending_metadata(
                user_id,
                user_sequence,
                assistant_id,
                assistant_sequence,
                omitted,
                cx,
            );
        });
        self.refresh_summaries(window, cx);
        if self.deferred_navigation.is_some() || self.close_requested.is_requested() {
            self.cancel_generation(cx);
        }
    }

    pub(super) fn start_agent_pending(
        &mut self,
        pending: PendingAgentGeneration,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let id = pending.conversation.id;
        let conversation = pending.conversation.clone();
        if self.active_conversation.is_none() {
            self.conversation.update(cx, |view, cx| {
                view.load(
                    ConversationThread {
                        conversation: conversation.clone(),
                        messages: Vec::new(),
                    },
                    cx,
                );
            });
        } else {
            self.conversation.update(cx, |view, cx| {
                view.set_generation(conversation.generation.clone(), cx);
            });
        }
        self.active_conversation = Some(id);
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.set_active(Some(id), cx));
        self.conversation.update(cx, |view, cx| {
            view.start_agent_generation(pending, window, cx);
        });
        self.refresh_summaries(window, cx);
        if self.deferred_navigation.is_some() || self.close_requested.is_requested() {
            self.cancel_generation(cx);
        }
    }

    pub(crate) fn regenerate(
        &mut self,
        target: MessageId,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.can_write(cx) || !matches!(self.account_state, AccountState::Connected(_)) {
            return;
        }
        let Some(id) = self.active_conversation else {
            return;
        };
        let workflow = self.regenerate_message.clone();
        self.operation = Operation::Preparing;
        self.update_composer_availability(cx);
        self.operation_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = workflow
                .execute(RegenerateMessageInput {
                    conversation_id: id,
                    target_message_id: target,
                })
                .await;
            _ = view.update_in(window, |main, window, cx| {
                main.operation_task = None;
                main.operation = Operation::Idle;
                match result {
                    Ok(pending) => {
                        let assistant_id = pending.assistant_message.id;
                        let assistant_sequence = pending.assistant_sequence;
                        let omitted = pending.context_report.omitted_messages;
                        main.conversation.update(cx, |view, cx| {
                            view.regenerate(
                                pending.target_message_id,
                                pending.assistant_message,
                                pending.provider_id,
                                pending.stream,
                                window,
                                cx,
                            );
                            view.set_pending_metadata(
                                MessageId(0),
                                magenta_core::MessageSequence(0),
                                assistant_id,
                                assistant_sequence,
                                omitted,
                                cx,
                            );
                        });
                        let navigation_pending = main.deferred_navigation.is_some();
                        let close_requested = main.close_requested.is_requested();
                        if navigation_pending || close_requested {
                            main.cancel_generation(cx);
                        }
                    }
                    Err(source) => {
                        Self::present_storage_error(
                            &MagentaError::RegenerateMessage { source },
                            window,
                            cx,
                        );
                        main.continue_navigation(window, cx);
                    }
                }
                main.update_composer_availability(cx);
            });
        }));
    }

    pub(crate) fn cancel_generation(&self, cx: &mut Context<'_, Self>) {
        self.conversation
            .update(cx, super::super::ConversationView::cancel);
    }

    pub(super) fn generate_conversation_title(
        &mut self,
        id: ConversationId,
        prompt: String,
        current_title: String,
        generation: magenta_core::GenerationConfig,
        window: &Window,
        cx: &Context<'_, Self>,
    ) {
        if prompt.trim().is_empty() {
            return;
        }
        let workflow = self.send_message.clone();
        self.title_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = workflow
                .generate_title(id, &prompt, current_title, generation)
                .await;
            _ = view.update_in(window, |main, window, cx| {
                main.title_task = None;
                match result {
                    Ok(Some(title)) => {
                        main.conversation.update(cx, |conversation, cx| {
                            conversation.rename(id, title.clone(), cx);
                        });
                        main.sidebar.update(cx, |sidebar, cx| {
                            sidebar.apply_generated_title(id, title, cx);
                        });
                        main.refresh_summaries(window, cx);
                    }
                    Ok(None) => {}
                    Err(error) => tracing::debug!(
                        error = ?error,
                        operation = "conversation.generate_title",
                        "kept fallback conversation title"
                    ),
                }
            });
        }));
    }

    pub(crate) fn save_response(
        &mut self,
        message: Message,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.composer
            .update(cx, |composer, cx| composer.set_generating(false, cx));
        self.operation = Operation::Saving;
        self.unsaved = Some(message.clone());
        self.update_composer_availability(cx);
        let history = self.history.clone();
        self.operation_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = history.finalize(message).await;
            _ = view.update_in(window, |main, window, cx| {
                main.operation_task = None;
                main.operation = Operation::Idle;
                match result {
                    Ok(()) => {
                        main.unsaved = None;
                        main.refresh_summaries(window, cx);
                        main.continue_navigation(window, cx);
                        if main.close_requested.is_requested() {
                            window.remove_window();
                        }
                    }
                    Err(source) => {
                        main.close_requested = CloseState::Open;
                        Self::present_storage_error(
                            &MagentaError::StorageWrite { source },
                            window,
                            cx,
                        );
                    }
                }
                main.update_composer_availability(cx);
                cx.notify();
            });
        }));
    }

    pub(crate) fn retry_save(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if self.operation != Operation::Idle {
            return;
        }
        if let Some(message) = self.unsaved.clone() {
            self.save_response(message, window, cx);
        }
    }

    pub(crate) fn set_pinned(
        &mut self,
        id: ConversationId,
        pinned: bool,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.can_write(cx) {
            return;
        }
        let history = self.history.clone();
        self.operation = Operation::Pinning;
        self.update_composer_availability(cx);
        self.operation_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = history.set_pinned(id, pinned).await;
            _ = view.update_in(window, |main, window, cx| {
                main.operation_task = None;
                main.operation = Operation::Idle;
                match result {
                    Ok(()) => main.refresh_summaries(window, cx),
                    Err(source) => Self::present_storage_error(
                        &MagentaError::StorageWrite { source },
                        window,
                        cx,
                    ),
                }
                main.continue_navigation(window, cx);
                main.update_composer_availability(cx);
                if main.close_requested.is_requested() {
                    window.remove_window();
                }
            });
        }));
    }

    pub(crate) fn rename_conversation(
        &mut self,
        id: ConversationId,
        title: String,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.can_write(cx) {
            return;
        }
        let history = self.history.clone();
        self.operation = Operation::Renaming;
        self.update_composer_availability(cx);
        self.operation_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = history.rename(id, title.clone()).await;
            _ = view.update_in(window, |main, window, cx| {
                main.operation_task = None;
                main.operation = Operation::Idle;
                match result {
                    Ok(()) => {
                        main.conversation.update(cx, |conversation, cx| {
                            conversation.rename(id, title, cx);
                        });
                        main.sidebar.update(cx, |sidebar, cx| {
                            sidebar.rename_succeeded(id, cx);
                        });
                        main.refresh_summaries(window, cx);
                    }
                    Err(source) => {
                        main.sidebar.update(cx, |sidebar, cx| {
                            sidebar.rename_failed(id, window, cx);
                        });
                        Self::present_storage_error(
                            &MagentaError::StorageWrite { source },
                            window,
                            cx,
                        );
                    }
                }
                main.continue_navigation(window, cx);
                main.update_composer_availability(cx);
                if main.close_requested.is_requested() {
                    window.remove_window();
                }
            });
        }));
    }

    pub(crate) fn confirm_delete_conversation(
        &mut self,
        id: ConversationId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.can_write(cx) {
            return;
        }
        let Some(title) = self.sidebar.read(cx).title_for(id) else {
            return;
        };
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        self.pending_deletion = Some(super::super::PendingDeletion {
            id,
            title,
            focus_handle,
        });
        cx.notify();
    }

    pub(crate) fn cancel_delete_conversation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.pending_deletion.take().is_some() {
            self.focus_handle.focus(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn confirm_pending_deletion(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if !self.can_write(cx) {
            return;
        }
        let Some(pending) = self.pending_deletion.take() else {
            return;
        };
        self.delete_conversation(pending.id, window, cx);
    }

    pub(super) fn delete_conversation(
        &mut self,
        id: ConversationId,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.can_write(cx) {
            return;
        }
        let history = self.history.clone();
        self.operation = Operation::Deleting;
        self.update_composer_availability(cx);
        self.operation_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = history.delete(id).await;
            _ = view.update_in(window, |main, window, cx| {
                main.operation_task = None;
                main.operation = Operation::Idle;
                match result {
                    Ok(()) => {
                        let was_active = main.active_conversation == Some(id);
                        main.sidebar
                            .update(cx, |sidebar, cx| sidebar.delete_succeeded(id, cx));
                        if was_active {
                            main.active_conversation = None;
                            main.conversation
                                .update(cx, super::super::ConversationView::clear);
                            main.sidebar
                                .update(cx, |sidebar, cx| sidebar.set_active(None, cx));
                        }
                        main.refresh_summaries(window, cx);
                    }
                    Err(source) => Self::present_storage_error(
                        &MagentaError::StorageWrite { source },
                        window,
                        cx,
                    ),
                }
                main.continue_navigation(window, cx);
                main.update_composer_availability(cx);
                if main.close_requested.is_requested() {
                    window.remove_window();
                }
                cx.notify();
            });
        }));
    }
}
