use gpui_kit::{Context, Window};
use magenta_application::{
    AgentSendTarget, PendingAgentGeneration, PendingGeneration, RegenerateMessageInput,
    RetryMessageInput, RetryWorkspaceAgentInput, RunWorkspaceAgentInput,
};
use magenta_core::{ConversationId, GenerationConfig, Message, MessageId};

use super::{AccountState, MainView, Operation};
use crate::{
    MagentaError,
    components::{agent_workbench::AgentWorkbench, prompt_input::PromptComposer},
};

impl MainView {
    pub(super) fn submit_agent(
        &mut self,
        request: &crate::components::prompt_input::PromptRequest,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(agent) = self.agent.clone() else {
            self.present_composer_error(
                &MagentaError::SendMessage {
                    source: magenta_application::SendMessageError::WorkspaceUnavailable,
                },
                cx,
            );
            return;
        };
        let Some(workspace_root) = request.workspace_root.clone() else {
            self.present_composer_error(
                &MagentaError::SendMessage {
                    source: magenta_application::SendMessageError::WorkspaceUnavailable,
                },
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
            command_id: request.command_id.clone(),
            generation: request.generation.clone(),
            workspace_root,
        };
        let submitted = request.clone();

        if let Some(workbench) = &self.workbench {
            workbench.update(cx, AgentWorkbench::begin_agent_run);
        }

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
                        main.present_composer_error(&MagentaError::SendMessage { source }, cx);
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
        let id = pending.conversation.id;
        let conversation = pending.conversation.clone();
        let show_run = self.active_conversation == Some(id)
            || (self.active_conversation.is_none() && self.deferred_navigation.is_none());
        let assistant_id = self.response_runs.start_chat(pending, window, cx);
        if show_run {
            self.active_conversation = Some(id);
            self.sidebar
                .update(cx, |sidebar, cx| sidebar.set_active(Some(id), cx));
            self.composer.update(cx, |composer, cx| {
                composer.set_conversation_context(
                    conversation.mode.clone(),
                    conversation.workspace_root.clone(),
                    cx,
                );
                composer.set_configuration(&conversation.generation, cx);
            });
            self.sync_settings_setup(cx);
            self.sync_run(assistant_id, cx);
        }
        self.operation = Operation::Idle;
        self.update_composer_availability(cx);
        self.refresh_summaries(window, cx);
        self.continue_navigation(window, cx);
    }

    pub(super) fn start_agent_pending(
        &mut self,
        pending: PendingAgentGeneration,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let id = pending.conversation.id;
        let conversation = pending.conversation.clone();
        let show_run = self.active_conversation == Some(id)
            || (self.active_conversation.is_none() && self.deferred_navigation.is_none());
        let assistant_id = self.response_runs.start_agent(pending, window, cx);
        if show_run {
            self.active_conversation = Some(id);
            self.sidebar
                .update(cx, |sidebar, cx| sidebar.set_active(Some(id), cx));
            self.composer.update(cx, |composer, cx| {
                composer.set_conversation_context(
                    conversation.mode.clone(),
                    conversation.workspace_root.clone(),
                    cx,
                );
                composer.set_configuration(&conversation.generation, cx);
            });
            self.sync_settings_setup(cx);
            self.sync_run(assistant_id, cx);
        }
        self.operation = Operation::Idle;
        self.update_composer_availability(cx);
        self.refresh_summaries(window, cx);
        self.continue_navigation(window, cx);
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
        if self.conversation.read(cx).is_agent_conversation() {
            self.regenerate_agent_response(target, id, window, cx);
            return;
        }
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
                        let Some(conversation) = main.conversation.read(cx).conversation_details()
                        else {
                            return;
                        };
                        let assistant_id = main.response_runs.start_regeneration(
                            pending,
                            conversation,
                            window,
                            cx,
                        );
                        main.sync_run(assistant_id, cx);
                        main.continue_navigation(window, cx);
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

    fn regenerate_agent_response(
        &mut self,
        target: MessageId,
        id: ConversationId,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(agent) = self.agent.clone() else {
            return;
        };
        self.operation = Operation::Preparing;
        self.update_composer_availability(cx);
        self.operation_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = agent
                .regenerate(magenta_application::RegenerateMessageInput {
                    conversation_id: id,
                    target_message_id: target,
                })
                .await;
            _ = view.update_in(window, |main, window, cx| {
                main.operation_task = None;
                main.operation = Operation::Idle;
                match result {
                    Ok(pending) => {
                        let assistant_id =
                            main.response_runs.start_agent_retry(pending, window, cx);
                        main.sync_run(assistant_id, cx);
                        main.continue_navigation(window, cx);
                    }
                    Err(source) => {
                        Self::present_storage_error(
                            &MagentaError::RetryMessage { source },
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

    pub(crate) fn retry_response(
        &mut self,
        target: MessageId,
        generation_override: Option<GenerationConfig>,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.can_write(cx) || !matches!(self.account_state, AccountState::Connected(_)) {
            return;
        }
        let Some(id) = self.active_conversation else {
            return;
        };
        if self.conversation.read(cx).is_agent_conversation() {
            self.retry_agent_response(target, generation_override, id, window, cx);
            return;
        }
        let generation = generation_override
            .or_else(|| self.conversation.read(cx).generation_for_message(target))
            .or_else(|| self.conversation.read(cx).conversation_generation());
        let Some(generation) = generation else {
            return;
        };
        let workflow = self.regenerate_message.clone();
        self.operation = Operation::Preparing;
        self.update_composer_availability(cx);
        self.operation_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = workflow
                .retry(RetryMessageInput {
                    conversation_id: id,
                    target_message_id: target,
                    generation,
                })
                .await;
            _ = view.update_in(window, |main, window, cx| {
                main.operation_task = None;
                main.operation = Operation::Idle;
                match result {
                    Ok(pending) => {
                        main.composer.update(cx, PromptComposer::clear_model_retry);
                        let assistant_id = main.response_runs.start_retry(pending, window, cx);
                        main.sync_run(assistant_id, cx);
                        main.continue_navigation(window, cx);
                    }
                    Err(source) => {
                        Self::present_storage_error(
                            &MagentaError::RetryMessage { source },
                            window,
                            cx,
                        );
                        main.retry_target = Some(target);
                    }
                }
                main.update_composer_availability(cx);
            });
        }));
    }

    fn retry_agent_response(
        &mut self,
        target: MessageId,
        generation_override: Option<GenerationConfig>,
        id: ConversationId,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(agent) = self.agent.clone() else {
            return;
        };
        let generation = generation_override
            .or_else(|| self.conversation.read(cx).generation_for_message(target))
            .or_else(|| self.conversation.read(cx).conversation_generation());
        let Some(generation) = generation else {
            return;
        };
        self.operation = Operation::Preparing;
        self.update_composer_availability(cx);
        self.operation_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = agent
                .retry(RetryWorkspaceAgentInput {
                    conversation_id: id,
                    target_message_id: target,
                    generation,
                })
                .await;
            _ = view.update_in(window, |main, window, cx| {
                main.operation_task = None;
                main.operation = Operation::Idle;
                match result {
                    Ok(pending) => {
                        main.composer.update(cx, PromptComposer::clear_model_retry);
                        let assistant_id =
                            main.response_runs.start_agent_retry(pending, window, cx);
                        main.sync_run(assistant_id, cx);
                        main.continue_navigation(window, cx);
                    }
                    Err(source) => {
                        Self::present_storage_error(
                            &MagentaError::RetryMessage { source },
                            window,
                            cx,
                        );
                        main.retry_target = Some(target);
                    }
                }
                main.update_composer_availability(cx);
            });
        }));
    }

    pub(crate) fn prepare_continuation(
        &mut self,
        target: MessageId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let prompt = "Continue from the completed work above. Do not repeat operations that already succeeded; inspect the current state first.";
        self.retry_target = None;
        self.composer.update(cx, |composer, cx| {
            composer.prepare_continuation(prompt, window, cx);
        });
        tracing::info!(
            message_id = target.0,
            operation = "conversation.prepare_continuation",
            "prepared a manual continuation after a failed agent response"
        );
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
        let Some(conversation) = self.conversation.read(cx).conversation_details() else {
            return;
        };
        let message_id = message.id;
        self.response_runs.insert_terminal(message, conversation);
        self.composer
            .update(cx, |composer, cx| composer.set_generating(false, cx));
        self.sync_run(message_id, cx);
        self.retry_unsaved_run(window, cx);
    }

    pub(crate) fn retry_save(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        self.retry_unsaved_run(window, cx);
    }
}
