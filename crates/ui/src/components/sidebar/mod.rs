mod model;

use gpui::{
    AnyElement, App, Bounds, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, RenderOnce,
    Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window, deferred, div,
    prelude::FluentBuilder as _, px,
};
use gpui_base::{Align, Placement, PopoverState, Positioner, actions::Cancel};
use gpui_component::{
    ActiveTheme as _, Collapsible, Disableable as _, ElementExt as _, Icon, IconName,
    Selectable as _, Sizable as _, StyledExt as _, ThemeStyled as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    sidebar::{Sidebar, SidebarCollapsible, SidebarItem},
    v_flex,
};
use magenta_core::ProviderAccount;

use crate::app::OpenConversationFinder;

pub use model::{ConversationId, ConversationPeriod, ConversationSummary, SidebarEvent};

use self::model::title_matches;

const EXPANDED_WIDTH: gpui::Pixels = px(260.);
const ROW_HEIGHT: gpui::Pixels = px(30.);
const INITIAL_RECENCY_LIMIT: usize = 6;
const RECENCY_PAGE_SIZE: usize = 6;
const ACCOUNT_MENU_WIDTH: gpui::Pixels = px(240.);
const ACCOUNT_MENU_GAP: gpui::Pixels = px(6.);

type AccountMenuBuilder =
    Box<dyn FnOnce(Entity<PopoverState>, &mut Window, &mut App) -> AnyElement>;

#[derive(IntoElement)]
struct AccountDropdown {
    id: ElementId,
    trigger: Button,
    menu: AccountMenuBuilder,
}

#[derive(Clone, Copy, Default)]
struct AccountDropdownAnchor {
    bounds: Bounds<gpui::Pixels>,
    captured: bool,
}

impl AccountDropdown {
    fn new(
        id: impl Into<ElementId>,
        trigger: Button,
        menu: impl FnOnce(Entity<PopoverState>, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            trigger,
            menu: Box::new(menu),
        }
    }
}

impl RenderOnce for AccountDropdown {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let open_state = window.use_keyed_state((self.id.clone(), "open"), cx, |_, cx| {
            PopoverState::new(false, cx)
        });
        let anchor_state = window.use_keyed_state((self.id.clone(), "anchor"), cx, |_, _| {
            AccountDropdownAnchor::default()
        });
        let parent_view = window.current_view();
        let open = open_state.read(cx).is_open();
        let trigger = self.trigger.selected(open);

        let root = div()
            .id(self.id)
            .debug_selector(|| "account-dropdown-trigger".into())
            .w_full()
            .child(trigger)
            .on_mouse_down(MouseButton::Left, {
                let open_state = open_state.clone();
                move |_, window, cx| {
                    cx.stop_propagation();
                    open_state.update(cx, |state, cx| {
                        state.set_open(open, cx);
                        state.toggle_open(window, cx);
                    });
                    cx.notify(parent_view);
                }
            })
            .on_prepaint({
                let anchor_state = anchor_state.clone();
                move |bounds, window, cx| {
                    let first = anchor_state.update(cx, |anchor, _| {
                        let first = !anchor.captured;
                        anchor.bounds = bounds;
                        anchor.captured = true;
                        first
                    });
                    if first {
                        window.request_animation_frame();
                    }
                }
            });

        let anchor = *anchor_state.read(cx);
        if !open || !anchor.captured {
            return root.into_any_element();
        }

        let focus_handle = open_state.read(cx).focus_handle(cx);
        let dismiss_state = open_state.clone();
        let cancel_state = open_state.clone();
        let menu = (self.menu)(open_state, window, cx);
        let surface = div()
            .id("sidebar-account-menu-surface")
            .debug_selector(|| "account-dropdown-surface".into())
            .role(Role::Dialog)
            .aria_label("Account menu")
            .occlude()
            .tab_group()
            .track_focus(&focus_handle)
            .key_context("Popover")
            .on_action(move |_: &Cancel, window, cx| {
                cancel_state.update(cx, |state, cx| state.dismiss(window, cx));
                cx.notify(parent_view);
            })
            .on_mouse_down_out(move |_, window, cx| {
                dismiss_state.update(cx, |state, cx| state.dismiss(window, cx));
                cx.notify(parent_view);
            })
            .child(menu);

