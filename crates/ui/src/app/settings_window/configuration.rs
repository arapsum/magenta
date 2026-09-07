use gpui::{App, Entity, IntoElement, ParentElement as _, Styled as _, div, px};
use gpui_component::{
    ActiveTheme as _, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    setting::{SettingGroup, SettingItem},
    v_flex,
};

use super::SettingsWindow;

pub(super) fn configuration_group(view: &Entity<SettingsWindow>) -> SettingGroup {
    let open_view = view.clone();
    let reload_view = view.clone();
    let reset_view = view.clone();
    SettingGroup::new()
        .title("Configuration")
        .description("Magenta keeps your preferences in an editable local TOML file.")
        .item(SettingItem::render(move |_, _, cx| {
            let open_view = open_view.clone();
            let reload_view = reload_view.clone();
            let reset_view = reset_view.clone();

            v_flex()
                .w_full()
                .overflow_hidden()
                .rounded(px(10.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().secondary.opacity(0.35))
                .child(configuration_row(
                    "Settings file",
                    "Open the TOML file to edit preferences directly.",
                    Button::new("settings-open-file")
                        .primary()
                        .small()
                        .icon(IconName::ExternalLink)
                        .label("Open file")
                        .on_click(move |_, _, cx| {
                            open_view.update(cx, SettingsWindow::open_settings_file);
                        }),
                    cx,
                ))
                .child(configuration_divider(cx))
                .child(configuration_row(
                    "Reload from disk",
                    "Apply changes made outside Magenta.",
                    Button::new("settings-reload")
                        .outline()
                        .small()
                        .label("Reload")
                        .on_click(move |_, _, cx| {
                            reload_view.update(cx, SettingsWindow::reload);
                        }),
                    cx,
                ))
                .child(configuration_divider(cx))
                .child(configuration_row(
                    "Restore defaults",
                    "Backs up this file before restoring Magenta's defaults.",
                    Button::new("settings-reset")
                        .danger()
                        .small()
                        .label("Reset")
                        .on_click(move |_, _, cx| {
                            reset_view.update(cx, SettingsWindow::reset);
                        }),
                    cx,
                ))
                .into_any_element()
        }))
}

fn configuration_row(
    title: &'static str,
    description: &'static str,
    action: Button,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .px(px(12.))
        .py(px(10.))
        .child(
            v_flex()
                .min_w_0()
                .gap(px(3.))
                .child(div().font_medium().text_size(px(13.)).child(title))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child(description),
                ),
        )
        .child(action)
}

fn configuration_divider(cx: &App) -> impl IntoElement {
    div().h(px(1.)).w_full().bg(cx.theme().border)
}
