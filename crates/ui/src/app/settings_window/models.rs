use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};
use gpui_kit::{App, Entity, SharedString};
use magenta_application::resolve_generation_defaults;
use magenta_core::{ConversationMode, EffortLevel, GenerationPreference, ModelDescriptor};

use super::SettingsWindow;
use crate::settings;

const AUTOMATIC: &str = "automatic";

pub(super) fn models_page(
    view: &Entity<SettingsWindow>,
    models: &[ModelDescriptor],
    model_catalog_loaded: bool,
    cx: &App,
) -> SettingPage {
    let current = settings::current(cx);
    SettingPage::new("Models and defaults")
        .default_open(true)
        .icon(gpui_kit::component::Icon::new(
            gpui_kit::component::IconName::Settings,
        ))
        .description(
            "Choose independent Chat and Work defaults. Automatic follows the live catalog.",
        )
        .groups([
            generation_group(
                view,
                models,
                model_catalog_loaded,
                &current,
                &ConversationMode::Chat,
                "Chat",
            ),
            generation_group(
                view,
                models,
                model_catalog_loaded,
                &current,
                &ConversationMode::Agent,
                "Work",
            ),
        ])
}

fn generation_group(
    view: &Entity<SettingsWindow>,
    models: &[ModelDescriptor],
    model_catalog_loaded: bool,
    settings: &magenta_core::AppSettings,
    mode: &ConversationMode,
    title: &'static str,
) -> SettingGroup {
    let model_options = model_options(models, model_catalog_loaded, settings, mode);
    let effort_options = effort_options(models, settings, mode);
    let automatic = current_preference(settings, mode).is_none();

    SettingGroup::new()
        .title(title)
        .description("These values seed new conversations in this mode.")
        .items([
            model_item(view, models, mode, model_options),
            effort_item(view, models, mode, effort_options, automatic),
        ])
}

fn model_item(
    view: &Entity<SettingsWindow>,
    models: &[ModelDescriptor],
    mode: &ConversationMode,
    options: Vec<(SharedString, SharedString)>,
) -> SettingItem {
    let value_mode = mode.clone();
    let set_mode = mode.clone();
    let set_models = models.to_vec();
    let set_view = view.clone();
    SettingItem::new(
        "Model",
        SettingField::scrollable_dropdown(
            options,
            move |cx: &App| selected_model_value(&settings::current(cx), &value_mode),
            move |value: SharedString, cx: &mut App| {
                let Some(model) = set_models
                    .iter()
                    .find(|model| model_key(model) == value.as_ref())
                    .cloned()
                else {
                    if value.as_ref() != AUTOMATIC {
                        return;
                    }
                    set_view.update(cx, |view, cx| {
                        view.update_generation_settings(
                            |generation| set_preference(generation, &set_mode, None),
                            cx,
                        );
                    });
                    return;
                };

                set_view.update(cx, |view, cx| {
                    view.update_generation_settings(
                        |generation| {
                            set_preference(
                                generation,
                                &set_mode,
                                Some(GenerationPreference::new(
                                    model.provider.clone(),
                                    model.id.clone(),
                                    model.default_effort.clone(),
                                )),
                            );
                        },
                        cx,
                    );
                });
            },
        ),
    )
    .description("Automatic selects the highest-priority available model.")
}

fn effort_item(
    view: &Entity<SettingsWindow>,
    models: &[ModelDescriptor],
    mode: &ConversationMode,
    options: Vec<(SharedString, SharedString)>,
    automatic: bool,
) -> SettingItem {
    let value_models = models.to_vec();
    let value_mode = mode.clone();
    let set_mode = mode.clone();
    let set_view = view.clone();
    SettingItem::new(
        "Effort",
        SettingField::dropdown(
            options,
            move |cx: &App| {
                selected_effort_value(&settings::current(cx), &value_mode, &value_models)
            },
            move |value: SharedString, cx: &mut App| {
                let Some(effort) = EffortLevel::from_wire(value.as_ref()) else {
                    return;
                };
                set_view.update(cx, |view, cx| {
                    view.update_generation_settings(
                        |generation| {
                            if let Some(preference) = preference_mut(generation, &set_mode) {
                                preference.effort = effort;
                            }
                        },
                        cx,
                    );
                });
            },
        ),
    )
    .disabled(automatic)
    .description(if automatic {
        "Provider default effort (Automatic)."
    } else {
        "Use the selected model's supported effort values."
    })
}

