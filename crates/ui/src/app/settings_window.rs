use std::{cell::RefCell, rc::Rc, sync::Arc};

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Task, Window, WindowHandle,
    WindowOptions, div, prelude::FluentBuilder as _, px, size,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    v_flex,
};
use magenta_core::{AppSettings, AppearanceMode, FontChoice, ProviderAccount, SettingsStore};

use crate::{components::titlebar, settings};

#[derive(Clone, Debug)]
pub enum SettingsWindowEvent {
    BeginLogin,
    SignOut,
    TypographyChanged,
}

#[derive(Clone, Debug, Default)]
pub struct AccountSettingsState {
    pub account: Option<ProviderAccount>,
    pub waiting: bool,
    pub error: Option<String>,
}

pub struct SettingsWindow {
    store: Arc<dyn SettingsStore>,
    account: AccountSettingsState,
    save_task: Option<Task<()>>,
    feedback: Option<String>,
}

impl EventEmitter<SettingsWindowEvent> for SettingsWindow {}

impl SettingsWindow {
    pub fn open(
        store: Arc<dyn SettingsStore>,
        account: AccountSettingsState,
        cx: &mut App,
    ) -> anyhow::Result<(WindowHandle<gpui_component::Root>, Entity<Self>)> {
        let slot = Rc::new(RefCell::new(None));
        let view_slot = Rc::clone(&slot);
        let handle = cx.open_window(settings_window_options(cx), move |window, cx| {
            let view = cx.new(|_| Self {
                store,
                account,
                save_task: None,
                feedback: None,
            });
            view_slot.replace(Some(view.clone()));
            cx.new(|cx| gpui_component::Root::new(view, window, cx))
        })?;
        let view = slot
            .borrow_mut()
            .take()
            .expect("settings window view should be created with its window");
        Ok((handle, view))
    }

    pub fn set_account(&mut self, account: AccountSettingsState, cx: &mut Context<'_, Self>) {
        self.account = account;
        cx.notify();
    }

    fn update_settings(
        &mut self,
        update: impl FnOnce(&mut AppSettings),
        cx: &mut Context<'_, Self>,
    ) {
        let saved = settings::update(update, cx);
        self.persist(saved, cx);
        cx.emit(SettingsWindowEvent::TypographyChanged);
        cx.notify();
    }

    fn persist(&mut self, value: AppSettings, cx: &Context<'_, Self>) {
        self.save_task.take();
        let store = Arc::clone(&self.store);
        self.save_task = Some(cx.spawn(async move |view, cx| {
            let result = store.save(value).await;
            _ = view.update(cx, |view, cx| {
                view.save_task = None;
                view.feedback = result.err().map(|error| error.source.to_string());
                cx.notify();
            });
        }));
    }

