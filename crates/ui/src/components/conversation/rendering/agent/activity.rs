mod data;
mod loader;
mod rows;

use super::*;

#[cfg(test)]
#[path = "../../../../../test/components/conversation/rendering/activity.rs"]
mod tests;

impl ConversationView {
    pub(crate) fn select_trace_tab(
        &mut self,
        message_id: MessageId,
        tab: TraceTab,
        cx: &mut Context<'_, Self>,
    ) {
        self.trace_tab_selections.insert(message_id, tab);
        self.remeasure_trace(message_id);
        cx.notify();
    }

    fn remeasure_trace(&self, message_id: MessageId) {
        if let Some(index) = self
            .messages
            .iter()
            .position(|item| item.message.id == message_id)
        {
            self.list_state.remeasure_items(index..index + 1);
        }
    }
}

pub(super) fn render(
    conversation: &ConversationView,
    message: &Message,
    cx: &App,
    view: &Entity<ConversationView>,
) -> AnyElement {
    let has_entries = !message.assistant_trace.entries.is_empty();
    if !has_entries && !ConversationView::trace_is_active(message) {
        return div().into_any_element();
    }

    let open = has_entries && conversation.trace_is_open(message);
    let selected = conversation
        .trace_tab_selections
        .get(&message.id)
        .copied()
        .unwrap_or_default();
    v_flex()
        .id(("assistant-activity", message.id.0))
        .w_full()
        .min_w_0()
        .gap(px(8.))
        .child(header(conversation, message, open, selected, cx, view))
        .when(open, |this| {
            this.child(
                div()
                    .id(("assistant-trace-viewport", message.id.0))
                    .debug_selector(|| "assistant-trace-viewport".to_owned())
                    .w_full()
                    .max_h(TRACE_VIEWPORT_MAX_HEIGHT)
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .min_w_0()
                            .ml(px(7.))
                            .pl(px(18.))
                            .pr(px(8.))
                            .py(px(4.))
                            .border_l_1()
                            .border_color(cx.theme().border.opacity(0.8))
                            .child(rows::render(conversation, message, selected, cx, view)),
                    ),
            )
            .child(tabs(message.id, selected, cx, view))
        })
        .into_any_element()
}

fn header(
    conversation: &ConversationView,
    message: &Message,
    open: bool,
    selected: TraceTab,
    cx: &App,
    view: &Entity<ConversationView>,
) -> AnyElement {
    let message_id = message.id;
    let active = ConversationView::trace_is_active(message)
        && !conversation.trace_is_final_answer_started(message_id);
    let has_entries = !message.assistant_trace.entries.is_empty();
    let label = header_label(conversation, message, selected, active);
    let mut leading = h_flex().min_w_0().items_center().gap(px(9.));
    if active {
        leading = leading.child(loader::render(
            message_id,
            label,
            conversation
                .generation_progress
                .as_ref()
                .filter(|progress| progress.message_id == message_id),
            cx,
        ));
    } else {
        leading = leading.child(terminal_icon(message.status, cx)).child(
            div()
                .text_size(px(13.))
                .text_color(cx.theme().muted_foreground)
                .child(label),
        );
    }
    if has_entries {
        leading = leading.child(
            Icon::new(if open {
                IconName::ChevronUp
            } else {
                IconName::ChevronDown
            })
            .size(px(12.))
            .text_color(cx.theme().muted_foreground.opacity(0.75)),
        );
    }
    let disclosure_view = view.clone();
    let disclosure = Button::new(("assistant-trace-disclosure", message_id.0))
        .ghost()
        .small()
        .compact()
        .min_w_0()
        .tab_stop(has_entries)
        .px_0()
        .toggled(open)
        .accessibility_label(if !has_entries {
            "Response in progress"
        } else if open {
            "Collapse activity"
        } else {
            "Expand activity"
        })
        .child(leading)
        .on_click(move |_, _, cx| {
            if has_entries {
                disclosure_view.update(cx, |view, cx| {
                    view.set_trace_disclosure_override(message_id, !open, cx);
                    view.remeasure_trace(message_id);
                });
            }
        });
    let stop_view = view.clone();
    h_flex()
        .debug_selector(|| "assistant-trace-header".to_owned())
        .w_full()
        .min_w_0()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .child(disclosure)
        .when(active, |this| {
            this.child(
                Button::new(("stop-assistant-trace", message_id.0))
                    .debug_selector(|| "assistant-activity-stop".to_owned())
                    .ghost()
                    .xsmall()
                    .compact()
                    .label("Stop")
                    .accessibility_label("Stop response")
                    .on_click(move |_, _, cx| {
                        stop_view.update(cx, |view, cx| view.request_stop(message_id, cx));
                    }),
            )
        })
        .into_any_element()
}

