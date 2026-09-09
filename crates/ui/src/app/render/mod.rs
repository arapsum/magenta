use super::*;
use gpui_component::resizable::{h_resizable, resizable_panel};

impl MainView {
    fn titlebar_controls(&self, cx: &Context<'_, Self>) -> AnyElement {
        let sidebar_view = self.sidebar.clone();
        let sidebar_collapsed = self.sidebar.read(cx).is_collapsed();
        let toggle_icon = if sidebar_collapsed {
            IconName::PanelLeftOpen
        } else {
            IconName::PanelLeftClose
        };
        let toggle_label = if sidebar_collapsed {
            "Show sidebar"
        } else {
            "Hide sidebar"
        };

        h_flex()
            .id("titlebar-controls")
            .h_full()
            .items_center()
            .gap(px(4.))
            .px(px(4.))
            .child(
                Button::new("titlebar-sidebar-toggle")
                    .ghost()
                    .small()
                    .icon(toggle_icon)
                    .tooltip(toggle_label)
                    .accessibility_id("toggle-sidebar")
                    .on_click(move |_, _, cx| {
                        sidebar_view.update(cx, SidebarView::toggle_collapsed);
                    }),
            )
            .when(
                self.workbench.is_some() && self.sidebar.read(cx).active_project().is_some(),
                |this| {
                    let open = self.workbench_open;
                    this.child(
                        Button::new("toggle-code-workbench")
                            .ghost()
                            .small()
                            .child(Icon::empty().path("icons/code.svg").xsmall())
                            .label(if open { "Conversation" } else { "Code" })
                            .tooltip(if open {
                                "Show conversation"
                            } else {
                                "Show code"
                            })
                            .on_click(cx.listener(|main, _, window, cx| {
                                let opening = !main.workbench_open;
                                main.workbench_open = opening;
                                if opening && let Some(workbench) = &main.workbench {
                                    workbench.update(cx, |workbench, cx| {
                                        workbench.prepare_to_show(window, cx);
                                    });
                                }
                                cx.notify();
                            })),
                    )
                },
            )
            .into_any_element()
    }

    fn conversation_content(&self, narrow: bool, cx: &Context<'_, Self>) -> AnyElement {
        let conversation = if self.active_conversation.is_some() {
            self.conversation.clone().into_any_element()
        } else {
            workspace::render(self.composer.clone(), &self.sidebar, cx)
        };
        let Some(workbench) = self.workbench.as_ref() else {
            return conversation;
        };
        if !self.workbench_open {
            return conversation;
        }
        if narrow {
            return workbench.clone().into_any_element();
        }
        h_resizable("conversation-workbench")
            .child(
                resizable_panel()
                    .size(px(620.))
                    .size_range(px(360.)..gpui::Pixels::MAX)
                    .child(conversation),
            )
            .child(
                resizable_panel()
                    .size(px(760.))
                    .size_range(px(280.)..gpui::Pixels::MAX)
                    .child(workbench.clone()),
            )
            .into_any_element()
    }

    fn main_panel(&self, content: AnyElement, narrow: bool, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(titlebar::render(self.titlebar_controls(cx)))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .when(narrow, gpui::Styled::p_0)
                    .when(!narrow, |this| this.p(px(12.)))
                    .child(
                        div()
                            .flex()
                            .size_full()
                            .min_h_0()
                            .min_w_0()
                            .overflow_hidden()
                            .rounded(px(16.))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().tokens.background.background)
                            .when(narrow, |this| {
                                this.rounded(px(0.))
                                    .border_0()
                                    .border_color(cx.theme().transparent)
                            })
                            .child(content),
                    ),
            )
            .into_any_element()
    }

    fn render_frame(
        &self,
        content: AnyElement,
        narrow: bool,
        show_sidebar: bool,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|main, _: &OpenConversationFinder, window, cx| {
                main.open_finder(window, cx);
            }))
            .on_action(
                cx.listener(|main, _: &CloseConversationFinder, window, cx| {
                    main.close_finder(window, cx);
                }),
            )
            .on_action(cx.listener(|main, _: &SelectNextFinderResult, _, cx| {
                main.move_finder_selection(1, cx);
            }))
            .on_action(cx.listener(|main, _: &SelectPreviousFinderResult, _, cx| {
                main.move_finder_selection(-1, cx);
            }))
            .on_action(cx.listener(|main, _: &ConfirmFinderResult, window, cx| {
                main.confirm_finder_selection(window, cx);
            }))
            .on_action(cx.listener(|main, _: &titlebar::CloseWindow, window, cx| {
                if main.request_close(window, cx) {
                    window.remove_window();
                }
            }))
            .relative()
            .flex()
            .flex_row()
            .size_full()
            .bg(cx.theme().tokens.background.background)
            .when(show_sidebar, |this| this.child(self.sidebar.clone()))
            .child(self.main_panel(content, narrow, cx))
            .when(self.finder_open.is_open(), |this| {
                this.child(self.finder_overlay(cx))
            })
            .when(self.pending_deletion.is_some(), |this| {
                this.child(self.delete_confirmation_overlay(cx))
            })
            .when(self.loading_conversation.is_some(), |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(36.))
                        .right(px(24.))
                        .p(px(8.))
                        .bg(cx.theme().background)
                        .child("Loading conversation…"),
                )
            })
            .when(
                self.unsaved.is_some() && self.operation == history::Operation::Idle,
                |this| {
                    this.child(
                        Button::new("retry-save")
                            .label("Response not saved · Retry")
                            .absolute()
                            .top(px(36.))
                            .right(px(24.))
                            .on_click(
                                cx.listener(|main, _, window, cx| main.retry_save(window, cx)),
                            ),
                    )
                },
            )
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let _ = &self.subscriptions;
        let narrow = window.viewport_size().width < px(696.);
        let content = self.conversation_content(narrow, cx);
        let sidebar_collapsed = self.sidebar.read(cx).is_collapsed();
        let show_sidebar = !narrow && !sidebar_collapsed;
        self.render_frame(content, narrow, show_sidebar, cx)
    }
}
