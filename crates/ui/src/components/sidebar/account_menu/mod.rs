use super::account_dropdown::AccountDropdown;
use super::*;
use crate::components::provider_icon;
use magenta_core::ProviderId;

impl SidebarView {
    pub(super) fn render_footer(&self, view: &Entity<Self>, cx: &App) -> AnyElement {
        let details = profile_details(self.account.as_ref());
        let connected = self.account.is_some();
        let is_dark = cx.theme().is_dark();
        let menu_view = view.clone();
        let menu_details = details.clone();
        let ProfileDetails {
            name,
            detail,
            initial,
            tooltip,
            ..
        } = details;
        let trigger = Button::new("sidebar-account-menu-trigger")
            .ghost()
            .accessibility_id("sidebar-account-menu")
            .tooltip(tooltip)
            .w_full()
            .h(px(56.))
            .items_center()
            .gap(px(8.))
            .p(px(8.))
            .rounded(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(28.))
                    .flex_none()
                    .rounded(px(8.))
                    .bg(cx.theme().sidebar_accent)
                    .text_color(cx.theme().sidebar_accent_foreground)
                    .border_1()
                    .border_color(cx.theme().sidebar_border)
                    .text_size(px(12.))
                    .font_medium()
                    .child(initial),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_medium()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(name),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(cx.theme().muted_foreground)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(detail),
                    ),
            )
            .child(
                Icon::new(IconName::ChevronsUpDown)
                    .xsmall()
                    .text_color(cx.theme().muted_foreground),
            );

        v_flex()
            .w_full()
            .pt(px(8.))
            .border_t_1()
            .border_color(cx.theme().sidebar_border)
            .child(AccountDropdown::new(
                "sidebar-account-dropdown",
                trigger,
                move |popover, _window, cx| {
                    Self::render_account_menu(
                        &menu_view,
                        &menu_details,
                        connected,
                        is_dark,
                        &popover,
                        cx,
                    )
                },
            ))
            .into_any_element()
    }

    fn render_account_menu(
        view: &Entity<Self>,
        details: &ProfileDetails,
        connected: bool,
        is_dark: bool,
        popover: &Entity<PopoverState>,
        cx: &App,
    ) -> AnyElement {
        let mut surface = v_flex()
            .w(ACCOUNT_MENU_WIDTH)
            .popover_style(cx)
            .rounded(px(13.))
            .p(px(4.))
            .child(Self::account_menu_identity(details, cx))
            .child(Self::account_menu_separator(cx))
            .child(Self::account_menu_actions(view, popover, is_dark, cx));

        if connected {
            surface = surface
                .child(Self::account_menu_separator(cx))
                .child(Self::account_plan_button(view, popover, details, cx));
        }

        surface
            .child(Self::account_menu_separator(cx))
            .child(Self::account_session_button(view, popover, connected, cx))
            .into_any_element()
    }

    fn account_menu_identity(details: &ProfileDetails, cx: &App) -> impl IntoElement {
        let detail = details.email.as_ref().unwrap_or(&details.detail).clone();

        h_flex()
            .w_full()
            .h(px(58.))
            .items_center()
            .gap(px(10.))
            .px(px(8.))
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(32.))
                    .flex_none()
                    .rounded(px(9.))
                    .bg(cx.theme().sidebar_accent)
                    .border_1()
                    .border_color(cx.theme().border)
                    .text_size(px(13.))
                    .font_medium()
                    .child(details.initial.clone()),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(13.))
                            .font_medium()
                            .child(details.name.clone()),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(11.))
                            .text_color(cx.theme().muted_foreground)
                            .child(detail),
                    ),
            )
    }

    fn account_menu_actions(
        view: &Entity<Self>,
        popover: &Entity<PopoverState>,
        is_dark: bool,
        cx: &App,
    ) -> impl IntoElement {
        let settings_view = view.clone();
        let settings_popover = popover.clone();
        let settings = Self::account_menu_button(
            "account-settings",
            "Settings",
            IconName::Settings,
            None,
            false,
            cx,
        )
        .on_click(move |_, window, cx| {
            Self::emit_account_event(
                &settings_view,
                &settings_popover,
                SidebarEvent::OpenSettings,
                window,
                cx,
            );
        });

        let theme_view = view.clone();
        let theme_popover = popover.clone();
        let appearance = Self::account_menu_button(
            "account-appearance",
            "Appearance",
            if is_dark {
                IconName::Sun
            } else {
                IconName::Moon
            },
            Some(if is_dark { "Dark" } else { "Light" }.into()),
            false,
            cx,
        )
        .on_click(move |_, window, cx| {
            Self::emit_account_event(
                &theme_view,
                &theme_popover,
                SidebarEvent::ToggleTheme,
                window,
                cx,
            );
        });

        v_flex()
            .w_full()
            .py(px(4.))
            .child(settings)
            .child(appearance)
            .child(Self::account_menu_button(
                "account-keyboard-shortcuts",
                "Keyboard shortcuts",
                IconName::SquareTerminal,
                Some("Soon".into()),
                true,
                cx,
            ))
            .child(Self::account_menu_button(
                "account-help-feedback",
                "Help & feedback",
                IconName::Info,
                Some("Soon".into()),
                true,
                cx,
            ))
    }

    fn account_plan_button(
        view: &Entity<Self>,
        popover: &Entity<PopoverState>,
        details: &ProfileDetails,
        cx: &App,
    ) -> impl IntoElement {
        let plan = details
            .plan
            .as_ref()
            .map_or_else(|| "Account plan".to_owned(), |plan| format!("{plan} plan"));
        let plan_view = view.clone();
        let plan_popover = popover.clone();

        v_flex().w_full().py(px(5.)).child(
            Button::new("account-plan")
                .ghost()
                .accessibility_id("manage-account-plan")
                .w_full()
                .h(px(54.))
                .px(px(9.))
                .rounded(px(8.))
                .bg(cx.theme().sidebar_accent.opacity(0.45))
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .gap(px(8.))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(2.))
                                .child(div().text_size(px(12.)).font_medium().child(plan))
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(cx.theme().muted_foreground)
                                        .child("OpenAI account"),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(11.))
                                .font_medium()
                                .child("Manage"),
                        ),
                )
                .on_click(move |_, window, cx| {
                    Self::emit_account_event(
                        &plan_view,
                        &plan_popover,
                        SidebarEvent::OpenSettings,
                        window,
                        cx,
                    );
                }),
        )
    }

    fn account_session_button(
        view: &Entity<Self>,
        popover: &Entity<PopoverState>,
        connected: bool,
        cx: &App,
    ) -> impl IntoElement {
        let final_event = if connected {
            SidebarEvent::SignOut
        } else {
            SidebarEvent::BeginLogin
        };
        let final_label = if connected {
            "Log out"
        } else {
            "Sign in with ChatGPT"
        };
        let final_icon = if connected {
            Icon::new(IconName::ArrowRight)
        } else {
            provider_icon(Some(&ProviderId::new("openai")))
        };
        let final_view = view.clone();
        let final_popover = popover.clone();
        let final_action = Self::account_menu_button(
            "account-session-action",
            final_label,
            final_icon,
            None,
            false,
            cx,
        )
        .on_click(move |_, window, cx| {
            Self::emit_account_event(&final_view, &final_popover, final_event.clone(), window, cx);
        });

        v_flex().w_full().py(px(4.)).child(final_action)
    }

    fn account_menu_separator(cx: &App) -> impl IntoElement {
        div().w_full().h(px(1.)).bg(cx.theme().border.opacity(0.75))
    }

    fn account_menu_button(
        id: &'static str,
        label: &'static str,
        icon: impl Into<Icon>,
        trailing: Option<SharedString>,
        disabled: bool,
        cx: &App,
    ) -> Button {
        let icon = icon.into();
        Button::new(id)
            .ghost()
            .accessibility_id(id)
            .w_full()
            .h(px(32.))
            .px(px(8.))
            .rounded(px(7.))
            .disabled(disabled)
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(px(8.))
                    .child(icon.xsmall())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .child(label),
                    )
                    .when_some(trailing, |this, trailing| {
                        this.child(
                            div()
                                .flex_none()
                                .text_size(px(10.))
                                .text_color(cx.theme().muted_foreground)
                                .child(trailing),
                        )
                    }),
            )
    }

    fn emit_account_event(
        view: &Entity<Self>,
        popover: &Entity<PopoverState>,
        event: SidebarEvent,
        window: &mut Window,
        cx: &mut App,
    ) {
        popover.update(cx, |state, cx| state.dismiss(window, cx));
        view.update(cx, |_, cx| {
            cx.emit(event);
            cx.notify();
        });
    }
}