fn terminal_icon(status: MessageStatus, cx: &App) -> AnyElement {
    Icon::new(
        if matches!(status, MessageStatus::Failed | MessageStatus::Stopped) {
            gpui_kit::assets::IconName::CircleX
        } else {
            gpui_kit::assets::IconName::Sparkles
        },
    )
    .xsmall()
    .text_color(if status == MessageStatus::Failed {
        cx.theme().danger
    } else {
        cx.theme().muted_foreground.opacity(0.8)
    })
    .into_any_element()
}

fn header_label(
    conversation: &ConversationView,
    message: &Message,
    selected: TraceTab,
    active: bool,
) -> String {
    if active {
        return data::active_label(message, conversation.generation_progress.as_ref()).to_owned();
    }
    // Terminal states always take precedence over the selected detail view.
    if message.status == MessageStatus::Complete {
        let entries = data::entries(message, selected);
        if !entries.is_empty() {
            match selected {
                TraceTab::Search => return "Searched the project".to_owned(),
                TraceTab::Tools => {
                    return format!(
                        "{} tool call{}",
                        entries.len(),
                        if entries.len() == 1 { "" } else { "s" }
                    );
                }
                TraceTab::Steps | TraceTab::Reasoning => {}
            }
        }
    }
    let mode = conversation
        .conversation
        .as_ref()
        .map_or(&ConversationMode::Chat, |conversation| &conversation.mode);
    trace_header_label(message, false, conversation.trace_elapsed(message), mode)
}

fn tabs(
    message_id: MessageId,
    selected: TraceTab,
    cx: &App,
    view: &Entity<ConversationView>,
) -> AnyElement {
    let mut group = h_flex()
        .ml(px(23.))
        .mt(px(8.))
        .mb(px(2.))
        .p(px(2.))
        .gap(px(1.))
        .rounded_full()
        .bg(cx.theme().muted.opacity(0.5));
    for tab in TraceTab::ALL {
        let tab_view = view.clone();
        let is_selected = selected == tab;
        group = group.child(
            Button::new(format!(
                "assistant-activity-tab-{}-{}",
                message_id.0,
                tab.label()
            ))
            .debug_selector(move || format!("assistant-activity-tab-{}", tab.label()))
            .ghost()
            .xsmall()
            .compact()
            .rounded(px(20.))
            .px(px(9.))
            .h(px(22.))
            .toggled(is_selected)
            .text_size(px(11.))
            .text_color(if is_selected {
                cx.theme().foreground
            } else {
                cx.theme().muted_foreground
            })
            .when(is_selected, |this| {
                this.bg(cx.theme().background)
                    .border_1()
                    .border_color(cx.theme().border)
            })
            .label(tab.label())
            .accessibility_label(format!("Show {} activity", tab.label().to_lowercase()))
            .on_click(move |_, _, cx| {
                tab_view.update(cx, |view, cx| view.select_trace_tab(message_id, tab, cx));
            }),
        );
    }
    h_flex().w_full().min_w_0().child(group).into_any_element()
}
