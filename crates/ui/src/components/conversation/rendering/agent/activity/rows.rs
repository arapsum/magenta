use super::*;

pub(super) fn render(
    conversation: &ConversationView,
    message: &Message,
    tab: TraceTab,
    cx: &App,
    view: &Entity<ConversationView>,
) -> AnyElement {
    let entries = data::entries(message, tab);
    let mut rows = v_flex().w_full().min_w_0().gap(px(10.));
    if entries.is_empty() {
        let label = match tab {
            TraceTab::Steps => "Activity will appear here.",
            TraceTab::Reasoning => "No reasoning summary provided yet.",
            TraceTab::Search => "No project searches in this response.",
            TraceTab::Tools => "No tool calls in this response.",
        };
        return rows
            .child(
                div()
                    .py(px(3.))
                    .text_size(px(12.))
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .into_any_element();
    }
    for entry in entries {
        rows = rows.child(match tab {
            TraceTab::Reasoning => div()
                .debug_selector(|| "assistant-activity-reasoning".to_owned())
                .w_full()
                .min_w_0()
                .child(render_reasoning_summary(message.id, entry, cx))
                .into_any_element(),
            TraceTab::Steps => step(conversation, message.id, entry, cx, view),
            TraceTab::Tools => tool(conversation, message.id, entry, cx, view),
            TraceTab::Search => search(conversation, message.id, entry, cx, view),
        });
    }
    rows.into_any_element()
}

fn indicator(entry: &AssistantTraceEntry, cx: &App) -> AnyElement {
    let icon = match entry.status {
        AssistantTraceStatus::Completed => IconName::Check,
        AssistantTraceStatus::Failed
        | AssistantTraceStatus::Rejected
        | AssistantTraceStatus::Stopped => IconName::CircleX,
        AssistantTraceStatus::AwaitingApproval => IconName::Info,
        _ => IconName::LoaderCircle,
    };
    Icon::new(icon)
        .size(px(13.))
        .text_color(trace_status_color(entry.status, cx).opacity(0.8))
        .into_any_element()
}

fn step(
    conversation: &ConversationView,
    message_id: MessageId,
    entry: &AssistantTraceEntry,
    cx: &App,
    view: &Entity<ConversationView>,
) -> AnyElement {
    let heading = h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(9.))
        .child(indicator(entry, cx))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_size(px(13.))
                .child(trace_entry_activity_label(entry)),
        )
        .when_some(data::target(entry), |this, target| {
            this.child(secondary(target, cx))
        })
        .into_any_element();
    disclosure(conversation, message_id, entry, heading, cx, view)
}

fn tool(
    conversation: &ConversationView,
    message_id: MessageId,
    entry: &AssistantTraceEntry,
    cx: &App,
    view: &Entity<ConversationView>,
) -> AnyElement {
    let heading = h_flex()
        .debug_selector(|| "assistant-activity-tool".to_owned())
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex_none()
                .text_size(px(13.))
                .child(data::tool_verb(entry)),
        )
        .when_some(data::target(entry), |this, target| {
            this.child(
                div()
                    .min_w_0()
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(secondary(target, cx)),
            )
        })
        .when_some(data::diff_stats(entry), |this, stats| {
            this.child(
                h_flex()
                    .gap(px(5.))
                    .flex_none()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(px(10.))
                    .child(
                        div()
                            .text_color(cx.theme().success)
                            .child(format!("+{}", stats.added)),
                    )
                    .child(
                        div()
                            .text_color(cx.theme().danger)
                            .child(format!("−{}", stats.removed)),
                    ),
            )
        })
        .into_any_element();
    disclosure(conversation, message_id, entry, heading, cx, view)
}

