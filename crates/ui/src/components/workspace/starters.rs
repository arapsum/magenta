use super::*;
use gpui_kit::assets::IconName;

pub(super) fn render(
    mode: &ConversationMode,
    composer: &Entity<PromptComposer>,
    cx: &Context<'_, MainView>,
) -> AnyElement {
    let prompts = if *mode == ConversationMode::Agent {
        [
            (
                IconName::FolderOpen,
                "Understand a project",
                "Map the structure",
                "Explore this project and explain its structure, main entry points, and how the pieces fit together.",
            ),
            (
                IconName::GitBranch,
                "Review the changes",
                "Get a second opinion",
                "Review the changes in this project. Look for bugs, edge cases, and anything that needs a closer look.",
            ),
            (
                IconName::Search,
                "Investigate an issue",
                "Find a path forward",
                "Help me investigate an issue in this project. Ask me what is going wrong before you start.",
            ),
        ]
    } else {
        [
            (
                IconName::Lightbulb,
                "Explore an idea",
                "See where it leads",
                "Help me explore an idea. Start by asking what I have in mind.",
            ),
            (
                IconName::BookOpen,
                "Learn something",
                "Make it click",
                "Help me understand a concept. Ask me what I want to learn and what I already know.",
            ),
            (
                IconName::ListTodo,
                "Make a plan",
                "Turn thoughts into steps",
                "Help me make a practical plan. Ask me about my goal and any constraints first.",
            ),
        ]
    };

    h_flex()
        .w_full()
        .flex_wrap()
        .items_stretch()
        .gap(px(10.))
        .children(
            prompts
                .into_iter()
                .enumerate()
                .map(|(index, (icon, title, detail, prompt))| {
                    let composer = composer.clone();
                    Button::new(("starter", index))
                        .ghost()
                        .accessibility_label(title)
                        .flex_1()
                        .min_w(px(160.))
                        .h(px(92.))
                        .p(px(13.))
                        .rounded(px(12.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .text_color(cx.theme().foreground)
                        .child(
                            v_flex()
                                .w_full()
                                .min_w_0()
                                .gap(px(9.))
                                .child(
                                    Icon::new(icon)
                                        .xsmall()
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .child(
                                    v_flex()
                                        .gap(px(3.))
                                        .child(div().text_size(px(12.)).font_medium().child(title))
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(cx.theme().muted_foreground)
                                                .child(detail),
                                        ),
                                ),
                        )
                        .on_click(move |_, window, cx| {
                            composer.update(cx, |composer, cx| {
                                if !composer.has_draft(cx) {
                                    composer.prepare_continuation(prompt, window, cx);
                                }
                            });
                        })
                }),
        )
        .into_any_element()
}
