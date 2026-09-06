use gpui::{AnyElement, Context, Entity, IntoElement, ParentElement as _, Styled as _, div, px};
use gpui_component::{
    ActiveTheme as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};

use crate::app::MainView;
use crate::components::{
    prompt_input::PromptComposer,
    sidebar::{ConversationSummary, SidebarEvent, SidebarView},
};

fn render_recent_row(
    conversation: ConversationSummary,
    sidebar: &Entity<SidebarView>,
    cx: &Context<'_, MainView>,
) -> AnyElement {
    let id = conversation.id;
    let row_sidebar = sidebar.clone();

    Button::new(("workspace-recent", id.0))
        .ghost()
        .w_full()
        .h(px(58.))
        .px(px(12.))
        .rounded(px(8.))
        .child(
            h_flex()
                .w_full()
                .items_start()
                .gap(px(12.))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .items_start()
                        .gap(px(2.))
                        .child(
                            div()
                                .w_full()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(14.))
                                .text_color(cx.theme().foreground)
                                .child(conversation.title),
                        )
                        .child(
                            div()
                                .w_full()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(12.))
                                .line_height(px(16.))
                                .text_color(cx.theme().muted_foreground)
                                .child(conversation.preview),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(cx.theme().muted_foreground)
                        .child(conversation.updated),
                ),
        )
        .on_click(move |_, _, cx| {
            row_sidebar.update(cx, |_, cx| cx.emit(SidebarEvent::OpenConversation(id)));
        })
        .into_any_element()
}

fn render_recent_rows(
    conversations: Vec<ConversationSummary>,
    sidebar: &Entity<SidebarView>,
    cx: &Context<'_, MainView>,
) -> AnyElement {
    let mut rows = v_flex().w_full().gap(px(3.));
    for conversation in conversations {
        rows = rows.child(render_recent_row(conversation, sidebar, cx));
    }
    rows.into_any_element()
}

#[must_use]
pub fn render(
    composer: Entity<PromptComposer>,
    sidebar: &Entity<SidebarView>,
    cx: &Context<'_, MainView>,
) -> AnyElement {
    let show_recent = sidebar.read(cx).history_available();
    let recent = if show_recent {
        sidebar.read(cx).recent_conversations()
    } else {
        Vec::new()
    };
    let recent_rows = render_recent_rows(recent, sidebar, cx);

    let mut copy = v_flex()
        .w_full()
        .max_w(px(648.))
        .items_start()
        .gap(px(8.))
        .child(
            div()
                .text_size(px(28.))
                .line_height(px(34.))
                .font_semibold()
                .child("Where were we?"),
        )
        .child(
            div()
                .text_size(px(13.))
                .line_height(px(20.))
                .text_color(cx.theme().muted_foreground)
                .child("Nothing here yet. Ask anything, or pick up a thread."),
        );
    if show_recent {
        copy = copy
            .child(
                div()
                    .mt(px(30.))
                    .text_size(px(10.))
                    .font_medium()
                    .text_color(cx.theme().muted_foreground)
                    .child("CONTINUE"),
            )
            .child(recent_rows);
    }

    div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .overflow_hidden()
        .bg(cx.theme().tokens.background.background)
        .text_color(cx.theme().foreground)
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .items_center()
                .justify_center()
                .px(px(28.))
                .child(copy),
        )
        .child(
            div()
                .flex_none()
                .w_full()
                .px(px(24.))
                .pb(px(20.))
                .child(div().w_full().max_w(px(608.)).mx_auto().child(composer)),
        )
        .into_any_element()
}
