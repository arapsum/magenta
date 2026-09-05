use gpui::{AnyElement, Context, Entity, IntoElement, ParentElement as _, Styled as _, div, px};
use gpui_component::{
    ActiveTheme as _, IconName, StyledExt as _,
    button::{Button, ButtonVariants as _},
    v_flex,
};

use crate::app::MainView;
use crate::components::{
    prompt_input::PromptComposer,
    sidebar::{SidebarEvent, SidebarView},
};

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
    let sidebar_view = sidebar.clone();
    let mut recent_rows = v_flex().w_full().gap(px(3.));
    for (id, title) in recent {
        let row_sidebar = sidebar_view.clone();
        recent_rows = recent_rows.child(
            Button::new(("workspace-recent", id.0))
                .ghost()
                .w_full()
                .h(px(36.))
                .px(px(10.))
                .rounded(px(7.))
                .icon(IconName::ChevronRight)
                .label(title)
                .on_click(move |_, _, cx| {
                    row_sidebar.update(cx, |_, cx| cx.emit(SidebarEvent::OpenConversation(id)));
                }),
        );
    }

    let mut copy = v_flex()
        .w_full()
        .max_w(px(560.))
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
                .child("Continue a conversation or start a new one."),
        );
    if show_recent {
        copy = copy
            .child(
                div()
                    .mt(px(12.))
                    .text_size(px(10.))
                    .font_medium()
                    .text_color(cx.theme().muted_foreground)
                    .child("Recent conversations"),
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
