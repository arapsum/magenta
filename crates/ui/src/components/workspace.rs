use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    AnyElement, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, div, px,
};
use magenta_core::Project;

use crate::app::MainView;
use crate::components::{
    prompt_input::PromptComposer,
    provider_icon,
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
        .h(px(52.))
        .px(px(12.))
        .rounded(px(8.))
        .child(
            h_flex()
                .w_full()
                .items_start()
                .gap(px(10.))
                .child(
                    provider_icon(Some(&conversation.provider))
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .items_start()
                        .gap(px(3.))
                        .child(
                            div()
                                .w_full()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(13.))
                                .font_medium()
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
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_size(px(11.))
                        .text_color(cx.theme().muted_foreground.opacity(0.84))
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

fn render_landing_content(
    project: Option<&Project>,
    recent: Vec<ConversationSummary>,
    show_recent: bool,
    composer: Entity<PromptComposer>,
    sidebar: &Entity<SidebarView>,
    cx: &Context<'_, MainView>,
) -> AnyElement {
    let has_recent = !recent.is_empty();
    let recent_rows = render_recent_rows(recent, sidebar, cx);
    let (heading, description, section_label) = project.as_ref().map_or_else(
        || {
            if has_recent {
                (
                    "What would you like to work on?".to_owned(),
                    "Start something new, or pick up a recent conversation.".to_owned(),
                    "Continue",
                )
            } else {
                (
                    "What can I help with?".to_owned(),
                    "Ask a question, explore an idea, or work through a problem.".to_owned(),
                    "Continue",
                )
            }
        },
        |project| {
            (
                format!("Ready to work in {}", project.name),
                "Ask the agent to inspect, create, or edit files in this workspace.".to_owned(),
                "Project threads",
            )
        },
    );

    let mut content = v_flex()
        .id("new-chat-start-content")
        .debug_selector(|| "new-chat-start-content".into())
        .w_full()
        .max_w(px(672.))
        .items_start()
        .gap(px(9.))
        .child(
            h_flex()
                .items_center()
                .gap(px(9.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(30.))
                        .rounded(px(9.))
                        .bg(cx.theme().sidebar_accent.opacity(0.72))
                        .text_color(cx.theme().foreground)
                        .child(
                            Icon::new(if project.is_some() {
                                IconName::FolderOpen
                            } else {
                                IconName::Bot
                            })
                            .small(),
                        ),
                )
                .child(
                    div()
                        .text_size(px(31.))
                        .line_height(px(37.))
                        .font_semibold()
                        .child(heading),
                ),
        )
        .child(
            div()
                .text_size(px(14.))
                .line_height(px(21.))
                .text_color(cx.theme().muted_foreground)
                .child(description),
        );
    if let Some(project) = project {
        content = content.child(
            h_flex()
                .gap(px(7.))
                .items_center()
                .mt(px(6.))
                .text_size(px(11.))
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::Folder).xsmall())
                .child(project.root.display().to_string()),
        );
    }
    content = content.child(div().w_full().mt(px(20.)).child(composer));
    if (project.is_some() && has_recent) || (project.is_none() && show_recent) {
        content = content
            .child(
                div()
                    .mt(px(24.))
                    .text_size(px(10.))
                    .font_medium()
                    .text_color(cx.theme().muted_foreground)
                    .child(section_label),
            )
            .child(recent_rows);
    }
    content.into_any_element()
}

fn render_landing_shell(content: AnyElement, cx: &Context<'_, MainView>) -> AnyElement {
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .overflow_y_scrollbar()
        .bg(cx.theme().tokens.background.background)
        .text_color(cx.theme().foreground)
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .items_center()
                .justify_center()
                .px(px(32.))
                .py(px(36.))
                .child(content),
        )
        .into_any_element()
}

#[must_use]
pub fn render(
    composer: Entity<PromptComposer>,
    sidebar: &Entity<SidebarView>,
    cx: &Context<'_, MainView>,
) -> AnyElement {
    let sidebar_state = sidebar.read(cx);
    let project = sidebar_state.active_project_details();
    let show_recent = sidebar_state.history_available();
    let recent = project.as_ref().map_or_else(
        || {
            if show_recent {
                sidebar_state.recent_conversations()
            } else {
                Vec::new()
            }
        },
        |project| sidebar_state.project_conversations_for(&project.root),
    );
    let content =
        render_landing_content(project.as_ref(), recent, show_recent, composer, sidebar, cx);
    render_landing_shell(content, cx)
}
