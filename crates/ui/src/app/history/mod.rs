mod operations;

use gpui::{Context, Window};
use gpui_component::WindowExt as _;
use magenta_application::{SendMessageInput, SendTarget};
use magenta_core::{AttachmentDraft, ConversationId, ConversationMode, MessageStatus};

use super::{AccountState, CloseState, MainView, StorageState};
use crate::{MagentaError, components::prompt_input::PromptRequest, notification_for_error};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Operation {
    Idle,
    Preparing,
    Saving,
    Pinning,
    Renaming,
    Deleting,
}

#[derive(Clone, Copy)]
pub(super) enum Navigation {
    New,
    Conversation(ConversationId),
}

impl MainView {
    pub(super) fn load_history(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if self.history_task.is_some() {
            return;
        }
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.set_history_loading(false, cx));
        let history = self.history.clone();
        self.history_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = async {
                history.initialize().await?;
                history.summaries().await
            }
            .await;
            _ = view.update_in(window, |main, window, cx| {
                main.history_task = None;
                match result {
                    Ok(summaries) => {
                        main.storage_ready = StorageState::Ready;
                        main.sidebar
                            .update(cx, |sidebar, cx| sidebar.set_history(summaries, cx));
                    }
                    Err(source) => {
                        main.storage_ready = StorageState::Failed;
                        main.sidebar
                            .update(cx, |sidebar, cx| sidebar.set_history_loading(true, cx));
                        Self::present_storage_error(
                            &MagentaError::StorageInitialize { source },
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

    fn refresh_summaries(&mut self, window: &Window, cx: &Context<'_, Self>) {
        let history = self.history.clone();
        self.history_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = history.summaries().await;
            _ = view.update_in(window, |main, window, cx| {
                main.history_task = None;
                match result {
                    Ok(summaries) => main
                        .sidebar
                        .update(cx, |sidebar, cx| sidebar.set_history(summaries, cx)),
                    Err(source) => Self::present_storage_error(
                        &MagentaError::StorageLoad { source },
                        window,
                        cx,
                    ),
                }
            });
        }));
    }

    pub(super) fn navigate(
        &mut self,
        id: Option<ConversationId>,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.storage_ready.is_ready() {
            return;
        }
        self.deferred_navigation = Some(id.map_or(Navigation::New, Navigation::Conversation));
        // Invalidate pending reads immediately, including when a save must finish first.
        self.load_generation = self.load_generation.wrapping_add(1);
        self.load_task.take();
        self.page_task.take();
        self.loading_conversation = None;
        if self.conversation.read(cx).is_streaming() {
            self.cancel_generation(cx);
            return;
        }
        self.continue_navigation(window, cx);
    }

    fn continue_navigation(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if self.operation != Operation::Idle || self.unsaved.is_some() {
            return;
        }
        let Some(id) = self.deferred_navigation.take() else {
            return;
        };
        let Navigation::Conversation(id) = id else {
            self.active_conversation = None;
            self.conversation.update(cx, super::ConversationView::clear);
            self.composer.update(cx, |composer, cx| {
                composer.set_conversation_context(ConversationMode::Chat, None, cx);
            });
            self.sidebar
                .update(cx, |sidebar, cx| sidebar.set_active(None, cx));
            self.update_composer_availability(cx);
            cx.notify();
            return;
        };
        let generation = self.load_generation;
        let history = self.history.clone();
        self.loading_conversation = Some(id);
        self.update_composer_availability(cx);
        self.load_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = history.load(id).await;
            _ = view.update_in(window, |main, window, cx| {
                if main.load_generation != generation {
                    return;
                }
                main.load_task = None;
                main.loading_conversation = None;
                match result {
                    Ok(loaded) => {
                        main.composer.update(cx, |composer, cx| {
                            composer.set_configuration(&loaded.conversation.generation, cx);
                            composer.set_conversation_context(
                                loaded.conversation.mode.clone(),
                                loaded.conversation.workspace_root.clone(),
                                cx,
                            );
                        });
                        main.conversation
                            .update(cx, |view, cx| view.load_page(loaded, cx));
                        main.active_conversation = Some(id);
                        main.sidebar
                            .update(cx, |sidebar, cx| sidebar.set_active(Some(id), cx));
                    }
                    Err(source) => Self::present_storage_error(
                        &MagentaError::StorageLoad { source },
                        window,
                        cx,
                    ),
                }
                main.update_composer_availability(cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn load_earlier(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if self.page_task.is_some() || self.loading_conversation.is_some() {
            return;
        }
        let Some(id) = self.active_conversation else {
            return;
        };
        let Some(cursor) = self.conversation.read(cx).earlier_cursor() else {
            return;
        };
        let generation = self.load_generation;
        let history = self.history.clone();
        self.conversation
            .update(cx, |view, cx| view.set_loading_earlier(true, cx));
        self.page_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = history.earlier(id, cursor).await;
            _ = view.update_in(window, |main, window, cx| {
                if main.active_conversation != Some(id) || main.load_generation != generation {
                    return;
                }
                main.page_task = None;
                match result {
                    Ok(page) => main
                        .conversation
                        .update(cx, |view, cx| view.prepend_page(page, cx)),
                    Err(source) => {
                        main.conversation
                            .update(cx, |view, cx| view.set_loading_earlier(false, cx));
                        Self::present_storage_error(
                            &MagentaError::StorageLoad { source },
                            window,
                            cx,
                        );
                    }
                }
            });
        }));
    }

    pub(super) fn submit(
        &mut self,
        request: &PromptRequest,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !matches!(self.account_state, AccountState::Connected(_)) {
            self.begin_login(window, cx);
            return;
        }
        if !self.can_write(cx) {
            return;
        }
        if request.mode == ConversationMode::Agent {
            self.submit_agent(request, window, cx);
            return;
        }
        let workflow = self.send_message.clone();
        let submitted = request.clone();
        let input = SendMessageInput {
            target: self
                .active_conversation
                .map_or(SendTarget::New, SendTarget::Existing),
            prompt: request.prompt.to_string(),
            attachments: request
                .attachments
                .iter()
                .map(|path| AttachmentDraft {
                    name: path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Attachment")
                        .to_owned(),
                    source_path: path.clone(),
                })
                .collect(),
            generation: request.generation.clone(),
            mode: ConversationMode::Chat,
            workspace_root: None,
        };
        self.operation = Operation::Preparing;
        self.update_composer_availability(cx);
        self.operation_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = workflow.execute(input).await;
            _ = view.update_in(window, |main, window, cx| {
                main.operation_task = None;
                main.operation = Operation::Idle;
                match result {
                    Ok(pending) => {
                        main.composer.update(cx, |composer, cx| {
                            composer.clear_submitted(&submitted, window, cx);
                        });
                        main.start_pending(pending, window, cx);
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

    fn can_write(&self, cx: &Context<'_, Self>) -> bool {
        self.storage_ready.is_ready()
            && self.operation == Operation::Idle
            && self.unsaved.is_none()
            && self.loading_conversation.is_none()
            && !self.conversation.read(cx).is_streaming()
    }

    fn update_composer_availability(&self, cx: &mut Context<'_, Self>) {
        let ready = self.can_write(cx);
        self.composer
            .update(cx, |composer, cx| composer.set_storage_ready(ready, cx));
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_history_actions_enabled(ready, cx);
        });
    }

    pub(super) fn request_close(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.unsaved.is_some() && self.operation == Operation::Idle {
            return false;
        }
        if self.conversation.read(cx).is_streaming() {
            self.close_requested = CloseState::Requested;
            self.cancel_generation(cx);
            return false;
        }
        if self.operation != Operation::Idle {
            self.close_requested = CloseState::Requested;
            return false;
        }
        true
    }

    pub(super) fn prepare_shutdown(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) -> impl std::future::Future<Output = ()> + use<> {
        let task = self.operation_task.take();
        let history = self.history.clone();
        let interrupted = self
            .conversation
            .update(cx, super::ConversationView::interrupt_for_shutdown);
        let unsaved = if self.operation == Operation::Idle {
            self.unsaved.take()
        } else {
            None
        };
        async move {
            if let Some(task) = task {
                task.await;
            }
            if let Some(mut message) = interrupted.or(unsaved) {
                if message.status == MessageStatus::Streaming {
                    message.status = MessageStatus::Stopped;
                }
                if let Err(error) = history.finalize(message).await {
                    tracing::error!(
                        kind = ?error.kind,
                        operation = "history.shutdown",
                        "could not save response before shutdown"
                    );
                }
            }
        }
    }

    fn present_storage_error(
        error: &MagentaError,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        tracing::error!(
            code = error.presentation().code,
            operation = "history",
            "conversation operation failed"
        );
        window.push_notification(notification_for_error(error), cx);
    }
}
