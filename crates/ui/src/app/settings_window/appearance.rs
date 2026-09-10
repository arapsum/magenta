use gpui_kit::component::setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem};
use gpui_kit::{App, Entity, SharedString};
use magenta_core::{AppSettings, AppearanceMode, FontChoice};

use super::SettingsWindow;
use crate::settings;

pub(super) fn theme_group(view: &Entity<SettingsWindow>) -> SettingGroup {
    let view = view.clone();
    SettingGroup::new().title("Theme").item(
        SettingItem::new(
            "Color mode",
            SettingField::dropdown(
                vec![
                    ("system".into(), "System".into()),
                    ("light".into(), "Light".into()),
                    ("dark".into(), "Dark".into()),
                ],
                |cx: &App| appearance_value(&settings::current(cx).appearance),
                move |value: SharedString, cx: &mut App| {
                    view.update(cx, |view, cx| {
                        view.update_settings(
                            |settings| {
                                settings.appearance = match value.as_ref() {
                                    "system" => AppearanceMode::System,
                                    "light" => AppearanceMode::Light,
                                    _ => AppearanceMode::Dark,
                                };
                            },
                            cx,
                        );
                    });
                },
            )
            .default_value(appearance_value(&AppSettings::default().appearance)),
        )
        .description("Use the system appearance or keep Magenta light or dark."),
    )
}

pub(super) fn typography_group(
    view: &Entity<SettingsWindow>,
    fonts: Vec<(SharedString, SharedString)>,
) -> SettingGroup {
    SettingGroup::new().title("Typography").items([
        ui_font_item(
            view,
            with_system_font(fonts.clone(), "system-ui", "System UI"),
        ),
        ui_size_item(view),
        monospace_font_item(
            view,
            with_system_font(fonts, "system-monospace", "System monospace"),
        ),
        monospace_size_item(view),
    ])
}

fn ui_font_item(
    view: &Entity<SettingsWindow>,
    options: Vec<(SharedString, SharedString)>,
) -> SettingItem {
    let view = view.clone();
    font_item(
        "UI font",
        "The font used throughout Magenta's interface.",
        options,
        |settings| settings.typography.ui_font.clone(),
        move |value, cx| {
            view.update(cx, |view, cx| {
                view.update_settings(
                    |settings| {
                        settings.typography.ui_font =
                            FontChoice::from_config_value(value.as_ref(), FontChoice::SystemUi);
                    },
                    cx,
                );
            });
        },
    )
}

fn ui_size_item(view: &Entity<SettingsWindow>) -> SettingItem {
    let view = view.clone();
    size_item(
        "UI font size",
        "The interface text size in pixels.",
        |settings| settings.typography.ui_size,
        move |value, cx| {
            view.update(cx, |view, cx| {
                view.update_settings(|settings| settings.typography.ui_size = value, cx);
            });
        },
    )
}

fn monospace_font_item(
    view: &Entity<SettingsWindow>,
    options: Vec<(SharedString, SharedString)>,
) -> SettingItem {
    let view = view.clone();
    font_item(
        "Monospace font",
        "Used by inline code, code blocks, and technical labels.",
        options,
        |settings| settings.typography.monospace_font.clone(),
        move |value, cx| {
            view.update(cx, |view, cx| {
                view.update_settings(
                    |settings| {
                        settings.typography.monospace_font = FontChoice::from_config_value(
                            value.as_ref(),
                            FontChoice::SystemMonospace,
                        );
                    },
                    cx,
                );
            });
        },
    )
}

fn monospace_size_item(view: &Entity<SettingsWindow>) -> SettingItem {
    let view = view.clone();
    size_item(
        "Monospace font size",
        "The code and technical-label size in pixels.",
        |settings| settings.typography.monospace_size,
        move |value, cx| {
            view.update(cx, |view, cx| {
                view.update_settings(|settings| settings.typography.monospace_size = value, cx);
            });
        },
    )
}