fn model_options(
    models: &[ModelDescriptor],
    model_catalog_loaded: bool,
    settings: &magenta_core::AppSettings,
    mode: &ConversationMode,
) -> Vec<(SharedString, SharedString)> {
    let mut options = vec![(AUTOMATIC.into(), "Automatic".into())];
    let preference = current_preference(settings, mode);
    let effective_model = resolve_generation_defaults(mode.clone(), &settings.generation, models)
        .config
        .and_then(|configuration| {
            models
                .iter()
                .find(|model| {
                    if model.provider != configuration.provider {
                        return false;
                    }
                    model.id == configuration.model
                })
                .cloned()
        });
    let has_saved_model = preference.as_ref().is_some_and(|preference| {
        models.iter().any(|model| {
            if model.provider != preference.provider {
                return false;
            }
            model.id == preference.model
        })
    });

    if !has_saved_model && let Some(preference) = preference {
        let effective = effective_model
            .as_ref()
            .map_or_else(String::new, |model| format!(" → {}", model.display_name));
        let label = if model_catalog_loaded {
            format!("Unavailable · {}{}", preference.model.0, effective)
        } else {
            format!("{} (saved)", preference.model.0)
        };
        options.push((
            format!("{}::{}", preference.provider.0, preference.model.0).into(),
            label.into(),
        ));
    }

    options.extend(models.iter().map(|model| {
        let effective = effective_model.as_ref().is_some_and(|effective| {
            if effective.provider != model.provider {
                return false;
            }
            effective.id == model.id
        });
        let label = if effective && current_preference(settings, mode).is_some() {
            format!("{} (effective)", model.display_name)
        } else {
            model.display_name.clone()
        };
        (model_key(model).into(), label.into())
    }));
    options
}

fn effort_options(
    models: &[ModelDescriptor],
    settings: &magenta_core::AppSettings,
    mode: &ConversationMode,
) -> Vec<(SharedString, SharedString)> {
    let preference = settings.generation.for_mode(mode.clone()).cloned();
    let effective = resolve_generation_defaults(mode.clone(), &settings.generation, models).config;
    let effective_effort = effective
        .as_ref()
        .map(|configuration| configuration.effort.clone());
    let effective_model = effective.and_then(|configuration| {
        models
            .iter()
            .find(|model| {
                if model.provider != configuration.provider {
                    return false;
                }
                model.id == configuration.model
            })
            .cloned()
    });
    let mut efforts = effective_model
        .as_ref()
        .map_or_else(Vec::new, |model| model.supported_efforts.clone());

    if let Some(preference) = preference.as_ref()
        && !efforts.contains(&preference.effort)
    {
        efforts.insert(0, preference.effort.clone());
    }

    efforts
        .into_iter()
        .map(|effort| {
            let unavailable = preference
                .as_ref()
                .is_some_and(|preference| preference.effort == effort)
                && effective_model
                    .as_ref()
                    .is_some_and(|model| !model.supported_efforts.contains(&effort));
            let effective = effective_effort.as_ref().is_some_and(|effective| {
                *effective == effort
                    && preference
                        .as_ref()
                        .is_some_and(|preference| preference.effort != effort)
            });
            let label = if unavailable {
                format!("{} (unavailable)", effort.label())
            } else if effective {
                format!("{} (effective)", effort.label())
            } else {
                effort.label().to_owned()
            };
            (effort.wire_value().to_owned().into(), label.into())
        })
        .collect()
}

fn current_preference(
    settings: &magenta_core::AppSettings,
    mode: &ConversationMode,
) -> Option<GenerationPreference> {
    settings.generation.for_mode(mode.clone()).cloned()
}

fn selected_model_value(
    settings: &magenta_core::AppSettings,
    mode: &ConversationMode,
) -> SharedString {
    settings.generation.for_mode(mode.clone()).map_or_else(
        || AUTOMATIC.into(),
        |preference| model_key_from_preference(preference).into(),
    )
}

fn selected_effort_value(
    settings: &magenta_core::AppSettings,
    mode: &ConversationMode,
    models: &[ModelDescriptor],
) -> SharedString {
    if let Some(preference) = settings.generation.for_mode(mode.clone()) {
        return preference.effort.wire_value().to_owned().into();
    }

    resolve_generation_defaults(mode.clone(), &settings.generation, models)
        .config
        .map_or_else(
            || "none".into(),
            |configuration| configuration.effort.wire_value().to_owned().into(),
        )
}

fn model_key(model: &ModelDescriptor) -> String {
    format!("{}::{}", model.provider.0, model.id.0)
}

fn model_key_from_preference(preference: &GenerationPreference) -> String {
    format!("{}::{}", preference.provider.0, preference.model.0)
}

fn set_preference(
    settings: &mut magenta_core::GenerationSettings,
    mode: &ConversationMode,
    value: Option<GenerationPreference>,
) {
    match mode {
        ConversationMode::Chat => settings.chat = value,
        ConversationMode::Agent => settings.work = value,
    }
}

const fn preference_mut<'a>(
    settings: &'a mut magenta_core::GenerationSettings,
    mode: &ConversationMode,
) -> Option<&'a mut GenerationPreference> {
    match mode {
        ConversationMode::Chat => settings.chat.as_mut(),
        ConversationMode::Agent => settings.work.as_mut(),
    }
}