        root.child(deferred(
            Positioner::side(anchor.bounds)
                .placement(Placement::Top)
                .align(Align::End)
                .offset(ACCOUNT_MENU_GAP)
                .margin(px(8.))
                .child(surface),
        ))
        .into_any_element()
    }
}

pub struct SidebarView {
    collapsed: bool,
    conversations: Vec<ConversationSummary>,
    active_conversation: Option<ConversationId>,
    finder_launcher_focus: FocusHandle,
    account: Option<ProviderAccount>,
    pinned_expanded: bool,
    recency_limit: usize,
    history_status: Option<&'static str>,
    history_failed: bool,
}

impl SidebarView {
    pub fn new(_window: &mut Window, cx: &Context<'_, Self>) -> Self {
        Self {
            collapsed: false,
            conversations: Vec::new(),
            active_conversation: None,
            finder_launcher_focus: cx.focus_handle(),
            account: None,
            pinned_expanded: true,
            recency_limit: INITIAL_RECENCY_LIMIT,
            history_status: Some("Loading conversations…"),
            history_failed: false,
        }
    }

    pub(crate) fn toggle_collapsed(&mut self, cx: &mut Context<'_, Self>) {
        self.collapsed = !self.collapsed;
        cx.notify();
    }

    pub(crate) const fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    #[cfg(test)]
    pub(crate) const fn active_conversation(&self) -> Option<ConversationId> {
        self.active_conversation
    }
    pub(crate) fn focus_finder_launcher(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.finder_launcher_focus.focus(window, cx);
    }

    pub(crate) fn matching_conversations(
        &self,
        query: &str,
    ) -> Vec<(ConversationId, SharedString, SharedString)> {
        self.conversations
            .iter()
            .filter(|conversation| title_matches(&conversation.title, query))
            .map(|conversation| {
                (
                    conversation.id,
                    conversation.title.clone().into(),
                    conversation.updated.clone().into(),
                )
            })
            .collect()
    }

    pub(crate) fn recent_conversations(&self) -> Vec<(ConversationId, SharedString)> {
        self.conversations
            .iter()
            .take(3)
            .map(|conversation| (conversation.id, conversation.title.clone().into()))
            .collect()
    }

    pub(crate) const fn history_available(&self) -> bool {
        self.history_status.is_none() && !self.conversations.is_empty()
    }

    fn new_chat(cx: &mut Context<'_, Self>) {
        cx.emit(SidebarEvent::NewChat);
        cx.notify();
    }

    fn select_conversation(id: ConversationId, cx: &mut Context<'_, Self>) {
        cx.emit(SidebarEvent::OpenConversation(id));
        cx.notify();
    }

    pub(crate) fn set_account(
        &mut self,
        account: Option<ProviderAccount>,
        cx: &mut Context<'_, Self>,
    ) {
        self.account = account;
        cx.notify();
    }

    fn set_pinned(&self, id: ConversationId, pinned: bool, cx: &mut Context<'_, Self>) {
        if self
            .conversations
            .iter()
            .any(|conversation| conversation.id == id)
        {
            cx.emit(SidebarEvent::SetPinned(id, pinned));
        }
    }

    pub(crate) fn set_history(
        &mut self,
        summaries: Vec<magenta_core::ConversationSummary>,
        cx: &mut Context<'_, Self>,
    ) {
        self.conversations = summaries.into_iter().map(Into::into).collect();
        self.history_status = None;
        self.history_failed = false;
        cx.notify();
    }

    pub(crate) fn set_history_loading(&mut self, failed: bool, cx: &mut Context<'_, Self>) {
        self.history_failed = failed;
        self.history_status = Some(if failed {
            "History could not be loaded."
        } else {
            "Loading conversations…"
        });
        cx.notify();
    }