pub(super) fn mathematics_group(view: &Entity<SettingsWindow>) -> SettingGroup {
    SettingGroup::new().title("Mathematics").items([
        math_font_item(view),
        inline_math_size_item(view),
        display_math_size_item(view),
    ])
}

fn math_font_item(view: &Entity<SettingsWindow>) -> SettingItem {
    let view = view.clone();
    SettingItem::new(
        "Mathematical font",
        SettingField::dropdown(
            vec![
                ("default".into(), "KaTeX Default".into()),
                ("roman".into(), "KaTeX Roman".into()),
                ("sans-serif".into(), "KaTeX Sans-serif".into()),
                ("typewriter".into(), "KaTeX Typewriter".into()),
            ],
            |cx: &App| {
                settings::current(cx)
                    .typography
                    .math_font
                    .as_config_value()
                    .into()
            },
            move |value: SharedString, cx: &mut App| {
                view.update(cx, |view, cx| {
                    view.update_settings(
                        |settings| {
                            settings.typography.math_font =
                                magenta_core::MathFontStyle::from_config_value(value.as_ref());
                        },
                        cx,
                    );
                });
            },
        )
        .default_value("default"),
    )
    .description("Uses metric-compatible KaTeX font styles.")
}

fn inline_math_size_item(view: &Entity<SettingsWindow>) -> SettingItem {
    let view = view.clone();
    size_item(
        "Inline mathematics size",
        "The size used for formulas inside text.",
        |settings| settings.typography.inline_math_size,
        move |value, cx| {
            view.update(cx, |view, cx| {
                view.update_settings(|settings| settings.typography.inline_math_size = value, cx);
            });
        },
    )
}

fn display_math_size_item(view: &Entity<SettingsWindow>) -> SettingItem {
    let view = view.clone();
    size_item(
        "Display mathematics size",
        "The size used for standalone formulas.",
        |settings| settings.typography.display_math_size,
        move |value, cx| {
            view.update(cx, |view, cx| {
                view.update_settings(|settings| settings.typography.display_math_size = value, cx);
            });
        },
    )
}

pub(super) fn installed_font_options(cx: &App) -> Vec<(SharedString, SharedString)> {
    cx.text_system()
        .all_font_names()
        .into_iter()
        .map(|name| (name.clone().into(), name.into()))
        .collect()
}

fn with_system_font(
    mut options: Vec<(SharedString, SharedString)>,
    value: &'static str,
    label: &'static str,
) -> Vec<(SharedString, SharedString)> {
    options.insert(0, (value.into(), label.into()));
    options
}

fn appearance_value(value: &AppearanceMode) -> SharedString {
    match value {
        AppearanceMode::System => "system".into(),
        AppearanceMode::Light => "light".into(),
        AppearanceMode::Dark => "dark".into(),
    }
}

fn font_item<Get, Set>(
    title: &'static str,
    description: &'static str,
    options: Vec<(SharedString, SharedString)>,
    get: Get,
    set: Set,
) -> SettingItem
where
    Get: Fn(&AppSettings) -> FontChoice + 'static,
    Set: Fn(SharedString, &mut App) + 'static,
{
    SettingItem::new(
        title,
        SettingField::scrollable_dropdown(
            options,
            move |cx: &App| get(&settings::current(cx)).as_config_value().into(),
            set,
        ),
    )
    .description(description)
}

fn size_item<Get, Set>(
    title: &'static str,
    description: &'static str,
    get: Get,
    set: Set,
) -> SettingItem
where
    Get: Fn(&AppSettings) -> u16 + 'static,
    Set: Fn(u16, &mut App) + 'static,
{
    SettingItem::new(
        title,
        SettingField::number_input(
            NumberFieldOptions {
                min: 8.,
                max: 72.,
                step: 1.,
            },
            move |cx: &App| f64::from(get(&settings::current(cx))),
            move |value: f64, cx: &mut App| set(pixel_size(value), cx),
        ),
    )
    .description(description)
}

fn pixel_size(value: f64) -> u16 {
    value
        .round()
        .clamp(8., 72.)
        .to_string()
        .parse()
        .unwrap_or(8)
}
