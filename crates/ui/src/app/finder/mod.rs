use super::*;

struct FinderRow {
    result: ConversationSearchResult,
    updated: SharedString,
}

impl MainView {
    pub(super) fn schedule_finder_search(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        self.finder_search_generation = self.finder_search_generation.wrapping_add(1);
        let generation = self.finder_search_generation;
        self.finder_search_task.take();
        let query = self.finder_input.read(cx).value().trim().to_owned();
        self.finder_results.clear();
        if query.is_empty() {
            self.finder_search_status = FinderSearchStatus::Idle;
            cx.notify();
            return;
        }

        self.finder_search_status = FinderSearchStatus::Searching;
        let history = self.history.clone();
        self.finder_search_task = Some(cx.spawn_in(window, async move |view, window| {
            window
                .background_executor()
                .timer(Duration::from_millis(if cfg!(test) { 0 } else { 120 }))
                .await;
            let results = history.search(query, 30).await;
            _ = view.update_in(window, |main, _, cx| {
                if main.finder_search_generation != generation {
                    return;
                }
                main.finder_search_task = None;
                match results {
                    Ok(results) => {
                        main.finder_results = results;
                        main.finder_search_status = FinderSearchStatus::Ready;
                    }
                    Err(error) => {
                        tracing::error!(
                            kind = ?error.kind,
                            operation = "history.search",
                            "conversation search failed"
                        );
                        main.finder_results.clear();
                        main.finder_search_status = FinderSearchStatus::Failed;
                    }
                }
                main.finder_selected = 0;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn finder_rows(&self, query: &str, cx: &Context<'_, Self>) -> Vec<FinderRow> {
        if query.is_empty() {
            return self
                .sidebar
                .read(cx)
                .matching_conversations("")
                .into_iter()
                .map(|(id, title, updated)| FinderRow {
                    result: ConversationSearchResult {
                        conversation_id: id,
                        message_id: None,
                        message_sequence: None,
                        title: title.to_string(),
                        title_highlights: Vec::new(),
                        snippet: String::new(),
                        snippet_highlights: Vec::new(),
                        updated_at: magenta_core::Timestamp(0),
                    },
                    updated,
                })
                .collect();
        }
        self.finder_results
            .iter()
            .cloned()
            .map(|result| FinderRow {
                updated: relative_timestamp(result.updated_at).into(),
                result,
            })
            .collect()
    }

    pub(crate) fn move_finder_selection(&mut self, offset: isize, cx: &mut Context<'_, Self>) {
        if !self.finder_open.is_open() {
            return;
        }

        let query = self.finder_input.read(cx).value().trim().to_owned();
        let count = self.finder_rows(&query, cx).len() + 1;
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

        let query = self.finder_input.read(cx).value().trim().to_owned();
        if !query.is_empty() && self.finder_search_status == FinderSearchStatus::Searching {
            return;
        }
        let matches = self.finder_rows(&query, cx);
        if let Some(row) = matches.get(self.finder_selected) {
            self.select_finder_result(row.result.clone(), window, cx);
        } else {
            self.new_chat_from_finder(window, cx);
        }
    }
    pub(crate) fn open_finder(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.finder_open = PanelState::Open;
        self.finder_selected = 0;
        self.finder_results.clear();
        self.finder_search_status = FinderSearchStatus::Idle;
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
        self.finder_search_generation = self.finder_search_generation.wrapping_add(1);
        self.finder_search_task.take();
        self.finder_results.clear();
        self.finder_search_status = FinderSearchStatus::Idle;
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
        result: ConversationSearchResult,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.close_finder(window, cx);
        self.navigate_to_search_result(result, window, cx);
    }

    fn new_chat_from_finder(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.close_finder(window, cx);
        self.navigate(None, window, cx);
        self.composer
            .update(cx, |composer, cx| composer.focus(window, cx));
    }

    fn finder_conversation_row(
        row: FinderRow,
        selected: bool,
        view: Entity<Self>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let FinderRow { result, updated } = row;
        let id = result.conversation_id;
        let accessibility_label = if result.snippet.is_empty() {
            result.title.clone()
        } else {
            format!("{}. {}", result.title, result.snippet)
        };
        let title = highlighted_text(result.title.clone(), result.title_highlights.clone());
        let has_snippet = !result.snippet.is_empty();
        let snippet = highlighted_text(result.snippet.clone(), result.snippet_highlights.clone());
        let selected_result = result;
        h_flex()
            .id(("finder-conversation", id.0))
            .debug_selector(move || format!("finder-conversation-{}", id.0))
            .role(Role::ListBoxOption)
            .aria_label(accessibility_label)
            .aria_selected(selected)
            .cursor_pointer()
            .w_full()
            .min_h(if has_snippet { px(58.) } else { px(40.) })
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
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .w_full()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .font_medium()
                            .text_size(px(13.))
                            .child(title),
                    )
                    .when(has_snippet, |this| {
                        this.child(
                            div()
                                .w_full()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(12.))
                                .text_color(cx.theme().muted_foreground)
                                .child(snippet),
                        )
                    }),
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
                    main.select_finder_result(selected_result.clone(), window, cx);
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
        matches: Vec<FinderRow>,
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
        for (index, row) in matches.into_iter().enumerate() {
            result_rows = result_rows.child(Self::finder_conversation_row(
                row,
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
                    .child(match (query.is_empty(), self.finder_search_status) {
                        (true, _) => "No conversations yet.",
                        (false, FinderSearchStatus::Searching) => "Searching conversation history…",
                        (false, FinderSearchStatus::Failed) => "Search is unavailable. Try again.",
                        (false, _) => "No conversations match your search.",
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
            .h(px(440.))
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
                    .debug_selector(|| "finder-result-list".to_owned())
                    .role(Role::ListBox)
                    .flex_1()
                    .min_h_0()
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
        let matches = self.finder_rows(&query, cx);
        let view = cx.entity();
        let result_rows = self.finder_results(&query, matches, view.clone(), cx);
        let popover = self.finder_popover(result_rows, cx);

        div()
            .id("conversation-finder-overlay")
            .absolute()
            .inset_0()
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
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

fn highlighted_text(text: String, ranges: Vec<std::ops::Range<usize>>) -> StyledText {
    let highlights = ranges.into_iter().map(|range| {
        (
            range,
            HighlightStyle {
                font_weight: Some(FontWeight::BOLD),
                ..Default::default()
            },
        )
    });
    StyledText::new(SharedString::from(text)).with_highlights(highlights)
}

fn relative_timestamp(timestamp: magenta_core::Timestamp) -> String {
    let Some(updated_at) = chrono::DateTime::from_timestamp_millis(timestamp.0)
        .map(|time| time.with_timezone(&chrono::Local))
    else {
        return String::new();
    };
    let minutes = chrono::Local::now()
        .signed_duration_since(updated_at)
        .num_minutes()
        .max(0);
    match minutes {
        0 => "now".to_owned(),
        1..=59 => format!("{minutes}m"),
        60..=1439 => format!("{}h", minutes / 60),
        _ => format!("{}d", minutes / 1_440),
    }
}
