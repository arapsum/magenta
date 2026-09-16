mod events;

use std::collections::HashMap;

use futures_util::StreamExt as _;
use gpui_kit::{Context, Window};
use magenta_application::{
    PendingAgentGeneration, PendingGeneration, PendingRegeneration, PendingRetry,
};
use magenta_core::{
    AgentRunEvent, GenerationEvent, GenerationOutcome, GenerationStream, Message, MessageId,
    ProviderError, ProviderErrorKind,
};

use super::{ResponseRun, ResponseRunCoordinator, RunSaveState, now};
use crate::components::conversation::LiveRunMessage;

#[derive(Debug)]
pub enum RunEventResult {
    Continue,
    Completed(GenerationOutcome),
    WorkspaceChange(magenta_core::AgentWorkspaceChange),
    WorkspaceInvalidated,
}

impl ResponseRunCoordinator {
    pub(crate) fn start_chat(
        &mut self,
        pending: PendingGeneration,
        window: &Window,
        cx: &Context<'_, super::super::MainView>,
    ) -> MessageId {
        let PendingGeneration {
            conversation,
            user_message,
            assistant_message,
            stream,
            user_sequence,
            assistant_sequence,
            context_report,
        } = pending;

        let assistant_id = assistant_message.id;
        let generation = conversation.generation.clone();

        self.insert_run(ResponseRun {
            progress: crate::components::conversation::GenerationProgress::new(
                assistant_id,
                conversation.generation.provider.clone(),
                Some(conversation.generation.clone()),
            ),
            conversation: conversation.clone(),
            user: Some(live_message(
                user_message,
                Some(user_sequence),
                None,
                0,
                generation.clone(),
            )),
            assistant: live_message(
                assistant_message,
                Some(assistant_sequence),
                None,
                context_report.omitted_messages,
                generation,
            ),
            streaming: true,
            controller: None,
            pending_approval: None,
            live_commands: HashMap::new(),
            save_state: RunSaveState::Pending,
        });

        self.spawn_generation(
            assistant_id,
            conversation.generation.provider,
            stream,
            window,
            cx,
        );

        assistant_id
    }

    pub(crate) fn start_regeneration(
        &mut self,
        pending: PendingRegeneration,
        conversation: magenta_core::Conversation,
        window: &Window,
        cx: &Context<'_, super::super::MainView>,
    ) -> MessageId {
        let PendingRegeneration {
            target_message_id,
            assistant_message,
            provider_id,
            stream,
            assistant_sequence,
            context_report,
        } = pending;

        let assistant_id = assistant_message.id;
        let generation = conversation.generation.clone();

        self.insert_run(ResponseRun {
            progress: crate::components::conversation::GenerationProgress::new(
                assistant_id,
                provider_id.clone(),
                Some(conversation.generation.clone()),
            ),
            conversation,
            user: None,
            assistant: live_message(
                assistant_message,
                Some(assistant_sequence),
                Some(target_message_id),
                context_report.omitted_messages,
                generation,
            ),
            streaming: true,
            controller: None,
            pending_approval: None,
            live_commands: HashMap::new(),
            save_state: RunSaveState::Pending,
        });

        self.spawn_generation(assistant_id, provider_id, stream, window, cx);

        assistant_id
    }

    pub(crate) fn start_retry(
        &mut self,
        pending: PendingRetry,
        window: &Window,
        cx: &Context<'_, super::super::MainView>,
    ) -> MessageId {
        let PendingRetry {
            conversation,
            assistant_message,
            provider_id,
            stream,
            assistant_sequence,
            context_report,
        } = pending;

        let assistant_id = assistant_message.id;
        let generation = conversation.generation.clone();

        self.insert_run(ResponseRun {
            progress: crate::components::conversation::GenerationProgress::new(
                assistant_id,
                provider_id.clone(),
                Some(conversation.generation.clone()),
            ),
            conversation,
            user: None,
            assistant: live_message(
                assistant_message,
                Some(assistant_sequence),
                None,
                context_report.omitted_messages,
                generation,
            ),
            streaming: true,
            controller: None,
            pending_approval: None,
            live_commands: HashMap::new(),
            save_state: RunSaveState::Pending,
        });

        self.spawn_generation(assistant_id, provider_id, stream, window, cx);

        assistant_id
    }

    pub(crate) fn start_agent(
        &mut self,
        pending: PendingAgentGeneration,
        window: &Window,
        cx: &Context<'_, super::super::MainView>,
    ) -> MessageId {
        let PendingAgentGeneration {
            conversation,
            user_message,
            assistant_message,
            stream,
            controller,
            user_sequence,
            assistant_sequence,
            context_report,
        } = pending;

        let assistant_id = assistant_message.id;
        let generation = conversation.generation.clone();
        let provider_id = conversation.generation.provider.clone();

        self.insert_run(ResponseRun {
            progress: crate::components::conversation::GenerationProgress::new(
                assistant_id,
                provider_id.clone(),
                Some(conversation.generation.clone()),
            ),
            conversation,
            user: Some(live_message(
                user_message,
                Some(user_sequence),
                None,
                0,
                generation.clone(),
            )),
            assistant: live_message(
                assistant_message,
                Some(assistant_sequence),
                None,
                context_report.omitted_messages,
                generation,
            ),
            streaming: true,
            controller: Some(controller),
            pending_approval: None,
            live_commands: HashMap::new(),
            save_state: RunSaveState::Pending,
        });

        self.spawn_agent(assistant_id, provider_id, stream, window, cx);

        assistant_id
    }

