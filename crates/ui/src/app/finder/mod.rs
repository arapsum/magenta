use super::*;

impl MainView {
    pub(crate) fn move_finder_selection(&mut self, offset: isize, cx: &mut Context<'_, Self>) {
        if !self.finder_open.is_open() {
            return;
        }

        let query = self.finder_input.read(cx).value();
        let count = self
            .sidebar
            .read(cx)
            .matching_conversations(query.trim())
            .len()
            + 1;
        self.finder_selected = (self.finder_selected.cast_signed() + offset)
            .rem_euclid(count.cast_signed())
            .cast_unsigned();
        cx.notify();
    }

    pub(crate) fn confirm_finder_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.finder_open.is_open() {
            return;
        }

        let query = self.finder_input.read(cx).value();
        let matches = self.sidebar.read(cx).matching_conversations(query.trim());
        if let Some((id, _, _)) = matches.get(self.finder_selected) {
            self.select_finder_result(*id, window, cx);
        } else {
            self.new_chat_from_finder(window, cx);
        }
    }
    pub(crate) fn open_finder(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.finder_open = PanelState::Open;
        self.finder_selected = 0;
        self.finder_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.finder_input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        cx.notify();
    }

    pub(crate) fn close_finder(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.finder_open.is_open() {
            return;
        }

        self.finder_open = PanelState::Closed;
        self.finder_selected = 0;
        self.finder_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        if !self.sidebar.read(cx).is_collapsed() && window.viewport_size().width >= px(696.) {
            self.sidebar
                .update(cx, |sidebar, cx| sidebar.focus_finder_launcher(window, cx));
        } else {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    fn select_finder_result(
        &mut self,
        id: ConversationId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.close_finder(window, cx);
        self.navigate(Some(id), window, cx);
    }

    fn new_chat_from_finder(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.close_finder(window, cx);
        self.navigate(None, window, cx);
        self.composer
            .update(cx, |composer, cx| composer.focus(window, cx));
    }

    fn finder_conversation_row(
        id: ConversationId,
        title: SharedString,
        updated: SharedString,
        selected: bool,
        view: Entity<Self>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let accessibility_label = title.to_string();
        h_flex()
            .id(("finder-conversation", id.0))
            .debug_selector(move || format!("finder-conversation-{}", id.0))
            .role(Role::ListBoxOption)
            .aria_label(accessibility_label)
            .aria_selected(selected)
            .cursor_pointer()
            .w_full()
            .h(px(40.))
            .px(px(10.))
            .gap(px(12.))
            .rounded(px(8.))
            .when(selected, |this| {
                this.bg(cx.theme().accent)
                    .text_color(cx.theme().accent_foreground)
            })
            .hover(|this| this.bg(cx.theme().accent))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .size(px(14.))
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .absolute()
                            .top(px(1.))
                            .left(px(1.))
                            .size(px(11.))
                            .rounded(px(3.))
                            .border_1()
                            .border_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .absolute()
                            .bottom(px(0.))
                            .left(px(3.))
                            .size(px(3.))
                            .border_l_1()
                            .border_b_1()
                            .border_color(cx.theme().muted_foreground),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .font_medium()
                    .text_size(px(13.))
                    .child(title),
            )
            .child(
                div()
                    .flex_none()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground.opacity(0.8))
                    .child(updated),
            )
            .on_click(move |_, window, cx| {
                view.update(cx, |main, cx| {
                    main.select_finder_result(id, window, cx);
                });
            })
            .into_any_element()
    }

    fn finder_new_chat_row(
        selected: bool,
        view: Entity<Self>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        h_flex()
            .id("finder-new-chat")
            .role(Role::ListBoxOption)
            .aria_label("New chat")
            .aria_selected(selected)
            .cursor_pointer()
            .w_full()
            .h(px(40.))
            .px(px(10.))
            .gap(px(12.))
            .rounded(px(8.))
            .when(selected, |this| {
                this.bg(cx.theme().accent)
                    .text_color(cx.theme().accent_foreground)
            })
            .hover(|this| this.bg(cx.theme().accent))
            .child(
                Icon::new(IconName::Plus)
                    .xsmall()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_medium()
                    .text_size(px(13.))
                    .child("New chat"),
            )
            .child(
                div()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground.opacity(0.8))
                    .child(if cfg!(target_os = "macos") {
                        "⌘N"
                    } else {
                        "Ctrl N"
                    }),
            )
            .on_click(move |_, window, cx| {
                view.update(cx, |main, cx| {
                    main.new_chat_from_finder(window, cx);
                });
            })
            .into_any_element()
    }

    fn finder_results(
        &self,
        query: &str,
        matches: Vec<(ConversationId, SharedString, SharedString)>,
        view: Entity<Self>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let has_matches = !matches.is_empty();
        let action_index = matches.len();
        let mut result_rows = v_flex().w_full();
        if has_matches {
            result_rows = result_rows.child(
                h_flex()
                    .h(px(28.))
                    .px(px(10.))
                    .font_semibold()
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground.opacity(0.8))
                    .child("CHATS"),
            );
        }
        for (index, (id, title, updated)) in matches.into_iter().enumerate() {
            result_rows = result_rows.child(Self::finder_conversation_row(
                id,
                title,
                updated,
                self.finder_selected == index,
                view.clone(),
                cx,
            ));
        }
        if !has_matches {
            result_rows = result_rows.child(
                div()
                    .w_full()
                    .px(px(10.))
                    .py(px(20.))
                    .text_center()
                    .text_size(px(13.))
                    .text_color(cx.theme().muted_foreground)
                    .child(if query.is_empty() {
                        "No conversations yet."
                    } else {
                        "No conversations match your search."
                    }),
            );
        }

        result_rows = result_rows.child(
            h_flex()
                .h(px(28.))
                .px(px(10.))
                .font_semibold()
                .text_size(px(10.))
                .text_color(cx.theme().muted_foreground.opacity(0.8))
                .child("ACTIONS"),
        );
        result_rows
            .child(Self::finder_new_chat_row(
                self.finder_selected == action_index,
                view,
                cx,
            ))
            .into_any_element()
    }

    fn finder_popover(&self, result_rows: AnyElement, cx: &Context<'_, Self>) -> AnyElement {
        let keycap = |label: &'static str| {
            h_flex()
                .h(px(18.))
                .min_w(px(24.))
                .justify_center()
                .px(px(5.))
                .rounded(px(5.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(px(10.))
                .child(label)
        };
        v_flex()
            .id("conversation-finder-dialog")
            .key_context("ConversationFinder")
            .role(Role::Dialog)
            .aria_label("Search chats")
            .w(px(385.))
            .max_h(px(560.))
            .overflow_hidden()
            .rounded(px(14.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div().p(px(4.)).child(
                    Input::new(&self.finder_input)
                        .appearance(false)
                        .bordered(false)
                        .h(px(36.))
                        .w_full()
                        .px(px(8.))
                        .rounded(px(10.))
                        .bg(cx.theme().muted.opacity(0.55))
                        .accessibility_id("conversation-finder-input")
                        .aria_label("Search chats")
                        .prefix(
                            Icon::new(IconName::Search)
                                .xsmall()
                                .text_color(cx.theme().muted_foreground),
                        ),
                ),
            )
            .child(
                div()
                    .id("finder-result-list")
                    .role(Role::ListBox)
                    .max_h(px(320.))
                    .overflow_y_scrollbar()
                    .p(px(4.))
                    .child(result_rows),
            )
            .child(
                h_flex()
                    .h(px(36.))
                    .px(px(14.))
                    .gap(px(14.))
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted.opacity(0.4))
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground)
                    .child(h_flex().gap(px(5.)).child(keycap("↑↓")).child("Navigate"))
                    .child(h_flex().gap(px(5.)).child(keycap("↵")).child("Open")),
            )
            .into_any_element()
    }

    pub(crate) fn finder_overlay(&self, cx: &Context<'_, Self>) -> AnyElement {
        let query = self.finder_input.read(cx).value().trim().to_owned();
        let matches = self.sidebar.read(cx).matching_conversations(&query);
        let view = cx.entity();
        let result_rows = self.finder_results(&query, matches, view.clone(), cx);
        let popover = self.finder_popover(result_rows, cx);

        div()
            .id("conversation-finder-overlay")
            .absolute()
            .inset_0()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(cx.theme().background.opacity(0.72))
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        view.update(cx, |main, cx| main.close_finder(window, cx));
                    }),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_start()
                    .justify_center()
                    .pt(px(86.))
                    .child(popover),
            )
            .into_any_element()
    }
}
