use super::*;

impl MainView {
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
                        main.set_account_state(AccountState::Failed(error.source.to_string()), cx);
                    }
                }
                cx.notify();
            });
        }));
    }

    fn load_models(&mut self, window: &Window, cx: &Context<'_, Self>) {
        self.model_task.take();
        let catalog = Arc::clone(&self.model_catalog);
        self.model_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = catalog.models().await;
            _ = view.update_in(window, |main, _, cx| {
                main.model_task = None;
                match result {
                    Ok(models) => {
                        main.composer.update(cx, |composer, cx| {
                            composer.set_models(models, cx);
                        });
                    }
                    Err(error) => {
                        tracing::warn!(
                            error = ?error,
                            operation = "models.load",
                            "could not load OpenAI models"
                        );
                        main.set_account_state(AccountState::Failed(error.source.to_string()), cx);
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
                        main.set_account_state(AccountState::Failed(error.source.to_string()), cx);
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
                        main.set_account_state(AccountState::Failed(error.source.to_string()), cx);
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
        self.composer.update(cx, |composer, cx| {
            composer.set_models(Vec::new(), cx);
        });
        self.account_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = authenticator.sign_out().await;
            _ = view.update_in(window, |main, _, cx| {
                main.account_task = None;
                if let Err(error) = result {
                    main.set_account_state(AccountState::Failed(error.source.to_string()), cx);
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
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_account(account, cx);
        });
        if let Some(settings_view) = self.settings_view.as_ref() {
            let state = self.account_settings_state();
            settings_view.update(cx, |settings, cx| settings.set_account(state, cx));
        }
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
                error: Some(error.clone()),
                ..Default::default()
            },
            AccountState::SignedOut => AccountSettingsState::default(),
        }
    }

    pub(crate) fn load_settings(&mut self, window: &Window, cx: &Context<'_, Self>) {
        let store = Arc::clone(&self.settings_store);
        self.settings_load_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = store.load().await;
            _ = view.update_in(window, |main, _, cx| {
                main.settings_load_task = None;
                match result {
                    Ok(value) => crate::settings::replace(value, cx),
                    Err(error) => tracing::warn!(
                        error = %error.source,
                        operation = "settings.load",
                        "could not load settings; using defaults"
                    ),
                }
                cx.notify();
            });
        }));
    }

    pub(crate) fn open_settings(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if let Some(handle) = self.settings_window
            && handle.is_active(cx).is_some()
        {
            _ = handle.update(cx, |_, settings_window, _| {
                settings_window.activate_window();
            });
            return;
        }

        let account = self.account_settings_state();
        match SettingsWindow::open(Arc::clone(&self.settings_store), account, cx) {
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
