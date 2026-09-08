use super::*;

impl SidebarView {
    fn new_chat_button(&self, view: Entity<Self>, _cx: &App) -> AnyElement {
        let selected = self.active_conversation.is_none() && self.active_project.is_none();
        Button::new("sidebar-new-chat")
            .when(selected, gpui_component::button::ButtonVariants::primary)
            .when(!selected, gpui_component::button::ButtonVariants::ghost)
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
            .rounded(px(9.))
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
                        .bg(cx.theme().foreground.opacity(0.7)),
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
            .rounded(px(9.))
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
                            .text_size(px(13.))
                            .when(selected, gpui_component::StyledExt::font_medium)
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
                        .when(selected, gpui::Styled::invisible)
                        .group_hover(group_name.clone(), gpui::Styled::invisible)
                        .child(updated),
                )
            })
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .when(!selected, |this| {
                        this.invisible()
                            .group_hover(group_name, gpui::Styled::visible)
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
                    .h(px(32.))
                    .flex()
                    .items_end()
                    .px(px(8.))
                    .pb(px(6.))
                    .text_size(px(11.))
                    .font_medium()
                    .text_color(cx.theme().muted_foreground.opacity(0.9))
                    .child(title)
                    .into_any_element()
            },
            |expanded| {
                let label_view = view;
                Button::new("pinned-disclosure")
                    .ghost()
                    .w_full()
                    .h(px(32.))
                    .px(px(7.))
                    .rounded(px(6.))
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                Icon::new(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .xsmall(),
                            )
                            .child(div().text_size(px(11.)).font_medium().child(title)),
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
        mut content: gpui::Div,
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
            .h(px(34.))
            .px(px(10.))
            .gap(px(7.))
            .rounded(px(9.))
            .border_1()
            .border_color(cx.theme().input.opacity(0.72))
            .bg(cx.theme().sidebar_accent.opacity(0.32))
            .text_color(cx.theme().muted_foreground)
            .hover(|this| this.bg(cx.theme().sidebar_accent.opacity(0.72)))
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
            .gap(px(6.))
            .child(self.new_chat_button(view.clone(), cx))
            .child(self.search_button(cx));
        let mut content = v_flex().w_full().gap(px(4.)).child(controls);

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
                Some(self.pinned_expanded),
                view.clone(),
                cx,
            ));
            if self.pinned_expanded {
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