    pub(crate) fn set_active(&mut self, id: Option<ConversationId>, cx: &mut Context<'_, Self>) {
        self.active_conversation = id;
        cx.notify();
    }

    fn toggle_pinned_expanded(&mut self, cx: &mut Context<'_, Self>) {
        self.pinned_expanded = !self.pinned_expanded;
        cx.notify();
    }

    fn show_more(&mut self, cx: &mut Context<'_, Self>) {
        self.recency_limit = self.recency_limit.saturating_add(RECENCY_PAGE_SIZE);
        cx.notify();
    }

    fn render_footer(&self, view: &Entity<Self>, cx: &App) -> AnyElement {
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
            IconName::ArrowRight
        } else {
            IconName::Bot
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
            Self::emit_account_event(&final_view, &final_popover, final_event, window, cx);
        });

        v_flex().w_full().py(px(4.)).child(final_action)
    }

    fn account_menu_separator(cx: &App) -> impl IntoElement {
        div().w_full().h(px(1.)).bg(cx.theme().border.opacity(0.75))
    }

    fn account_menu_button(
        id: &'static str,
        label: &'static str,
        icon: IconName,
        trailing: Option<SharedString>,
        disabled: bool,
        cx: &App,
    ) -> Button {
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
                    .child(Icon::new(icon).xsmall())
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

    fn new_chat_button(view: Entity<Self>, _cx: &App) -> AnyElement {
        Button::new("sidebar-new-chat")
            .primary()
            .accessibility_id("new-chat")
            .w_full()
            .h(px(32.))
            .rounded(px(7.))
            .icon(IconName::Plus)
            .label("New chat")
            .on_click(move |_, _window, cx| {
                view.update(cx, |_, cx| Self::new_chat(cx));
            })
            .into_any_element()
    }

    fn conversation_row(
        &self,
        conversation: &ConversationSummary,
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let selected = self.active_conversation == Some(conversation.id);
        let id = conversation.id;
        let select_view = view.clone();
        let menu_view = view;
        let next_pinned = !conversation.pinned;
        let menu_label = if next_pinned { "Pin" } else { "Unpin" };
        let group_name: SharedString = format!("conversation-row-{}", id.0).into();
        let metadata = (!conversation.pinned).then(|| conversation.updated.clone());

        h_flex()
            .relative()
            .group(group_name.clone())
            .w_full()
            .h(ROW_HEIGHT)
            .items_center()
            .rounded(px(6.))
            .when(selected, |this| {
                this.bg(cx.theme().sidebar_accent)
                    .text_color(cx.theme().sidebar_accent_foreground)
            })
            .hover(|this| this.bg(cx.theme().sidebar_accent))
            .child(
                Button::new(("conversation", id.0))
                    .ghost()
                    .accessibility_id(format!("conversation-{}", id.0))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .px(px(8.))
                    .rounded(px(6.))
                    .text_color(if selected {
                        cx.theme().sidebar_accent_foreground
                    } else {
                        cx.theme().sidebar_foreground
                    })
                    .child(
                        div()
                            .w_full()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(13.))
                            .child(conversation.title.clone()),
                    )
                    .on_click(move |_, _, cx| {
                        select_view.update(cx, |_, cx| Self::select_conversation(id, cx));
                    }),
            )
            .child(
                div()
                    .relative()
                    .flex_none()
                    .size(px(28.))
                    .when_some(metadata, |this, updated| {
                        this.child(
                            h_flex()
                                .absolute()
                                .inset_0()
                                .justify_center()
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_size(px(11.))
                                .text_color(cx.theme().muted_foreground.opacity(0.8))
                                .when(selected, gpui::Styled::invisible)
                                .group_hover(group_name.clone(), gpui::Styled::invisible)
                                .child(updated),
                        )
                    })
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .when(!selected, |this| {
                                this.invisible()
                                    .group_hover(group_name, gpui::Styled::visible)
                            })
                            .child(
                                Button::new(("conversation-more", id.0))
                                    .ghost()
                                    .xsmall()
                                    .size(px(28.))
                                    .p_0()
                                    .icon(IconName::Ellipsis)
                                    .tooltip(format!("{menu_label} conversation"))
                                    .accessibility_id(format!("conversation-more-{}", id.0))
                                    .text_color(cx.theme().muted_foreground)
                                    .dropdown_menu(move |menu, window, _cx| {
                                        let action_view = menu_view.clone();
                                        menu.item(PopupMenuItem::new(menu_label).on_click(
                                            window.listener_for(
                                                &action_view,
                                                move |sidebar, _, _, cx| {
                                                    sidebar.set_pinned(id, next_pinned, cx);
                                                },
                                            ),
                                        ))
                                    }),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn section_label(
        title: &'static str,
        disclosure: Option<bool>,
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        disclosure.map_or_else(
            || {
                div()
                    .h(px(27.))
                    .flex()
                    .items_end()
                    .px(px(8.))
                    .pb(px(4.))
                    .text_size(px(10.))
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground.opacity(0.8))
                    .child(title.to_uppercase())
                    .into_any_element()
            },
            |expanded| {
                let label_view = view;
                Button::new("pinned-disclosure")
                    .ghost()
                    .w_full()
                    .h(px(28.))
                    .px(px(7.))
                    .rounded(px(6.))
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                Icon::new(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .xsmall(),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .font_semibold()
                                    .child(title.to_uppercase()),
                            ),
                    )
                    .on_click(move |_, _, cx| {
                        label_view.update(cx, Self::toggle_pinned_expanded);
                    })
                    .into_any_element()
            },
        )
    }

    fn render_history_status(
        &self,
        mut content: gpui::Div,
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        if let Some(status) = self.history_status {
            content = content.child(div().p(px(9.)).text_size(px(12.)).child(status));
            if self.history_failed {
                let retry_view = view;
                content = content.child(
                    Button::new("retry-history")
                        .ghost()
                        .label("Retry")
                        .on_click(move |_, _, cx| {
                            retry_view.update(cx, |_, cx| cx.emit(SidebarEvent::RetryHistory));
                        }),
                );
            }
            return content.into_any_element();
        }
        if self.conversations.is_empty() {
            return content
                .child(
                    div()
                        .p(px(9.))
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child("Your conversations will appear here."),
                )
                .into_any_element();
        }

        content.into_any_element()
    }

    fn search_button(&self, cx: &App) -> AnyElement {
        let finder_shortcut = if cfg!(target_os = "macos") {
            "⌘K"
        } else {
            "Ctrl K"
        };
        h_flex()
            .id("sidebar-search-chats")
            .track_focus(&self.finder_launcher_focus)
            .role(Role::Button)
            .aria_label("Search chats")
            .tab_stop(true)
            .cursor_pointer()
            .w_full()
            .h(px(32.))
            .px(px(10.))
            .gap(px(7.))
            .rounded(px(8.))
            .border_1()
            .border_color(cx.theme().input)
            .bg(cx.theme().background)
            .text_color(cx.theme().muted_foreground)
            .hover(|this| this.bg(cx.theme().muted.opacity(0.6)))
            .child(Icon::new(IconName::Search).xsmall())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.))
                    .child("Search chats"),
            )
            .child(
                h_flex()
                    .h(px(18.))
                    .min_w(if cfg!(target_os = "macos") {
                        px(24.)
                    } else {
                        px(38.)
                    })
                    .justify_center()
                    .px(px(5.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().sidebar)
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
                    .child(finder_shortcut),
            )
            .on_click(move |_, window, cx| {
                window.dispatch_action(Box::new(OpenConversationFinder), cx);
            })
            .into_any_element()
    }

    fn render_content(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        let controls = v_flex()
            .w_full()
            .gap(px(8.))
            .child(Self::new_chat_button(view.clone(), cx))
            .child(self.search_button(cx));
        let mut content = v_flex().w_full().gap(px(3.)).child(controls);

        if self.history_status.is_some() || self.conversations.is_empty() {
            return self.render_history_status(content, view, cx);
        }

        let pinned: Vec<_> = self
            .conversations
            .iter()
            .filter(|item| item.pinned)
            .collect();
        if !pinned.is_empty() {
            content = content.child(Self::section_label(
                "Pinned",
                Some(self.pinned_expanded),
                view.clone(),
                cx,
            ));
            if self.pinned_expanded {
                for item in pinned {
                    content = content.child(self.conversation_row(item, view.clone(), cx));
                }
            }
        }

        let recency: Vec<_> = self
            .conversations
            .iter()
            .filter(|item| !item.pinned)
            .collect();
        let limited_recency: Vec<_> = recency.iter().take(self.recency_limit).copied().collect();
        for period in ConversationPeriod::ALL {
            let grouped: Vec<_> = limited_recency
                .iter()
                .copied()
                .filter(|item| item.period == period)
                .collect();
            if !grouped.is_empty() {
                content =
                    content.child(Self::section_label(period.label(), None, view.clone(), cx));
                for item in grouped {
                    content = content.child(self.conversation_row(item, view.clone(), cx));
                }
            }
        }

        if recency.len() > limited_recency.len() {
            let more_view = view;
            content = content.child(
                Button::new("show-more-conversations")
                    .ghost()
                    .w_full()
                    .h(px(30.))
                    .px(px(8.))
                    .rounded(px(6.))
                    .text_color(cx.theme().muted_foreground)
                    .label("Show more")
                    .on_click(move |_, _, cx| more_view.update(cx, Self::show_more)),
            );
        }

        content.into_any_element()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProfileDetails {
    name: String,
    detail: String,
    initial: String,
    tooltip: String,
    email: Option<String>,
    plan: Option<String>,
}

fn profile_details(account: Option<&ProviderAccount>) -> ProfileDetails {
    let Some(account) = account else {
        return ProfileDetails {
            name: "Adleio".to_owned(),
            detail: "Local profile".to_owned(),
            initial: "A".to_owned(),
            tooltip: "Local profile and settings".to_owned(),
            email: None,
            plan: None,
        };
    };

    let name = account
        .name
        .clone()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .or_else(|| account.email.as_deref().and_then(email_display_name))
        .or_else(|| account.email.clone())
        .unwrap_or_else(|| "ChatGPT account".to_owned());
    let email = account.email.clone();
    let plan = account.plan.as_deref().and_then(display_plan);
    let detail = match (plan.as_deref(), email.as_deref()) {
        (Some(plan), Some(email)) => format!("{plan} · {email}"),
        (Some(plan), None) => plan.to_owned(),
        (None, Some(email)) => email.to_owned(),
        (None, None) => "OpenAI account".to_owned(),
    };
    let initial = name.chars().next().map_or_else(
        || "O".to_owned(),
        |character| character.to_uppercase().collect(),
    );
    let tooltip = account.email.clone().map_or_else(
        || "OpenAI account and settings".to_owned(),
        |email| format!("{name} · {email}"),
    );

    ProfileDetails {
        name,
        detail,
        initial,
        tooltip,
        email,
        plan,
    }
}

fn display_plan(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| capitalize(value))
}

