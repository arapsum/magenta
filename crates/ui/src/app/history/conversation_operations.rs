use gpui_kit::{Context, Window};
use magenta_core::ConversationId;

use super::{MainView, Operation};
use crate::MagentaError;

impl MainView {
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
                            main.clear_workbench_session(cx);
                            main.conversation
                                .update(cx, super::super::ConversationView::clear);
                            main.sidebar.update(cx, |sidebar, cx| {
                                sidebar.set_active(None, cx);
                                sidebar.set_active_project(None, cx);
                            });
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