fn search(
    conversation: &ConversationView,
    message_id: MessageId,
    entry: &AssistantTraceEntry,
    cx: &App,
    view: &Entity<ConversationView>,
) -> AnyElement {
    let query = data::argument(entry, "query")
        .or_else(|| data::argument(entry, "path"))
        .unwrap_or_else(|| trace_entry_title(entry));
    let heading = h_flex()
        .w_full()
        .min_w_0()
        .gap(px(9.))
        .items_center()
        .child(
            Icon::new(IconName::Search)
                .size(px(13.))
                .text_color(cx.theme().muted_foreground),
        )
        .child(div().flex_1().min_w_0().child(secondary(query, cx)))
        .into_any_element();
    let matches = data::search_matches(entry);
    let results_key = format!("search-results:{}", entry.key);
    let expanded = conversation
        .trace_entry_overrides
        .get(&(message_id, results_key.clone()))
        .copied()
        .unwrap_or(false);
    let mut results = v_flex().w_full().min_w_0().gap(px(9.)).child(disclosure(
        conversation,
        message_id,
        entry,
        heading,
        cx,
        view,
    ));
    for (index, result) in matches
        .iter()
        .take(if expanded { matches.len() } else { 3 })
        .enumerate()
    {
        results = results.child(search_result(index, result, cx));
    }
    if matches.len() > 3 {
        let detail_view = view.clone();
        results = results.child(
            h_flex().w_full().child(
                Button::new(format!(
                    "assistant-search-more-{}-{}",
                    message_id.0, entry.key
                ))
                .debug_selector(|| "assistant-search-more".to_owned())
                .ghost()
                .xsmall()
                .compact()
                .px_0()
                .text_size(px(11.))
                .text_color(cx.theme().muted_foreground)
                .label(if expanded {
                    "Show less".to_owned()
                } else {
                    format!("+{} more", matches.len() - 3)
                })
                .accessibility_label(if expanded {
                    "Show fewer search results"
                } else {
                    "Show all search results"
                })
                .on_click(move |_, _, cx| {
                    detail_view.update(cx, |view, cx| {
                        view.trace_entry_overrides
                            .insert((message_id, results_key.clone()), !expanded);
                        view.remeasure_trace(message_id);
                        cx.notify();
                    });
                }),
            ),
        );
    }
    results.into_any_element()
}

fn search_result(index: usize, result: &data::SearchMatch, cx: &App) -> AnyElement {
    h_flex()
        .debug_selector(move || format!("assistant-activity-search-result-{index}"))
        .w_full()
        .min_w_0()
        .items_start()
        .gap(px(9.))
        .child(
            Icon::new(IconName::File)
                .size(px(13.))
                .text_color(cx.theme().muted_foreground.opacity(0.8)),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    h_flex()
                        .min_w_0()
                        .gap(px(6.))
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_size(px(12.))
                                .child(result.path.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(11.))
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(":{}", result.location)),
                        ),
                )
                .child(secondary(result.excerpt.clone(), cx)),
        )
        .into_any_element()
}

fn secondary(text: String, cx: &App) -> AnyElement {
    div()
        .min_w_0()
        .overflow_hidden()
        .text_ellipsis()
        .whitespace_nowrap()
        .text_size(px(12.))
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

fn disclosure(
    conversation: &ConversationView,
    message_id: MessageId,
    entry: &AssistantTraceEntry,
    heading: AnyElement,
    cx: &App,
    view: &Entity<ConversationView>,
) -> AnyElement {
    let open = conversation
        .trace_entry_overrides
        .get(&(message_id, entry.key.clone()))
        .copied()
        .unwrap_or(false);
    let entry_view = view.clone();
    let key = entry.key.clone();
    let status = entry.status;
    let title = trace_entry_title(entry);
    let heading = h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .child(div().flex_1().min_w_0().child(heading))
        .when(status != AssistantTraceStatus::Completed, |this| {
            this.child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(trace_status_color(status, cx))
                    .child(trace_status_label(status)),
            )
        });
    v_flex()
        .w_full()
        .min_w_0()
        .flex_none()
        .gap(px(6.))
        .child(
            Button::new(format!(
                "assistant-trace-entry-{}-{}",
                message_id.0, entry.key
            ))
            .ghost()
            .small()
            .compact()
            .w_full()
            .min_w_0()
            .h_auto()
            .px_0()
            .py(px(2.))
            .toggled(open)
            .accessibility_label(format!(
                "{} {title} details",
                if open { "Collapse" } else { "Expand" }
            ))
            .child(heading)
            .on_click(move |_, _, cx| {
                entry_view.update(cx, |view, cx| {
                    view.trace_entry_overrides
                        .insert((message_id, key.clone()), !open);
                    view.remeasure_trace(message_id);
                    cx.notify();
                });
            }),
        )
        .when(open, |this| {
            this.child(ConversationView::render_trace_entry_detail(
                message_id, entry, cx,
            ))
        })
        .into_any_element()
}
