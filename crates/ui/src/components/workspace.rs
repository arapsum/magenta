use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, div, prelude::FluentBuilder as _, px,
};
use magenta_core::ConversationMode;

use crate::app::{
    MainView,
    setup::{SetupItem, SetupItemState, SetupPrimaryAction, SetupReadiness},
};
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
    setup: Option<SetupReadiness>,
    compact_setup: bool,
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
        .gap(px(16.))
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
        .when_some(setup, |this, setup| {
            this.child(render_setup_panel(setup, compact_setup, cx))
        })
        .child(div().w_full().child(composer))
        .into_any_element()
}

fn render_setup_panel(
    setup: SetupReadiness,
    compact: bool,
    cx: &Context<'_, MainView>,
) -> AnyElement {
    let view = cx.entity();
    let ready = setup.chat_ready;
    let saving = setup.saving;
    let primary = primary_setup_action(&setup, cx);
    let dismiss_view = view.clone();
    let workspace_view = view.clone();
    let compact_work = SetupItem {
        title: "Work · optional",
        detail: if setup.file_tools.state == SetupItemState::Ready {
            "File tools are ready; commands are unavailable in this build.".to_owned()
        } else {
            "Work tools are unavailable in this build.".to_owned()
        },
        state: setup.file_tools.state,
    };

    v_flex()
        .id("setup-readiness-panel")
        .debug_selector(|| "setup-readiness-panel".into())
        .w_full()
        .gap(px(12.))
        .p(px(16.))
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().border.opacity(0.78))
        .bg(super::visual::surface(
            super::visual::SurfaceLevel::Raised,
            cx,
        ))
        .child(
            v_flex()
                .gap(px(3.))
                .when(!compact, |this| this.child(
                    div()
                        .text_size(px(15.))
                        .font_medium()
                        .child(if ready { "Ready to chat" } else { "Set up Magenta" }),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "Chat is for questions and attachments. Work adds project-scoped file tools.",
                        ),
                ),
        )
        .child(
            v_flex()
                .w_full()
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border.opacity(0.64))
                .child(render_setup_item("setup-account", &setup.account, None, cx))
                .child(render_setup_item("setup-models", &setup.models, None, cx))
                .child(
                    div()
                        .px(px(12.))
                        .pt(px(10.))
                        .pb(px(4.))
                        .border_t_1()
                        .border_color(cx.theme().border.opacity(0.64))
                        .text_size(px(10.))
                        .font_semibold()
                        .text_color(cx.theme().muted_foreground)
                        .child("WORK · OPTIONAL"),
                ))
                .child(render_setup_item(
                    "setup-workspace",
                    if compact { &compact_work } else { &setup.workspace },
                    Some(
                        Button::new("setup-choose-workspace")
                            .ghost()
                            .small()
                            .label("Choose…")
                            .on_click(move |_, window, cx| {
                                workspace_view.update(cx, |main, cx| {
                                    main.choose_setup_workspace(window, cx);
                                });
                            })
                            .into_any_element(),
                    ),
                    cx,
                ))
                .when(!compact, |this| {
                    this
                        .child(render_setup_item("setup-file-tools", &setup.file_tools, None, cx))
                        .child(render_setup_item("setup-commands", &setup.commands, None, cx))
                        .child(render_setup_item("setup-bubblewrap", &setup.bubblewrap, None, cx))
                }),
        )
        .when_some(setup.error, |this, error| {
            this.child(
                h_flex()
                    .items_start()
                    .gap(px(7.))
                    .text_size(px(11.))
                    .text_color(cx.theme().danger)
                    .child(Icon::new(IconName::CircleX).xsmall())
                    .child(format!("{} Reference: {}", error.message, error.code)),
            )
        })
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .child(
                    Button::new("setup-not-now")
                        .ghost()
                        .small()
                        .label("Not now")
                        .disabled(saving)
                        .on_click(move |_, window, cx| {
                            dismiss_view.update(cx, |main, cx| {
                                main.acknowledge_setup(true, window, cx);
                            });
                        }),
                )
                .child(primary),
        )
        .into_any_element()
}

fn primary_setup_action(setup: &SetupReadiness, cx: &Context<'_, MainView>) -> AnyElement {
    let view = cx.entity();
    let saving = setup.saving;
    let (label, disabled) = match setup.primary_action {
        SetupPrimaryAction::CheckingAccount => ("Checking…", true),
        SetupPrimaryAction::Connect => ("Connect ChatGPT…", false),
        SetupPrimaryAction::LoadingModels => ("Loading models…", true),
        SetupPrimaryAction::ReloadModels => ("Retry models", false),
        SetupPrimaryAction::Finish => ("Start chatting", false),
    };
    let action = setup.primary_action;

    Button::new("setup-primary-action")
        .primary()
        .small()
        .label(if saving { "Saving…" } else { label })
        .disabled(disabled || saving)
        .on_click(move |_, window, cx| {
            view.update(cx, |main, cx| match action {
                SetupPrimaryAction::Connect => main.begin_login(window, cx),
                SetupPrimaryAction::ReloadModels => main.load_models(window, cx),
                SetupPrimaryAction::Finish => main.acknowledge_setup(true, window, cx),
                SetupPrimaryAction::CheckingAccount | SetupPrimaryAction::LoadingModels => {}
            });
        })
        .into_any_element()
}

fn render_setup_item(
    id: &'static str,
    item: &SetupItem,
    action: Option<AnyElement>,
    cx: &Context<'_, MainView>,
) -> AnyElement {
    let (icon, color) = match item.state {
        SetupItemState::Loading => (IconName::LoaderCircle, cx.theme().muted_foreground),
        SetupItemState::Ready => (IconName::CircleCheck, cx.theme().success),
        SetupItemState::Attention => (IconName::TriangleAlert, cx.theme().warning),
        SetupItemState::Unavailable => (IconName::Info, cx.theme().muted_foreground),
    };

    h_flex()
        .id(id)
        .w_full()
        .min_h(px(38.))
        .items_center()
        .gap(px(9.))
        .px(px(12.))
        .py(px(6.))
        .child(Icon::new(icon).xsmall().text_color(color))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.))
                .child(div().text_size(px(12.)).font_medium().child(item.title))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(cx.theme().muted_foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(item.detail.clone()),
                ),
        )
        .when_some(action, |this, action| this.child(action))
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
    setup: Option<SetupReadiness>,
    compact_setup: bool,
    cx: &Context<'_, MainView>,
) -> AnyElement {
    let composer_state = composer.read(cx);
    let mode = composer_state.mode().clone();
    let mode_selector = composer_state.mode_selector(composer.clone(), cx);
    let content = render_landing_content(&mode, composer.clone(), setup, compact_setup, cx);

    render_landing_shell(mode_selector, content, cx)
}