fn email_display_name(email: &str) -> Option<String> {
    let local_part = email.split('@').next()?.trim();
    let parts = local_part
        .split(['.', '_', '-'])
        .filter(|part| !part.is_empty())
        .map(capitalize)
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn capitalize(value: &str) -> String {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };

    let mut capitalized = first.to_uppercase().collect::<String>();
    capitalized.push_str(characters.as_str());
    capitalized
}

#[cfg(test)]
mod tests {
    use gpui::{
        App, Context, InteractiveElement as _, IntoElement, Modifiers, ParentElement as _, Render,
        Styled as _, TestAppContext, Window, div, px,
    };
    use gpui_component::{ActiveTheme as _, button::Button};
    use magenta_core::{ProviderAccount, ProviderId};

    use super::{AccountDropdown, ProfileDetails, profile_details};

    struct AccountDropdownHarness;

    impl Render for AccountDropdownHarness {
        fn render(
            &mut self,
            _window: &mut Window,
            _cx: &mut Context<'_, Self>,
        ) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .items_end()
                .p(px(20.))
                .child(AccountDropdown::new(
                    "account-dropdown-test",
                    Button::new("account-dropdown-test-trigger")
                        .w(px(200.))
                        .h(px(40.))
                        .label("Account"),
                    |_, _, cx: &mut App| {
                        div()
                            .debug_selector(|| "account-dropdown-test-menu".into())
                            .w(px(180.))
                            .h(px(100.))
                            .bg(cx.theme().popover)
                            .into_any_element()
                    },
                ))
        }
    }

    #[gpui::test]
    fn account_dropdown_opens_and_dismisses_from_an_outside_click(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let (_, cx) = cx.add_window_view(|_, _| AccountDropdownHarness);
        cx.update(|window, cx| window.draw(cx).clear(cx));

        let trigger = cx.debug_bounds("account-dropdown-trigger").unwrap();
        cx.simulate_click(trigger.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert!(cx.debug_bounds("account-dropdown-test-menu").is_some());

        cx.simulate_click(gpui::point(px(5.), px(5.)), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert!(cx.debug_bounds("account-dropdown-test-menu").is_none());
    }

    #[test]
    fn signed_out_profile_keeps_the_local_fallback() {
        assert_eq!(
            profile_details(None),
            ProfileDetails {
                name: "Adleio".to_owned(),
                detail: "Local profile".to_owned(),
                initial: "A".to_owned(),
                tooltip: "Local profile and settings".to_owned(),
                email: None,
                plan: None,
            }
        );
    }

    #[test]
    fn connected_profile_uses_name_email_and_initial() {
        let account = ProviderAccount {
            provider: ProviderId::new("openai"),
            name: Some("Jacob Cooper".to_owned()),
            email: Some("jacob@example.com".to_owned()),
            plan: Some("plus".to_owned()),
        };

        assert_eq!(
            profile_details(Some(&account)),
            ProfileDetails {
                name: "Jacob Cooper".to_owned(),
                detail: "Plus · jacob@example.com".to_owned(),
                initial: "J".to_owned(),
                tooltip: "Jacob Cooper · jacob@example.com".to_owned(),
                email: Some("jacob@example.com".to_owned()),
                plan: Some("Plus".to_owned()),
            }
        );
    }

    #[test]
    fn connected_profile_derives_a_friendly_name_when_claims_have_no_name() {
        let account = ProviderAccount {
            provider: ProviderId::new("openai"),
            name: None,
            email: Some("jacob.cooper@example.com".to_owned()),
            plan: Some("plus".to_owned()),
        };

        let details = profile_details(Some(&account));

        assert_eq!(details.name, "Jacob Cooper");
        assert_eq!(details.detail, "Plus · jacob.cooper@example.com");
        assert_eq!(details.initial, "J");
        assert_eq!(details.email.as_deref(), Some("jacob.cooper@example.com"));
        assert_eq!(details.plan.as_deref(), Some("Plus"));
    }
}

impl EventEmitter<SidebarEvent> for SidebarView {}

#[derive(Clone)]
struct SidebarContent {
    view: Entity<SidebarView>,
    collapsed: bool,
}

impl Collapsible for SidebarContent {
    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    fn is_collapsed(&self) -> bool {
        self.collapsed
    }
}

impl SidebarItem for SidebarContent {
    fn render(
        self,
        id: impl Into<gpui::ElementId>,
        _window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        v_flex()
            .id(id.into())
            .w_full()
            .child(self.view.read(cx).render_content(self.view.clone(), cx))
    }
}

impl Render for SidebarView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let view = cx.entity();
        Sidebar::new("magenta-sidebar")
            .w(EXPANDED_WIDTH)
            .collapsible(SidebarCollapsible::None)
            .child(SidebarContent {
                view: view.clone(),
                collapsed: false,
            })
            .footer(self.render_footer(&view, cx))
    }
}
