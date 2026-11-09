use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement as _,
    v_flex,
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

mod starters;

const fn landing_heading(mode: &ConversationMode) -> &'static str {
    match mode {
        ConversationMode::Chat => "What’s on your mind?",
        ConversationMode::Agent => "What should we work on?",
    }
}

fn render_landing_content(
    mode: &ConversationMode,
    composer: &Entity<PromptComposer>,
    setup: Option<SetupReadiness>,
    compact_setup: bool,
    cx: &Context<'_, MainView>,
) -> AnyElement {
    let content_width = if *mode == ConversationMode::Agent {
        px(760.)
    } else {
        px(660.)
    };
    let show_starters = setup.is_none() && !composer.read(cx).has_draft(cx);
    let has_setup = setup.is_some();

    v_flex()
        .id("new-chat-start-content")
        .debug_selector(|| "new-chat-start-content".into())
        .w_full()
        .max_w(content_width)
        .gap(px(if has_setup { 16. } else { 24. }))
        .child(
            v_flex()
                .w_full()
                .gap(px(9.))
                .child(
                    div()
                        .text_size(px(if compact_setup { 28. } else { 34. }))
                        .line_height(px(42.))
                        .font_medium()
                        .text_color(cx.theme().foreground)
                        .child(landing_heading(mode)),
                )
                .when(!compact_setup, |this| {
                    this.child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(21.))
                            .text_color(cx.theme().muted_foreground)
                            .child(if *mode == ConversationMode::Agent {
                                "Bring a project. We’ll work through the details together."
                            } else {
                                "A question, an idea, a fresh way to see things. Start anywhere."
                            }),
                    )
                }),
        )
        .when_some(setup, |this, setup| {
            this.child(render_setup_panel(&setup, compact_setup, cx))
        })
        .child(div().w_full().child(composer.clone()))
        .when(show_starters, |this| {
            this.child(starters::render(mode, composer, cx))
        })
        .into_any_element()
}

fn render_setup_panel(
    setup: &SetupReadiness,
    compact: bool,
    cx: &Context<'_, MainView>,
) -> AnyElement {
    let primary = primary_setup_action(setup, cx);
    let dismiss_view = cx.entity();
    let workspace_view = dismiss_view.clone();
    let compact_work = compact_work_item(&setup.file_tools);
    let checks = v_flex()
        .w_full()
        .rounded(px(10.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.7))
        .child(render_setup_item("setup-account", &setup.account, None, cx))
        .child(render_setup_item("setup-models", &setup.models, None, cx))
        .child(
            div()
                .px(px(12.))
                .pt(px(12.))
                .pb(px(4.))
                .border_t_1()
                .border_color(cx.theme().border.opacity(0.7))
                .text_size(px(10.))
                .font_medium()
                .text_color(cx.theme().muted_foreground)
                .child("WORK · OPTIONAL"),
        )
        .child(render_setup_item(
            "setup-workspace",
            if compact {
                &compact_work
            } else {
                &setup.workspace
            },
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
            this.child(render_setup_item(
                "setup-file-tools",
                &setup.file_tools,
                None,
                cx,
            ))
            .child(render_setup_item(
                "setup-commands",
                &setup.commands,
                None,
                cx,
            ))
            .child(render_setup_item(
                "setup-bubblewrap",
                &setup.bubblewrap,
                None,
                cx,
            ))
        });

    v_flex()
        .id("setup-readiness-panel")
        .debug_selector(|| "setup-readiness-panel".into())
        .w_full()
        .gap(px(14.))
        .p(px(18.))
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().border)
        .bg(super::visual::surface(
            super::visual::SurfaceLevel::Raised,
            cx,
        ))
        .when(!compact, |this| {
            this.child(render_setup_intro(setup.chat_ready, cx))
        })
        .child(checks)
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
        .child(render_setup_actions(setup.saving, primary, dismiss_view))
        .into_any_element()
}

fn render_setup_intro(ready: bool, cx: &gpui_kit::App) -> AnyElement {
    v_flex()
        .gap(px(5.))
        .child(div().text_size(px(15.)).font_medium().child(if ready {
            "You’re ready to chat"
        } else {
            "Welcome to Magenta"
        }))
        .child(
            div()
                .text_size(px(12.))
                .line_height(px(18.))
                .text_color(cx.theme().muted_foreground)
                .child(if ready {
                    "Your account and model are ready. Add a project whenever you need file tools."
                } else {
                    "Connect ChatGPT to get started. You can add a project later."
                }),
        )
        .into_any_element()
}

fn compact_work_item(file_tools: &SetupItem) -> SetupItem {
    SetupItem {
        title: "Project tools",
        detail: if file_tools.state == SetupItemState::Ready {
            "File tools are ready; commands are unavailable in this build.".to_owned()
        } else {
            "Work tools are unavailable in this build.".to_owned()
        },
        state: file_tools.state,
    }
}

fn render_setup_actions(saving: bool, primary: AnyElement, view: Entity<MainView>) -> AnyElement {
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
                    view.update(cx, |main, cx| {
                        main.acknowledge_setup(true, window, cx);
                    });
                }),
        )
        .child(primary)
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
        .when_some(action, gpui_kit::ParentElement::child)
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
                .h(px(58.))
                .items_center()
                .justify_center()
                .child(mode_selector),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_y_scrollbar()
                .child(
                    v_flex()
                        .w_full()
                        .min_h_full()
                        .items_center()
                        .justify_center()
                        .px(px(32.))
                        .pt(px(24.))
                        .pb(px(42.))
                        .child(content),
                ),
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
    let content = render_landing_content(&mode, composer, setup, compact_setup, cx);

    render_landing_shell(mode_selector, content, cx)
}
