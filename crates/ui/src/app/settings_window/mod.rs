mod appearance;
mod configuration;

use std::{cell::RefCell, rc::Rc, sync::Arc};

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Task, Window, WindowHandle, WindowOptions, div,
    prelude::FluentBuilder as _, px, size,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::Button,
    h_flex,
    setting::{SettingGroup, SettingItem, SettingPage, Settings},
    v_flex,
};
use magenta_core::{AppSettings, ProviderAccount, SettingsStore};

use self::{
    appearance::{installed_font_options, mathematics_group, theme_group, typography_group},
    configuration::configuration_group,
};
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