    pub(crate) fn start_agent_retry(
        &mut self,
        pending: PendingAgentGeneration,
        window: &Window,
        cx: &Context<'_, super::super::MainView>,
    ) -> MessageId {
        let PendingAgentGeneration {
            conversation,
            assistant_message,
            stream,
            controller,
            assistant_sequence,
            context_report,
            ..
        } = pending;

        let assistant_id = assistant_message.id;
        let generation = conversation.generation.clone();
        let provider_id = conversation.generation.provider.clone();

        self.insert_run(ResponseRun {
            progress: crate::components::conversation::GenerationProgress::new(
                assistant_id,
                provider_id.clone(),
                Some(conversation.generation.clone()),
            ),
            conversation,
            user: None,
            assistant: live_message(
                assistant_message,
                Some(assistant_sequence),
                None,
                context_report.omitted_messages,
                generation,
            ),
            streaming: true,
            controller: Some(controller),
            pending_approval: None,
            live_commands: HashMap::new(),
            save_state: RunSaveState::Pending,
        });

        self.spawn_agent(assistant_id, provider_id, stream, window, cx);

        assistant_id
    }

    fn insert_run(&mut self, run: ResponseRun) {
        let message_id = run.assistant_id();
        self.active_by_conversation
            .insert(run.conversation.id, message_id);
        self.runs.insert(message_id, run);
    }

    fn spawn_generation(
        &mut self,
        assistant_id: MessageId,
        provider_id: magenta_core::ProviderId,
        stream: GenerationStream,
        window: &Window,
        cx: &Context<'_, super::super::MainView>,
    ) {
        let task = cx.spawn_in(window, async move |view, window| {
            let mut stream = stream;

            while let Some(event) = stream.next().await {
                match event {
                    Ok(event) => {
                        let Ok(continue_stream) = view.update_in(window, |main, window, cx| {
                            main.handle_generation_event(assistant_id, event, window, cx)
                        }) else {
                            return;
                        };

                        if !continue_stream {
                            return;
                        }
                    }
                    Err(error) => {
                        _ = view.update_in(window, |main, window, cx| {
                            main.fail_response_run(assistant_id, &error, window, cx);
                        });
                        return;
                    }
                }
            }

            _ = view.update_in(window, |main, window, cx| {
                main.fail_response_run(
                    assistant_id,
                    &ProviderError::with_kind(
                        provider_id,
                        ProviderErrorKind::IncompleteResponse,
                        IncompleteGeneration,
                    ),
                    window,
                    cx,
                );
            });
        });

        self.stream_tasks.insert(assistant_id, task);
    }

    fn spawn_agent(
        &mut self,
        assistant_id: MessageId,
        provider_id: magenta_core::ProviderId,
        stream: magenta_core::AgentRunStream,
        window: &Window,
        cx: &Context<'_, super::super::MainView>,
    ) {
        let task = cx.spawn_in(window, async move |view, window| {
            let mut stream = stream;

            while let Some(event) = stream.next().await {
                match event {
                    Ok(event) => {
                        let Ok(continue_stream) = view.update_in(window, |main, window, cx| {
                            main.handle_agent_event(assistant_id, event, window, cx)
                        }) else {
                            return;
                        };

                        if !continue_stream {
                            return;
                        }
                    }
                    Err(error) => {
                        _ = view.update_in(window, |main, window, cx| {
                            main.fail_response_run(assistant_id, &error, window, cx);
                        });
                        return;
                    }
                }
            }

            _ = view.update_in(window, |main, window, cx| {
                main.fail_response_run(
                    assistant_id,
                    &ProviderError::with_kind(
                        provider_id,
                        ProviderErrorKind::IncompleteResponse,
                        IncompleteGeneration,
                    ),
                    window,
                    cx,
                );
            });
        });

        self.stream_tasks.insert(assistant_id, task);
    }
}

impl super::super::MainView {
    pub(super) fn handle_generation_event(
        &mut self,
        assistant_id: MessageId,
        event: GenerationEvent,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        match self
            .response_runs
            .apply_generation_event(assistant_id, event)
        {
            RunEventResult::Completed(outcome) => {
                self.complete_response_run(assistant_id, outcome, window, cx);
                false
            }
            RunEventResult::Continue => {
                self.sync_run(assistant_id, cx);
                true
            }
            RunEventResult::WorkspaceChange(_) | RunEventResult::WorkspaceInvalidated => true,
        }
    }

    pub(super) fn handle_agent_event(
        &mut self,
        assistant_id: MessageId,
        event: AgentRunEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        match self.response_runs.apply_agent_event(assistant_id, event) {
            RunEventResult::Completed(outcome) => {
                self.complete_response_run(assistant_id, outcome, window, cx);
                false
            }
            RunEventResult::WorkspaceChange(change) => {
                self.show_workspace_change(change, window, cx);
                self.sync_run(assistant_id, cx);
                true
            }
            RunEventResult::WorkspaceInvalidated => {
                self.invalidate_workspace(assistant_id, window, cx);
                self.sync_run(assistant_id, cx);
                true
            }
            RunEventResult::Continue => {
                self.sync_run(assistant_id, cx);
                true
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("provider stream ended before completion")]
struct IncompleteGeneration;

fn live_message(
    message: Message,
    sequence: Option<magenta_core::MessageSequence>,
    replaces: Option<MessageId>,
    omitted_context_messages: usize,
    generation: magenta_core::GenerationConfig,
) -> LiveRunMessage {
    LiveRunMessage {
        message,
        sequence,
        created_at: now(),
        generation,
        omitted_context_messages,
        replaces,
    }
}
