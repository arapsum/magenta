use super::*;
use gpui_kit::{linear_color_stop, linear_gradient};

impl SidebarView {
    fn brand_header(cx: &App) -> AnyElement {
        let mark = linear_gradient(
            135.,
            linear_color_stop(cx.theme().primary, 0.),
            linear_color_stop(cx.theme().yellow, 1.),
        );

        h_flex()
            .h(px(42.))
            .items_center()
            .gap(px(9.))
            .px(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(28.))
                    .rounded(px(9.))
                    .bg(mark)
                    .text_color(cx.theme().primary_foreground)
                    .shadow_sm()
                    .child(Icon::new(IconName::Bot).xsmall()),
            )
            .child(
                div()
                    .text_size(px(15.))
                    .font_semibold()
                    .text_color(cx.theme().foreground)
                    .child("Magenta"),
            )
            .child(
                div()
                    .size(px(5.))
                    .rounded_full()
                    .bg(cx.theme().primary.opacity(0.86)),
            )
            .into_any_element()
    }

    fn new_chat_button(view: Entity<Self>) -> AnyElement {
        Button::new("sidebar-new-chat")
            .primary()
            .accessibility_id("new-chat")
            .w_full()
            .h(px(36.))
            .rounded(px(10.))
            .icon(IconName::Plus)
            .label("New chat")
            .on_click(move |_, _window, cx| {
                view.update(cx, |_, cx| Self::new_chat(cx));
            })
            .into_any_element()
    }

    pub(super) fn conversation_row(
        &self,
        conversation: &ConversationSummary,
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let selected = self.active_conversation == Some(conversation.id);
        let id = conversation.id;
        let group_name: SharedString = format!("conversation-row-{}", id.0).into();
        let metadata = (!conversation.pinned).then(|| conversation.updated.clone());
        let rename = self.rename.as_ref().filter(|rename| rename.id == id);
        let is_renaming = rename.is_some();
        let selected_background = cx.theme().sidebar_accent;
        let hover_background = if selected {
            cx.theme().sidebar_accent
        } else {
            cx.theme().sidebar_accent.opacity(0.72)
        };

        h_flex()
            .relative()
            .group(group_name.clone())
            .w_full()
            .h(ROW_HEIGHT)
            .items_center()
            .rounded(ROW_RADIUS)
            .border_1()
            .border_color(if selected {
                cx.theme().primary.opacity(0.3)
            } else {
                cx.theme().transparent
            })
            .when(selected, |this| {
                this.bg(selected_background)
                    .text_color(cx.theme().foreground)
            })
            .hover(move |this| this.bg(hover_background))
            .when(selected, |this| {
                this.child(
                    div()
                        .absolute()
                        .left(px(0.))
                        .top(px(8.))
                        .bottom(px(8.))
                        .w(px(2.))
                        .rounded_full()
                        .bg(cx.theme().primary),
                )
            })
            .child(rename.map_or_else(
                || Self::conversation_title(conversation, selected, view.clone(), cx),
                |rename| Self::conversation_rename_input(id, rename, cx),
            ))
            .when(!is_renaming, |this| {
                this.child(Self::conversation_actions(
                    ConversationActionData {
                        id,
                        selected,
                        metadata,
                        group_name,
                        pinned: conversation.pinned,
                        history_actions: self.history_actions,
                    },
                    view,
                    cx,
                ))
            })
            .into_any_element()
    }

    fn conversation_title(
        conversation: &ConversationSummary,
        selected: bool,
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let id = conversation.id;
        Button::new(("conversation", id.0))
            .ghost()
            .accessibility_id(format!("conversation-{}", id.0))
            .flex_1()
            .min_w_0()
            .h_full()
            .px(px(8.))
            .rounded(ROW_RADIUS)
            .text_color(if selected {
                cx.theme().sidebar_accent_foreground
            } else {
                cx.theme().sidebar_foreground
            })
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap(px(7.))
                    .child(
                        provider_icon(Some(&conversation.provider))
                            .xsmall()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(13.5))
                            .when(selected, gpui_kit::component::StyledExt::font_medium)
                            .child(conversation.title.clone()),
                    ),
            )
            .on_click(move |_, _, cx| {
                view.update(cx, |_, cx| Self::select_conversation(id, cx));
            })
            .into_any_element()
    }

    fn conversation_rename_input(id: ConversationId, rename: &RenameState, cx: &App) -> AnyElement {
        Input::new(&rename.input)
            .appearance(false)
            .bordered(false)
            .flex_1()
            .min_w_0()
            .h_full()
            .px(px(8.))
            .rounded(px(6.))
            .border_1()
            .border_color(if rename.invalid {
                cx.theme().danger
            } else {
                cx.theme().ring
            })
            .bg(cx.theme().background)
            .text_size(px(13.))
            .accessibility_id(format!("rename-conversation-{}", id.0))
            .aria_label(if rename.invalid {
                "Conversation title is required"
            } else {
                "Conversation title"
            })
            .into_any_element()
    }

    fn conversation_actions(
        actions: ConversationActionData,
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let ConversationActionData {
            id,
            selected,
            metadata,
            group_name,
            pinned,
            history_actions,
        } = actions;
        let history_actions_enabled = history_actions.is_enabled();
        let next_pinned = !pinned;
        let menu_label = if next_pinned { "Pin" } else { "Unpin" };

        div()
            .relative()
            .flex_none()
            .size(px(30.))
            .when_some(metadata, |this, updated| {
                this.child(
                    h_flex()
                        .absolute()
                        .inset_0()
                        .justify_center()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_size(px(10.))
                        .text_color(cx.theme().muted_foreground.opacity(0.8))
                        .when(selected, gpui_kit::Styled::invisible)
                        .group_hover(group_name.clone(), gpui_kit::Styled::invisible)
                        .child(updated),
                )
            })
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .when(!selected, |this| {
                        this.invisible()
                            .group_hover(group_name, gpui_kit::Styled::visible)
                    })
                    .child(
                        Button::new(("conversation-more", id.0))
                            .ghost()
                            .xsmall()
                            .size(px(30.))
                            .p_0()
                            .icon(IconName::Ellipsis)
                            .tooltip(format!("{menu_label} conversation"))
                            .accessibility_id(format!("conversation-more-{}", id.0))
                            .text_color(cx.theme().muted_foreground)
                            .dropdown_menu(move |menu, window, _cx| {
                                let action_view = view.clone();
                                let rename_view = view.clone();
                                let delete_view = view.clone();
                                menu.item(
                                    PopupMenuItem::new(menu_label)
                                        .icon(Icon::empty().path("icons/conversation-pin.svg"))
                                        .disabled(!history_actions_enabled)
                                        .on_click(window.listener_for(
                                            &action_view,
                                            move |sidebar, _, _, cx| {
                                                sidebar.set_pinned(id, next_pinned, cx);
                                            },
                                        )),
                                )
                                .item(
                                    PopupMenuItem::new("Rename")
                                        .icon(Icon::empty().path("icons/conversation-rename.svg"))
                                        .disabled(!history_actions_enabled)
                                        .on_click(window.listener_for(
                                            &rename_view,
                                            move |sidebar, _, window, cx| {
                                                sidebar.start_rename(id, window, cx);
                                            },
                                        )),
                                )
                                .item(PopupMenuItem::separator())
                                .item(
                                    PopupMenuItem::new("Delete")
                                        .icon(Icon::empty().path("icons/conversation-delete.svg"))
                                        .disabled(!history_actions_enabled)
                                        .on_click(window.listener_for(
                                            &delete_view,
                                            move |sidebar, _, _, cx| {
                                                if sidebar.history_actions.is_enabled() {
                                                    cx.emit(SidebarEvent::DeleteConversation(id));
                                                }
                                            },
                                        )),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    fn section_label(
        title: &'static str,
        disclosure: Option<bool>,
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        disclosure.map_or_else(
            || {
                div()
                    .h(SECTION_LABEL_HEIGHT)
                    .flex()
                    .items_center()
                    .px(px(8.))
                    .text_size(px(11.))
                    .font_medium()
                    .text_color(cx.theme().muted_foreground.opacity(0.78))
                    .child(title)
                    .into_any_element()
            },
            |expanded| {
                let label_view = view;
                Button::new("pinned-disclosure")
                    .ghost()
                    .w_full()
                    .h(SECTION_LABEL_HEIGHT)
                    .px(px(7.))
                    .rounded(px(6.))
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap(px(8.))
                            .child(Icon::empty().path("icons/conversation-pin.svg").xsmall())
                            .child(div().flex_1().text_size(px(13.)).font_medium().child(title))
                            .child(
                                Icon::new(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .xsmall()
                                .text_color(cx.theme().muted_foreground.opacity(0.58)),
                            ),
                    )
                    .on_click(move |_, _, cx| {
                        label_view.update(cx, Self::toggle_pinned_expanded);
                    })
                    .into_any_element()
            },
        )
    }

    fn render_history_status(
        &self,
        mut content: gpui_kit::Div,
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        if let Some(status) = self.history_status {
            content = content.child(div().p(px(9.)).text_size(px(12.)).child(status));
            if self.history_failed {
                let retry_view = view;
                content = content.child(
                    Button::new("retry-history")
                        .ghost()
                        .label("Retry")
                        .on_click(move |_, _, cx| {
                            retry_view.update(cx, |_, cx| cx.emit(SidebarEvent::RetryHistory));
                        }),
                );
            }
            return content.into_any_element();
        }
        if self.conversations.is_empty() {
            return content
                .child(
                    div()
                        .p(px(9.))
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child("Your conversations will appear here."),
                )
                .into_any_element();
        }

        content.into_any_element()
    }

    fn search_button(&self, cx: &App) -> AnyElement {
        let finder_shortcut = if cfg!(target_os = "macos") {
            "⌘K"
        } else {
            "Ctrl K"
        };
        h_flex()
            .id("sidebar-search-chats")
            .track_focus(&self.finder_launcher_focus)
            .role(Role::Button)
            .aria_label("Search chats")
            .tab_stop(true)
            .cursor_pointer()
            .w_full()
            .h(px(32.))
            .px(px(10.))
            .gap(px(7.))
            .rounded(px(8.))
            .border_1()
            .border_color(cx.theme().input.opacity(0.72))
            .bg(cx.theme().popover.opacity(0.62))
            .text_color(cx.theme().muted_foreground)
            .hover(|this| this.bg(cx.theme().sidebar_accent.opacity(0.58)))
            .child(Icon::new(IconName::Search).xsmall())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.))
                    .child("Search chats"),
            )
            .child(
                h_flex()
                    .h(px(18.))
                    .min_w(if cfg!(target_os = "macos") {
                        px(24.)
                    } else {
                        px(38.)
                    })
                    .justify_center()
                    .px(px(5.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().sidebar)
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
                    .child(finder_shortcut),
            )
            .on_click(move |_, window, cx| {
                window.dispatch_action(Box::new(OpenConversationFinder), cx);
            })
            .into_any_element()
    }

    pub(super) fn render_content(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        let controls = v_flex()
            .w_full()
            .gap(px(8.))
            .child(Self::brand_header(cx))
            .child(Self::new_chat_button(view.clone()))
            .child(self.search_button(cx));
        let mut content = v_flex().w_full().gap(px(6.)).child(controls).child(
            div()
                .mx(px(4.))
                .my(px(3.))
                .h(px(1.))
                .bg(cx.theme().sidebar_border.opacity(0.72)),
        );

        content = content.child(self.render_projects(&view, cx));

        if self.history_status.is_some() || self.conversations.is_empty() {
            return self.render_history_status(content, view, cx);
        }

        let pinned: Vec<_> = self
            .conversations
            .iter()
            .filter(|item| item.pinned && !self.belongs_to_registered_project(item))
            .collect();
        if !pinned.is_empty() {
            content = content.child(Self::section_label(
                "Pinned",
                Some(self.pinned_disclosure.is_expanded()),
                view.clone(),
                cx,
            ));
            if self.pinned_disclosure.is_expanded() {
                for item in pinned {
                    content = content.child(self.conversation_row(item, view.clone(), cx));
                }
            }
        }

        let recency: Vec<_> = self
            .conversations
            .iter()
            .filter(|item| !item.pinned && !self.belongs_to_registered_project(item))
            .collect();
        let limited_recency: Vec<_> = recency.iter().take(self.recency_limit).copied().collect();
        for period in ConversationPeriod::ALL {
            let grouped: Vec<_> = limited_recency
                .iter()
                .copied()
                .filter(|item| item.period == period)
                .collect();
            if !grouped.is_empty() {
                content =
                    content.child(Self::section_label(period.label(), None, view.clone(), cx));
                for item in grouped {
                    content = content.child(self.conversation_row(item, view.clone(), cx));
                }
            }
        }

        if recency.len() > limited_recency.len() {
            let more_view = view;
            content = content.child(
                Button::new("show-more-conversations")
                    .ghost()
                    .w_full()
                    .h(px(30.))
                    .px(px(8.))
                    .rounded(px(6.))
                    .text_color(cx.theme().muted_foreground)
                    .label("Show more")
                    .on_click(move |_, _, cx| more_view.update(cx, Self::show_more)),
            );
        }

        content.into_any_element()
    }
}
