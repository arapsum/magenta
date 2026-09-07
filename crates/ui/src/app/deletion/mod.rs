use super::*;

impl MainView {
    fn delete_confirmation_copy(title: &str, cx: &Context<'_, Self>) -> AnyElement {
        v_flex()
            .gap(px(8.))
            .p(px(20.))
            .child(
                div()
                    .font_semibold()
                    .text_size(px(16.))
                    .child(format!("Delete “{title}”?")),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "This conversation and its messages will be permanently deleted. \
                         This cannot be undone.",
                    ),
            )
            .into_any_element()
    }

    fn delete_confirmation_actions(cx: &Context<'_, Self>) -> AnyElement {
        h_flex()
            .justify_end()
            .gap(px(8.))
            .px(px(20.))
            .pb(px(20.))
            .child(
                Button::new("cancel-delete-conversation")
                    .label("Cancel")
                    .outline()
                    .debug_selector(|| "cancel-delete-conversation".into())
                    .accessibility_id("cancel-delete-conversation")
                    .on_click(cx.listener(|main, _, window, cx| {
                        main.cancel_delete_conversation(window, cx);
                    })),
            )
            .child(
                Button::new("confirm-delete-conversation")
                    .label("Delete")
                    .danger()
                    .debug_selector(|| "confirm-delete-conversation".into())
                    .accessibility_id("confirm-delete-conversation")
                    .on_click(cx.listener(|main, _, window, cx| {
                        main.confirm_pending_deletion(window, cx);
                    })),
            )
            .into_any_element()
    }

    fn delete_confirmation_dialog(
        title: &str,
        focus_handle: &FocusHandle,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        v_flex()
            .id("delete-conversation-dialog")
            .track_focus(focus_handle)
            .key_context("DeleteConversationDialog")
            .on_action(
                cx.listener(|main, _: &ConfirmConversationDeletion, window, cx| {
                    main.confirm_pending_deletion(window, cx);
                }),
            )
            .on_action(
                cx.listener(|main, _: &CancelConversationDeletion, window, cx| {
                    main.cancel_delete_conversation(window, cx);
                }),
            )
            .role(Role::AlertDialog)
            .aria_label("Delete conversation")
            .w(px(420.))
            .rounded(px(14.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(Self::delete_confirmation_copy(title, cx))
            .child(Self::delete_confirmation_actions(cx))
            .into_any_element()
    }

    pub(crate) fn delete_confirmation_overlay(&self, cx: &Context<'_, Self>) -> AnyElement {
        let pending = self
            .pending_deletion
            .as_ref()
            .expect("delete confirmation is only rendered while pending");
        let dialog = Self::delete_confirmation_dialog(&pending.title, &pending.focus_handle, cx);
        let backdrop_view = cx.entity();

        div()
            .id("delete-conversation-overlay")
            .absolute()
            .inset_0()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(cx.theme().background.opacity(0.72))
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        backdrop_view.update(cx, |main, cx| {
                            main.cancel_delete_conversation(window, cx);
                        });
                    }),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(dialog),
            )
            .into_any_element()
    }
}
