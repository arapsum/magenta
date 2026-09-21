use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, div, px,
};
use magenta_core::ConversationMode;

use crate::app::MainView;
use crate::components::{prompt_input::PromptComposer, sidebar::SidebarView};

const fn landing_heading(mode: &ConversationMode) -> &'static str {
    match mode {
        ConversationMode::Chat => "Ready when you are",
        ConversationMode::Agent => "What should we work on?",
    }
}

fn render_landing_content(
    mode: &ConversationMode,
    composer: Entity<PromptComposer>,
    cx: &Context<'_, MainView>,
) -> AnyElement {
    let content_width = if *mode == ConversationMode::Agent {
        px(760.)
    } else {
        px(660.)
    };

    v_flex()
        .id("new-chat-start-content")
        .debug_selector(|| "new-chat-start-content".into())
        .w_full()
        .max_w(content_width)
        .items_center()
        .gap(px(20.))
        .child(
            div()
                .w_full()
                .text_center()
                .text_size(px(31.))
                .line_height(px(38.))
                .font_medium()
                .text_color(cx.theme().foreground.opacity(0.88))
                .child(landing_heading(mode)),
        )
        .child(div().w_full().child(composer))
        .into_any_element()
}

fn render_landing_shell(
    mode_selector: AnyElement,
    content: AnyElement,
    cx: &Context<'_, MainView>,
) -> AnyElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .bg(cx.theme().tokens.background.background)
        .text_color(cx.theme().foreground)
        .child(
            h_flex()
                .flex_none()
                .w_full()
                .h(px(54.))
                .items_center()
                .justify_center()
                .child(mode_selector),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .items_center()
                .justify_center()
                .px(px(32.))
                .pb(px(82.))
                .child(content),
        )
        .into_any_element()
}

#[must_use]
pub fn render(
    composer: &Entity<PromptComposer>,
    _sidebar: &Entity<SidebarView>,
    cx: &Context<'_, MainView>,
) -> AnyElement {
    let composer_state = composer.read(cx);
    let mode = composer_state.mode().clone();
    let mode_selector = composer_state.mode_selector(composer.clone(), cx);
    let content = render_landing_content(&mode, composer.clone(), cx);

    render_landing_shell(mode_selector, content, cx)
}
