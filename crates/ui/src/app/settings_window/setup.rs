use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::Button,
    h_flex,
    setting::{SettingGroup, SettingItem, SettingPage},
    v_flex,
};
use gpui_kit::{
    App, Entity, IntoElement, ParentElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};

use super::super::setup::{SetupItem, SetupItemState, SetupPrimaryAction, SetupReadiness};
use super::{SettingsWindow, SettingsWindowEvent, settings_page_header_style};

pub(super) fn setup_page(
    view: &Entity<SettingsWindow>,
    setup: &SetupReadiness,
    _cx: &App,
) -> SettingPage {
    SettingPage::new("Setup & readiness")
        .resettable(false)
        .icon(Icon::new(IconName::CircleCheck))
        .description("Review provider, model, workspace, and local tool availability.")
        .header_style(&settings_page_header_style())
        .groups([
            chat_group(view, setup),
            work_group(view, setup),
            guidance_group(view, setup),
        ])
}

fn chat_group(view: &Entity<SettingsWindow>, setup: &SetupReadiness) -> SettingGroup {
    let account_action = match setup.primary_action {
        SetupPrimaryAction::Connect => Some((
            "setup-settings-connect",
            "Connect…",
            SettingsWindowEvent::BeginLogin,
        )),
        SetupPrimaryAction::CheckingAccount
        | SetupPrimaryAction::LoadingModels
        | SetupPrimaryAction::ReloadModels
        | SetupPrimaryAction::Finish => None,
    };
    let model_action = match setup.primary_action {
        SetupPrimaryAction::ReloadModels => Some((
            "setup-settings-reload-models",
            "Retry",
            SettingsWindowEvent::ReloadModels,
        )),
        SetupPrimaryAction::Connect
        | SetupPrimaryAction::CheckingAccount
        | SetupPrimaryAction::LoadingModels
        | SetupPrimaryAction::Finish => None,
    };
    let account = setup.account.clone();
    let models = setup.models.clone();

    SettingGroup::new()
        .title("Chat")
        .description("A connected account and usable model are required to start a conversation.")
        .items([
            status_setting(account, account_action, view),
            status_setting(models, model_action, view),
        ])
}

fn work_group(view: &Entity<SettingsWindow>, setup: &SetupReadiness) -> SettingGroup {
    let workspace = setup.workspace.clone();
    let file_tools = setup.file_tools.clone();
    let commands = setup.commands.clone();
    let bubblewrap = setup.bubblewrap.clone();
    let choose = (
        "setup-settings-choose-workspace",
        "Choose…",
        SettingsWindowEvent::ChooseWorkspace,
    );

    SettingGroup::new()
        .title("Work")
        .description("Work setup is optional and does not affect Chat readiness.")
        .items([
            status_setting(workspace, Some(choose), view),
            status_setting(file_tools, None, view),
            status_setting(commands, None, view),
            status_setting(bubblewrap, None, view),
        ])
}

fn guidance_group(view: &Entity<SettingsWindow>, setup: &SetupReadiness) -> SettingGroup {
    let show_view = view.clone();
    let acknowledged = setup.acknowledged;
    SettingGroup::new()
        .title("Guidance")
        .item(SettingItem::render(move |_, _, cx| {
            let show_view = show_view.clone();
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .gap(px(16.))
                .child(
                    v_flex()
                        .min_w_0()
                        .gap(px(3.))
                        .child(div().font_medium().child("New-chat setup"))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(cx.theme().muted_foreground)
                                .child(if acknowledged {
                                    "The first-run panel has been acknowledged."
                                } else {
                                    "The panel will appear on the next empty conversation."
                                }),
                        ),
                )
                .child(
                    Button::new("setup-settings-show-again")
                        .outline()
                        .small()
                        .label("Show on new chat")
                        .disabled(!acknowledged)
                        .on_click(move |_, _, cx| {
                            show_view.update(cx, |_, cx| {
                                cx.emit(SettingsWindowEvent::ShowSetupOnNewChat);
                            });
                        }),
                )
                .into_any_element()
        }))
}

fn status_setting(
    item: SetupItem,
    action: Option<(&'static str, &'static str, SettingsWindowEvent)>,
    view: &Entity<SettingsWindow>,
) -> SettingItem {
    let action_view = view.clone();
    SettingItem::render(move |_, _, cx| {
        let (icon, color) = match item.state {
            SetupItemState::Loading => (IconName::LoaderCircle, cx.theme().muted_foreground),
            SetupItemState::Ready => (IconName::CircleCheck, cx.theme().success),
            SetupItemState::Attention => (IconName::TriangleAlert, cx.theme().warning),
            SetupItemState::Unavailable => (IconName::Info, cx.theme().muted_foreground),
        };
        let action = action.map(|(id, label, event)| event_button(id, label, event, &action_view));
        h_flex()
            .w_full()
            .items_center()
            .gap(px(10.))
            .child(Icon::new(icon).small().text_color(color))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.))
                    .child(div().font_medium().child(item.title))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(cx.theme().muted_foreground)
                            .child(item.detail.clone()),
                    ),
            )
            .when_some(action, |this, action| this.child(action))
            .into_any_element()
    })
}

fn event_button(
    id: &'static str,
    label: &'static str,
    event: SettingsWindowEvent,
    view: &Entity<SettingsWindow>,
) -> Button {
    let event_view = view.clone();
    Button::new(id)
        .outline()
        .small()
        .label(label)
        .on_click(move |_, _, cx| {
            event_view.update(cx, |_, cx| cx.emit(event));
        })
}