    fn reload(&mut self, cx: &mut Context<'_, Self>) {
        let store = Arc::clone(&self.store);
        self.save_task.take();
        self.save_task = Some(cx.spawn(async move |view, cx| {
            let result = store.load().await;
            _ = view.update(cx, |view, cx| {
                view.save_task = None;
                match result {
                    Ok(value) => {
                        settings::replace(value, cx);
                        view.feedback = Some("Reloaded settings from disk.".to_owned());
                        cx.emit(SettingsWindowEvent::TypographyChanged);
                    }
                    Err(error) => view.feedback = Some(error.source.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn open_settings_file(&mut self, cx: &mut Context<'_, Self>) {
        let store = Arc::clone(&self.store);
        let path = store.path();
        let value = settings::current(cx);
        self.save_task.take();
        self.save_task = Some(cx.spawn(async move |view, cx| {
            let result = store.save(value).await;
            _ = view.update(cx, |view, cx| {
                view.save_task = None;
                match result {
                    Ok(()) => {
                        cx.open_with_system(&path);
                        view.feedback = Some("Opened settings file.".to_owned());
                    }
                    Err(error) => view.feedback = Some(error.source.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn reset(&mut self, cx: &mut Context<'_, Self>) {
        let store = Arc::clone(&self.store);
        self.save_task.take();
        self.save_task = Some(cx.spawn(async move |view, cx| {
            let result = store.reset().await;
            _ = view.update(cx, |view, cx| {
                view.save_task = None;
                match result {
                    Ok(value) => {
                        settings::replace(value, cx);
                        view.feedback = Some("Restored default settings.".to_owned());
                        cx.emit(SettingsWindowEvent::TypographyChanged);
                    }
                    Err(error) => view.feedback = Some(error.source.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn settings_pages(&self, _: &mut Window, cx: &Context<'_, Self>) -> Vec<SettingPage> {
        let view = cx.entity();
        vec![Self::appearance_page(&view, cx), self.providers_page(&view)]
    }

    fn appearance_page(view: &Entity<Self>, cx: &App) -> SettingPage {
        let fonts = installed_font_options(cx);
        let header_style = settings_page_header_style();
        SettingPage::new("Appearance")
            .default_open(true)
            .icon(Icon::new(IconName::Settings))
            .description("Change Magenta's appearance and typography.")
            .header_style(&header_style)
            .groups([
                theme_group(view),
                typography_group(view, fonts),
                mathematics_group(view),
                configuration_group(view),
            ])
    }

    fn providers_page(&self, view: &Entity<Self>) -> SettingPage {
        let header_style = settings_page_header_style();
        let (status, detail, action, event) = match &self.account.account {
            Some(account) => (
                "Connected",
                account
                    .email
                    .clone()
                    .unwrap_or_else(|| "ChatGPT account".to_owned()),
                "Disconnect",
                SettingsWindowEvent::SignOut,
            ),
            None if self.account.waiting => (
                "Waiting for sign-in",
                "Finish signing in through your browser.".to_owned(),
                "Waiting…",
                SettingsWindowEvent::BeginLogin,
            ),
            None => (
                "Not connected",
                self.account
                    .error
                    .clone()
                    .unwrap_or_else(|| "Connect ChatGPT to use your subscription.".to_owned()),
                "Connect ChatGPT",
                SettingsWindowEvent::BeginLogin,
            ),
        };
        let disabled = self.account.waiting;
        let event_view = view.clone();
        SettingPage::new("Providers")
            .icon(Icon::new(IconName::Bot))
            .description("Connect the accounts that supply models to Magenta.")
            .header_style(&header_style)
            .group(
                SettingGroup::new().title("OpenAI").item(
                    SettingItem::render(move |options, _, cx| {
                        let label = action;
                        let detail = detail.clone();
                        let event_view = event_view.clone();
                        let event = event.clone();
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap(px(16.))
                            .child(
                                v_flex()
                                    .gap(px(3.))
                                    .child(div().font_medium().child(status))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(cx.theme().muted_foreground)
                                            .child(detail),
                                    ),
                            )
                            .child(
                                Button::new("settings-openai-account")
                                    .outline()
                                    .with_size(options.size())
                                    .disabled(disabled)
                                    .label(label)
                                    .on_click(move |_, _, cx| {
                                        event_view.update(cx, |_, cx| {
                                            cx.emit(event.clone());
                                        });
                                    }),
                            )
                            .into_any_element()
                    })
                    .description("Uses ChatGPT sign-in; credentials remain in the system keyring."),
                ),
            )
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let pages = self.settings_pages(window, cx);
        v_flex()
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .when_some(self.feedback.clone(), |this, feedback| {
                this.child(
                    div()
                        .px(px(16.))
                        .py(px(8.))
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(feedback),
                )
            })
            .child(
                div().flex().flex_1().min_h_0().child(
                    Settings::new("magenta-settings")
                        .sidebar_width(px(220.))
                        .sidebar_size_range(px(220.)..px(220.))
                        .pages(pages),
                ),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left(px(220.))
                    .right_0()
                    .h(px(32.))
                    .child(titlebar::render_minimize_close(
                        settings_titlebar_content(),
                        |window, _| window.remove_window(),
                    )),
            )
    }
}

fn settings_page_header_style() -> gpui::StyleRefinement {
    gpui::StyleRefinement::default().pt(px(48.))
}

fn settings_titlebar_content() -> impl IntoElement {
    h_flex()
        .id("settings-titlebar-content")
        .h_full()
        .items_center()
        .px(px(12.))
        .text_size(px(13.))
        .font_medium()
        .child("Settings")
}

fn theme_group(view: &Entity<SettingsWindow>) -> SettingGroup {
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

fn typography_group(
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

fn mathematics_group(view: &Entity<SettingsWindow>) -> SettingGroup {
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

fn configuration_group(view: &Entity<SettingsWindow>) -> SettingGroup {
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

fn settings_window_options(cx: &App) -> WindowOptions {
    let bounds = gpui::Bounds::centered(None, size(px(900.), px(650.)), cx);
    let mut options = gpui_component::TitleBar::window_options();
    options.window_bounds = Some(gpui::WindowBounds::Windowed(bounds));
    options.window_min_size = Some(size(px(640.), px(480.)));
    if let Some(titlebar) = options.titlebar.as_mut() {
        titlebar.title = Some("Magenta Settings".into());
    }
    options
}

fn installed_font_options(cx: &App) -> Vec<(SharedString, SharedString)> {
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
