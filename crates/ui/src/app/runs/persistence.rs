use gpui_kit::{Context, Window};
use magenta_core::{AgentApprovalDecision, AgentWorkspaceChange, MessageId, ProviderError};

use super::super::MainView;
use crate::components::sidebar::ConversationActivity;

impl MainView {
    pub(crate) fn sync_run(&self, assistant_id: MessageId, cx: &mut Context<'_, Self>) {
        let Some(run) = self.response_runs.run_for_message(assistant_id) else {
            return;
        };
        let conversation_id = run.conversation.id;
        if self.active_conversation == Some(conversation_id)
            && let Some(snapshot) = self
                .response_runs
                .snapshot_for_conversation(conversation_id)
        {
            self.conversation.update(cx, |conversation, cx| {
                conversation.apply_live_run(snapshot, cx);
            });
            self.composer.update(cx, |composer, cx| {
                composer
                    .set_generating(self.response_runs.has_active_for(Some(conversation_id)), cx);
            });
        }
        self.sync_sidebar_activity(conversation_id, cx);
        self.update_composer_availability(cx);
    }

    pub(crate) fn sync_sidebar_activity(
        &self,
        conversation_id: magenta_core::ConversationId,
        cx: &mut Context<'_, Self>,
    ) {
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_run_indicator(
                conversation_id,
                self.response_runs
                    .indicator(conversation_id)
                    .map(|indicator| match indicator {
                        super::RunIndicator::Running => ConversationActivity::Running,
                        super::RunIndicator::AwaitingApproval => {
                            ConversationActivity::AwaitingApproval
                        }
                        super::RunIndicator::Unsaved => ConversationActivity::Unsaved,
                    }),
                cx,
            );
        });
    }

    pub(crate) fn complete_response_run(
        &mut self,
        assistant_id: MessageId,
        outcome: magenta_core::GenerationOutcome,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.response_runs.complete(assistant_id, outcome).is_some() {
            self.finish_workspace_review(assistant_id, window, cx);
            self.sync_run(assistant_id, cx);
            self.save_run(assistant_id, window, cx);
        }
    }

    pub(crate) fn fail_response_run(
        &mut self,
        assistant_id: MessageId,
        error: &ProviderError,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        tracing::error!(
            provider = %error.provider.0,
            kind = ?error.kind,
            message_id = assistant_id.0,
            operation = "conversation.generate",
            "provider generation failed"
        );
        if self.response_runs.fail(assistant_id, error).is_some() {
            self.sync_run(assistant_id, cx);
            self.finish_workspace_review(assistant_id, window, cx);
            self.save_run(assistant_id, window, cx);
        }
    }

    pub(crate) fn stop_run(
        &mut self,
        assistant_id: MessageId,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.response_runs.stop(assistant_id).is_some() {
            self.sync_run(assistant_id, cx);
            self.finish_workspace_review(assistant_id, window, cx);
            self.save_run(assistant_id, window, cx);
        }
    }

    fn finish_workspace_review(
        &mut self,
        message_id: MessageId,
        window: &Window,
        cx: &Context<'_, Self>,
    ) {
        let Some(controller) = self.response_runs.controller_for(message_id) else {
            return;
        };
        let weak = cx.weak_entity();
        let task = cx.spawn_in(window, async move |_, window| {
            let result = controller.finish_workspace_review().await;
            _ = weak.update_in(window, |main, _window, cx| {
                main.response_runs.control_tasks.remove(&message_id);
                match result {
                    Ok(()) => {
                        if !controller.has_pending_workspace_review() {
                            main.response_runs.set_controller(message_id, None);
                            main.response_runs.remove_after_review(message_id);
                        }
                    }
                    Err(error) => {
                        tracing::error!(message_id = message_id.0, %error, "could not finish Work session");
                    }
                }
                main.sync_run(message_id, cx);
                cx.notify();
            });
        });
        self.response_runs.control_tasks.insert(message_id, task);
    }

    pub(crate) fn stop_active_run(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        let Some(conversation_id) = self.active_conversation else {
            return;
        };
        let Some(message_id) = self
            .response_runs
            .run_for_conversation(conversation_id)
            .map(super::state::ResponseRun::assistant_id)
        else {
            return;
        };
        self.stop_run(message_id, window, cx);
    }

    pub(crate) fn decide_agent_approval(
        &mut self,
        message_id: MessageId,
        decision: AgentApprovalDecision,
        cx: &mut Context<'_, Self>,
    ) {
        if self
            .response_runs
            .decide_agent_approval(message_id, decision)
        {
            self.sync_run(message_id, cx);
        }
    }

    pub(crate) fn apply_workspace_review(
        &mut self,
        message_id: MessageId,
        window: &Window,
        cx: &Context<'_, Self>,
    ) {
        let Some(controller) = self.response_runs.controller_for(message_id) else {
            return;
        };
        let weak = cx.weak_entity();
        let task = cx.spawn_in(window, async move |_, window| {
            let result = controller.apply_workspace_changes().await;
            _ = weak.update_in(window, |main, _window, cx| {
                if result.is_ok() {
                    main.response_runs.control_tasks.remove(&message_id);
                    main.response_runs.set_controller(message_id, None);
                    main.response_runs.remove_after_review(message_id);
                    main.sync_run(message_id, cx);
                    cx.notify();
                }
            });
        });
        self.response_runs.control_tasks.insert(message_id, task);
    }

    pub(crate) fn discard_workspace_review(
        &mut self,
        message_id: MessageId,
        window: &Window,
        cx: &Context<'_, Self>,
    ) {
        let Some(controller) = self.response_runs.controller_for(message_id) else {
            return;
        };
        let weak = cx.weak_entity();
        let task = cx.spawn_in(window, async move |_, window| {
            let result = controller.discard_workspace_changes().await;
            _ = weak.update_in(window, |main, _window, cx| {
                if result.is_ok() {
                    main.response_runs.control_tasks.remove(&message_id);
                    main.response_runs.set_controller(message_id, None);
                    main.response_runs.remove_after_review(message_id);
                    main.sync_run(message_id, cx);
                    cx.notify();
                }
            });
        });
        self.response_runs.control_tasks.insert(message_id, task);
    }

    fn save_run(&mut self, message_id: MessageId, window: &Window, cx: &mut Context<'_, Self>) {
        if self.response_runs.save_tasks.contains_key(&message_id) {
            return;
        }
        let Some(message) = self.response_runs.message_for_save(message_id) else {
            return;
        };
        self.response_runs.mark_saving(message_id);
        let history = self.history.clone();
        let weak = cx.weak_entity();
        let task = cx.spawn_in(window, async move |_, window| {
            let result = history.finalize(message).await;
            _ = weak.update_in(window, |main, window, cx| {
                main.response_runs.save_tasks.remove(&message_id);
                match result {
                    Ok(()) => {
                        let conversation_id = main
                            .response_runs
                            .run_for_message(message_id)
                            .map(|run| run.conversation.id);
                        main.response_runs.mark_saved(message_id);
                        if let Some(conversation_id) = conversation_id {
                            main.sync_sidebar_activity(conversation_id, cx);
                        }
                        main.refresh_summaries(window, cx);
                        main.continue_navigation(window, cx);
                        if main.close_requested.is_requested()
                            && !main.response_runs.has_blocking_work()
                        {
                            window.remove_window();
                        }
                    }
                    Err(source) => {
                        let conversation_id = main
                            .response_runs
                            .run_for_message(message_id)
                            .map(|run| run.conversation.id);
                        main.response_runs.mark_save_failed(message_id);
                        if let Some(conversation_id) = conversation_id {
                            main.sync_sidebar_activity(conversation_id, cx);
                        }
                        main.close_requested = super::super::CloseState::Open;
                        tracing::error!(
                            kind = ?source.kind,
                            error = %source.source,
                            message_id = message_id.0,
                            operation = "response.save",
                            "response could not be saved; retaining per-message recovery state"
                        );
                    }
                }
                main.update_composer_availability(cx);
                cx.notify();
            });
        });
        self.response_runs.save_tasks.insert(message_id, task);
        self.sync_run(message_id, cx);
    }

    pub(crate) fn retry_unsaved_run(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if self.operation != super::super::history::Operation::Idle {
            return;
        }
        let Some(conversation_id) = self.active_conversation else {
            return;
        };
        let Some(message_id) = self
            .response_runs
            .run_for_conversation(conversation_id)
            .filter(|run| run.is_unsaved())
            .map(super::state::ResponseRun::assistant_id)
        else {
            return;
        };
        self.response_runs.mark_pending_save(message_id);
        self.save_run(message_id, window, cx);
    }

    pub(crate) fn show_workspace_change(
        &mut self,
        change: AgentWorkspaceChange,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(workbench) = &self.workbench {
            self.workbench_open = true;
            workbench.update(cx, |workbench, cx| {
                workbench.prepare_to_show(window, cx);
                workbench.show_change(change, window, cx);
            });
        }
    }

    pub(crate) fn invalidate_workspace(
        &self,
        _message_id: MessageId,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(workbench) = &self.workbench {
            let visible = self.workbench_open;
            workbench.update(cx, |workbench, cx| {
                workbench.mark_workspace_invalidated(visible, window, cx);
            });
        }
    }
}

impl super::ResponseRunCoordinator {
    pub(crate) fn has_blocking_work(&self) -> bool {
        self.runs.values().any(|run| {
            run.is_active()
                || run.is_unsaved()
                || run.controller.as_ref().is_some_and(
                    magenta_application::AgentApprovalController::has_pending_workspace_review,
                )
        }) || !self.save_tasks.is_empty()
            || !self.control_tasks.is_empty()
    }
}
