use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    AnyElement, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, div, linear_color_stop, linear_gradient, px,
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
        .h(px(68.))
        .px(px(14.))
        .rounded(px(12.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.72))
        .bg(cx.theme().popover.opacity(0.68))
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(36.))
                        .flex_none()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(cx.theme().primary.opacity(0.18))
                        .bg(cx.theme().accent.opacity(0.72))
                        .text_color(cx.theme().primary)
                        .child(provider_icon(Some(&conversation.provider)).xsmall()),
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
                                .text_size(px(14.))
                                .font_semibold()
                                .text_color(cx.theme().foreground)
                                .child(conversation.title),
                        )
                        .child(
                            div()
                                .w_full()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(12.5))
                                .line_height(px(16.))
                                .text_color(cx.theme().muted_foreground)
                                .child(conversation.preview),
                        ),
                )
                .child(
                    h_flex()
                        .flex_none()
                        .gap(px(8.))
                        .text_color(cx.theme().muted_foreground.opacity(0.84))
                        .child(
                            div()
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_size(px(11.))
                                .child(conversation.updated),
                        )
                        .child(Icon::new(IconName::ChevronRight).xsmall()),
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
    let mut rows = v_flex().w_full().gap(px(8.));
    for conversation in conversations {
        rows = rows.child(render_recent_row(conversation, sidebar, cx));
    }
    rows.into_any_element()
}

fn landing_copy(project: Option<&Project>, has_recent: bool) -> (String, String, &'static str) {
    project.map_or_else(
        || {
            if has_recent {
                (
                    "What would you like to work on?".to_owned(),
                    "Start a conversation, explore an idea, or continue where you left off."
                        .to_owned(),
                    "Continue",
                )
            } else {
                (
                    "What can I help with?".to_owned(),
                    "Think through a question, shape an idea, or build something new.".to_owned(),
                    "Continue",
                )
            }
        },
        |project| {
            (
                format!("Ready to work in {}", project.name),
                "Ask Magenta to inspect, create, or edit files in this workspace.".to_owned(),
                "Project threads",
            )
        },
    )
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
    let (heading, description, section_label) = landing_copy(project, has_recent);

    let eyebrow = if project.is_some() {
        "Active workspace"
    } else {
        "Your AI workspace"
    };
    let icon_background = linear_gradient(
        135.,
        linear_color_stop(cx.theme().primary, 0.),
        linear_color_stop(cx.theme().yellow, 1.),
    );

    let mut content = v_flex()
        .id("new-chat-start-content")
        .debug_selector(|| "new-chat-start-content".into())
        .w_full()
        .max_w(px(880.))
        .items_start()
        .gap(px(12.))
        .child(
            h_flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(11.))
                .font_medium()
                .text_color(cx.theme().primary)
                .child(Icon::new(IconName::Bot).xsmall())
                .child(eyebrow),
        )
        .child(
            h_flex()
                .items_center()
                .gap(px(14.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(46.))
                        .rounded(px(14.))
                        .bg(icon_background)
                        .text_color(cx.theme().primary_foreground)
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
                        .text_size(px(38.))
                        .line_height(px(44.))
                        .font_semibold()
                        .child(heading),
                ),
        )
        .child(
            div()
                .text_size(px(16.))
                .line_height(px(24.))
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
    content = content.child(div().w_full().mt(px(26.)).child(composer));
    if (project.is_some() && has_recent) || (project.is_none() && show_recent) {
        content = content
            .child(
                div()
                    .mt(px(28.))
                    .text_size(px(12.))
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child(section_label),
            )
            .child(recent_rows);
    }
    content.into_any_element()
}

fn render_landing_shell(
    mode_selector: AnyElement,
    content: AnyElement,
    cx: &Context<'_, MainView>,
) -> AnyElement {
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
        .child(super::visual::ambient_field(cx))
        .child(
            h_flex()
                .relative()
                .flex_none()
                .w_full()
                .h(px(54.))
                .items_center()
                .justify_center()
                .border_b_1()
                .border_color(cx.theme().border.opacity(0.52))
                .bg(cx.theme().tokens.background.background.opacity(0.9))
                .child(mode_selector),
        )
        .child(
            div()
                .relative()
                .flex()
                .flex_1()
                .min_h_0()
                .justify_center()
                .px(px(48.))
                .pt(px(48.))
                .pb(px(64.))
                .child(content),
        )
        .into_any_element()
}

#[must_use]
pub fn render(
    composer: &Entity<PromptComposer>,
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
    let content = render_landing_content(
        project.as_ref(),
        recent,
        show_recent,
        composer.clone(),
        sidebar,
        cx,
    );
    let mode_selector = composer.read(cx).mode_selector(composer.clone(), cx);

    render_landing_shell(mode_selector, content, cx)
}
