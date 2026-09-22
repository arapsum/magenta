use super::*;

impl MainView {
    pub(crate) fn refresh_composer_blocking_error(&mut self, cx: &mut Context<'_, Self>) {
        let error = match &self.account_state {
            AccountState::Failed(error) => Some(*error),
            AccountState::SignedOut => Some(crate::ErrorPresentation {
                code: "MAG-ACCOUNT-SIGNED-OUT",
                severity: crate::ErrorSeverity::Warning,
                title: "Connect a provider account",
                message: "Connect ChatGPT to load models and send a response.",
            }),
            AccountState::Connected(_) => match self.model_catalog_state {
                ModelCatalogState::Failed(error) => Some(error),
                ModelCatalogState::NotLoaded
                | ModelCatalogState::Loading
                | ModelCatalogState::Loaded => None,
            },
            AccountState::Restoring | AccountState::WaitingForBrowser => None,
        };
        self.composer.update(cx, |composer, cx| {
            composer.set_account_error(error, cx);
        });
    }

    pub(crate) fn restore_account(&mut self, window: &Window, cx: &Context<'_, Self>) {
        let authenticator = Arc::clone(&self.authenticator);
        self.account_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = authenticator.restore().await;
            _ = view.update_in(window, |main, window, cx| {
                main.account_task = None;
                match result {
                    Ok(Some(account)) => {
                        main.set_account_state(AccountState::Connected(account), cx);
                        main.load_models(window, cx);
                    }
                    Ok(None) => {
                        main.set_account_state(AccountState::SignedOut, cx);
                    }
                    Err(error) => {
                        tracing::warn!(
                            error = ?error,
                            operation = "account.restore",
                            "could not restore the OpenAI account"
                        );
                        main.set_account_state(
                            AccountState::Failed(crate::provider_error_presentation(error.kind)),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
    }

    pub(crate) fn load_models(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        self.model_task.take();
        self.model_catalog_state = ModelCatalogState::Loading;
        self.refresh_composer_blocking_error(cx);
        self.sync_settings_setup(cx);
        let catalog = Arc::clone(&self.model_catalog);
        self.model_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = catalog.models().await;
            _ = view.update_in(window, |main, window, cx| {
                main.model_task = None;
                match result {
                    Ok(models) => {
                        main.set_model_catalog(models, window, cx);
                    }
                    Err(error) => {
                        tracing::warn!(
                            error = ?error,
                            operation = "models.load",
                            "could not load OpenAI models"
                        );
                        main.model_catalog_state = ModelCatalogState::Failed(
                            crate::provider_error_presentation(error.kind),
                        );
                        main.refresh_composer_blocking_error(cx);
                        main.sync_settings_setup(cx);
                    }
                }
                cx.notify();
            });
        }));
    }

    pub(crate) fn begin_login(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if self.account_task.is_some() {
            return;
        }

        self.set_account_state(AccountState::WaitingForBrowser, cx);
        let authenticator = Arc::clone(&self.authenticator);
        self.account_task = Some(cx.spawn_in(window, async move |view, window| {
            let session = match authenticator.begin_login().await {
                Ok(session) => session,
                Err(error) => {
                    _ = view.update_in(window, |main, _, cx| {
                        main.account_task = None;
                        main.set_account_state(
                            AccountState::Failed(crate::provider_error_presentation(error.kind)),
                            cx,
                        );
                        cx.notify();
                    });
                    return;
                }
            };
            let url = session.authorization_url.clone();
            _ = view.update_in(window, |_, _, cx| {
                cx.open_url(&url);
            });
            let result = session.completion.await;
            _ = view.update_in(window, |main, window, cx| {
                main.account_task = None;
                match result {
                    Ok(account) => {
                        main.set_account_state(AccountState::Connected(account), cx);
                        main.load_models(window, cx);
                    }
                    Err(error) => {
                        main.set_account_state(
                            AccountState::Failed(crate::provider_error_presentation(error.kind)),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(crate) fn sign_out(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if self.account_task.is_some() {
            return;
        }

        self.model_task.take();
        let authenticator = Arc::clone(&self.authenticator);
        self.set_account_state(AccountState::SignedOut, cx);
        self.clear_model_catalog(cx);
        self.account_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = authenticator.sign_out().await;
            _ = view.update_in(window, |main, _, cx| {
                main.account_task = None;
                if let Err(error) = result {
                    main.set_account_state(
                        AccountState::Failed(crate::provider_error_presentation(error.kind)),
                        cx,
                    );
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn set_account_state(&mut self, state: AccountState, cx: &mut Context<'_, Self>) {
        let account = match &state {
            AccountState::Connected(account) => Some(account.clone()),
            AccountState::Restoring
            | AccountState::SignedOut
            | AccountState::WaitingForBrowser
            | AccountState::Failed(_) => None,
        };
        self.account_state = state;
        self.refresh_composer_blocking_error(cx);
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_account(account, cx);
        });
        if let Some(settings_view) = self.settings_view.as_ref() {
            let state = self.account_settings_state();
            settings_view.update(cx, |settings, cx| settings.set_account(state, cx));
        }
        self.sync_settings_setup(cx);
    }

    fn account_settings_state(&self) -> AccountSettingsState {
        match &self.account_state {
            AccountState::Restoring | AccountState::WaitingForBrowser => AccountSettingsState {
                waiting: true,
                ..Default::default()
            },
            AccountState::Connected(account) => AccountSettingsState {
                account: Some(account.clone()),
                ..Default::default()
            },
            AccountState::Failed(error) => AccountSettingsState {
                error: Some(*error),
                ..Default::default()
            },
            AccountState::SignedOut => AccountSettingsState::default(),
        }
    }

    pub(crate) fn load_settings(&mut self, window: &Window, cx: &Context<'_, Self>) {
        let store = Arc::clone(&self.settings_store);
        self.settings_load_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = store.load().await;
            _ = view.update_in(window, |main, window, cx| {
                main.settings_load_task = None;
                match result {
                    Ok(value) => crate::settings::replace(value, cx),
                    Err(error) => {
                        tracing::warn!(
                            error = %error.source,
                            operation = "settings.load",
                            "could not load settings; using defaults"
                        );
                    }
                }
                main.apply_generation_defaults(window, cx);
                main.sync_settings_setup(cx);
                cx.notify();
            });
        }));
    }

    pub(crate) fn open_settings(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        self.open_settings_destination(SettingsDestination::Appearance, window, cx);
    }

    pub(crate) fn open_setup(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        self.open_settings_destination(SettingsDestination::Setup, window, cx);
    }

    fn open_settings_destination(
        &mut self,
        destination: SettingsDestination,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(handle) = self.settings_window
            && handle.is_active(cx).is_some()
        {
            if let Some(settings_view) = self.settings_view.as_ref() {
                settings_view.update(cx, |settings, cx| {
                    settings.set_destination(destination, cx);
                });
            }
            _ = handle.update(cx, |_, settings_window, _| {
                settings_window.activate_window();
            });
            return;
        }

        let account = self.account_settings_state();
        let setup = self.setup_readiness(cx);
        match SettingsWindow::open(
            Arc::clone(&self.settings_store),
            account,
            self.models.clone(),
            self.model_catalog_state.is_loaded(),
            setup,
            destination,
            cx,
        ) {
            Ok((handle, settings_view)) => {
                let subscription = cx.subscribe_in(
                    &settings_view,
                    window,
                    |main, _, event: &SettingsWindowEvent, window, cx| match event {
                        SettingsWindowEvent::BeginLogin => main.begin_login(window, cx),
                        SettingsWindowEvent::SignOut => main.sign_out(window, cx),
                        SettingsWindowEvent::TypographyChanged => {
                            main.conversation.update(cx, |conversation, cx| {
                                conversation.refresh_math_typography(cx);
                            });
                        }
                        SettingsWindowEvent::GenerationDefaultsChanged => {
                            main.apply_generation_defaults(window, cx);
                            main.sync_settings_setup(cx);
                        }
                        SettingsWindowEvent::ReloadModels => main.load_models(window, cx),
                        SettingsWindowEvent::ChooseWorkspace => {
                            main.choose_setup_workspace(window, cx);
                        }
                        SettingsWindowEvent::ShowSetupOnNewChat => {
                            main.show_setup_on_new_chat(window, cx);
                        }
                    },
                );
                self.settings_window = Some(handle);
                self.settings_view = Some(settings_view);
                self.settings_subscription = Some(subscription);
            }
            Err(error) => tracing::error!(
                ?error,
                operation = "settings.open",
                "could not open settings"
            ),
        }
    }
}
