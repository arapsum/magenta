mod model;
mod styles;

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    Window, div, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Collapsible, Icon, IconName, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    sidebar::{Sidebar, SidebarCollapsible, SidebarItem},
    v_flex,
};

use crate::theme;

pub use model::demo_conversations;
pub use model::{ConversationId, ConversationPeriod, ConversationSummary, SidebarEvent};

use self::{model::title_matches, styles::compact_bevel_light};

const EXPANDED_WIDTH: gpui::Pixels = px(252.);
const ROW_HEIGHT: gpui::Pixels = px(34.);
const INITIAL_RECENCY_LIMIT: usize = 6;
const RECENCY_PAGE_SIZE: usize = 6;

pub struct SidebarView {
    collapsed: bool,
    search: Entity<InputState>,
    conversations: Vec<ConversationSummary>,
    active_conversation: Option<ConversationId>,
    pinned_expanded: bool,
    recency_limit: usize,
    subscriptions: Vec<Subscription>,
}

impl SidebarView {
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search conversations"));
        let subscriptions =
            vec![
                cx.subscribe_in(&search, window, |sidebar, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        sidebar.recency_limit = INITIAL_RECENCY_LIMIT;
                        cx.notify();
                    }
                }),
            ];

        Self {
            collapsed: false,
            search,
            conversations: demo_conversations(),
            active_conversation: None,
            pinned_expanded: true,
            recency_limit: INITIAL_RECENCY_LIMIT,
            subscriptions,
        }
    }

    fn toggle_collapsed(&mut self, cx: &mut Context<'_, Self>) {
        self.collapsed = !self.collapsed;
        cx.notify();
    }

    fn new_chat(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.active_conversation = None;
        self.search
            .update(cx, |search, cx| search.set_value("", window, cx));
        cx.emit(SidebarEvent::NewChat);
        cx.notify();
    }

    fn select_conversation(&mut self, id: ConversationId, cx: &mut Context<'_, Self>) {
        self.active_conversation = Some(id);
        cx.emit(SidebarEvent::OpenConversation(id));
        cx.notify();
    }

    pub fn add_conversation(
        &mut self,
        conversation: ConversationSummary,
        cx: &mut Context<'_, Self>,
    ) {
        let id = conversation.id;
        self.conversations.retain(|item| item.id != id);
        self.conversations.insert(0, conversation);
        self.active_conversation = Some(id);
        self.recency_limit = INITIAL_RECENCY_LIMIT;
        cx.notify();
    }

    fn toggle_pinned(&mut self, id: ConversationId, cx: &mut Context<'_, Self>) {
        if let Some(conversation) = self.conversations.iter_mut().find(|item| item.id == id) {
            conversation.pinned = !conversation.pinned;
            cx.notify();
        }
    }

    fn toggle_pinned_expanded(&mut self, cx: &mut Context<'_, Self>) {
        self.pinned_expanded = !self.pinned_expanded;
        cx.notify();
    }

    fn show_more(&mut self, cx: &mut Context<'_, Self>) {
        self.recency_limit = self.recency_limit.saturating_add(RECENCY_PAGE_SIZE);
        cx.notify();
    }

    fn open_settings(cx: &mut Context<'_, Self>) {
        cx.emit(SidebarEvent::OpenSettings);
    }

    fn search_term(&self, cx: &App) -> String {
        self.search.read(cx).value().trim().to_lowercase()
    }

    fn matches_search(&self, conversation: &ConversationSummary, cx: &App) -> bool {
        title_matches(&conversation.title, &self.search_term(cx))
    }

    fn visible_recency(&self, cx: &App) -> Vec<&ConversationSummary> {
        self.conversations
            .iter()
            .filter(|item| !item.pinned && self.matches_search(item, cx))
            .collect()
    }

    fn render_header(&self, view: &Entity<Self>, cx: &App) -> AnyElement {
        let collapsed = self.collapsed;
        let toggle_view = view.clone();
        let toggle_label = if collapsed {
            "Expand sidebar"
        } else {
            "Collapse sidebar"
        };
        let toggle_icon = if collapsed {
            IconName::PanelLeftOpen
        } else {
            IconName::PanelLeftClose
        };

        h_flex()
            .w_full()
            .h(px(62.))
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(cx.theme().sidebar_border)
            .when(!collapsed, |this| {
                this.child(
                    div()
                        .text_size(px(21.))
                        .font_semibold()
                        .text_color(cx.theme().foreground)
                        .child("magenta"),
                )
            })
            .when(collapsed, gpui::Styled::justify_center)
            .child(
                Button::new("sidebar-toggle")
                    .ghost()
                    .small()
                    .icon(toggle_icon)
                    .tooltip(toggle_label)
                    .accessibility_id("toggle-sidebar")
                    .on_click(move |_, _, cx| {
                        toggle_view.update(cx, Self::toggle_collapsed);
                    }),
            )
            .into_any_element()
    }

    fn render_footer(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        let settings_view = view.clone();
        let theme_icon = if cx.theme().is_dark() {
            IconName::Sun
        } else {
            IconName::Moon
        };
        let theme_label = if cx.theme().is_dark() {
            "Use light theme"
        } else {
            "Use dark theme"
        };

        if self.collapsed {
            return h_flex()
                .w_full()
                .justify_center()
                .child(
                    Button::new("local-profile")
                        .ghost()
                        .size(px(32.))
                        .p_0()
                        .rounded_full()
                        .bg(cx.theme().primary.opacity(0.18))
                        .text_color(cx.theme().primary)
                        .label("A")
                        .tooltip("Local profile and settings")
                        .on_click(move |_, _, cx| {
                            settings_view.update(cx, |_, cx| Self::open_settings(cx));
                        }),
                )
                .into_any_element();
        }

        let theme_view = view;
        h_flex()
            .w_full()
            .items_center()
            .gap(px(8.))
            .p(px(9.))
            .rounded(px(10.))
            .border_1()
            .border_color(cx.theme().border.opacity(0.72))
            .bg(cx.theme().secondary.opacity(0.58))
            .child(
                Button::new("local-profile")
                    .ghost()
                    .size(px(28.))
                    .p_0()
                    .rounded_full()
                    .bg(cx.theme().primary.opacity(0.24))
                    .text_color(cx.theme().primary)
                    .label("A")
                    .tooltip("Local profile and settings")
                    .on_click(move |_, _, cx| {
                        settings_view.update(cx, |_, cx| Self::open_settings(cx));
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(div().text_size(px(12.)).font_medium().child("Adleio"))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(cx.theme().muted_foreground)
                            .child("Local profile"),
                    ),
            )
            .child(
                Button::new("sidebar-theme")
                    .ghost()
                    .small()
                    .icon(theme_icon)
                    .tooltip(theme_label)
                    .accessibility_id("toggle-theme")
                    .on_click(move |_, _, cx| {
                        if let Err(error) = theme::toggle(cx) {
                            tracing::error!(?error, "could not toggle the application theme");
                        }
                        theme_view.update(cx, |_, cx| cx.notify());
                    }),
            )
            .into_any_element()
    }

    fn new_chat_button(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        let active = self.active_conversation.is_none();
        let mut bevel_top = cx.theme().button_hover;
        bevel_top.s *= 0.34;
        let mut bevel_bottom = cx.theme().button;
        bevel_bottom.s *= 0.24;
        bevel_bottom.l = (bevel_bottom.l + 0.025).min(1.);
        let collapsed = self.collapsed;

        Button::new("sidebar-new-chat")
            .accessibility_id("new-chat")
            .w_full()
            .h(px(34.))
            .px(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(if active {
                cx.theme().input.opacity(0.88)
            } else {
                cx.theme().sidebar_border.opacity(0.78)
            })
            .bg(if active {
                linear_gradient(
                    180.,
                    linear_color_stop(bevel_top.opacity(0.98), 0.),
                    linear_color_stop(bevel_bottom.opacity(0.99), 1.),
                )
            } else {
                linear_gradient(
                    180.,
                    linear_color_stop(cx.theme().secondary.opacity(0.5), 0.),
                    linear_color_stop(cx.theme().secondary.opacity(0.34), 1.),
                )
            })
            .selected(active)
            .when(active, |this| {
                this.shadow(styles::illuminated_button_shadow(cx))
                    .child(compact_bevel_light(cx))
            })
            .text_color(cx.theme().foreground)
            .when(collapsed, |this| {
                this.size(px(32.))
                    .p_0()
                    .rounded(px(8.))
                    .icon(IconName::Plus)
                    .tooltip("New chat")
            })
            .when(!collapsed, |this| {
                this.child(
                    h_flex()
                        .relative()
                        .w_full()
                        .items_center()
                        .gap(px(10.))
                        .child(Icon::new(IconName::Plus).size_4())
                        .child(div().text_size(px(13.)).font_medium().child("New chat")),
                )
            })
            .on_click(move |_, window, cx| {
                view.update(cx, |sidebar, cx| sidebar.new_chat(window, cx));
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
        let pin_view = view;
        let star = if conversation.pinned {
            IconName::StarFill
        } else {
            IconName::Star
        };
        let pin_label = if conversation.pinned {
            "Unpin conversation"
        } else {
            "Pin conversation"
        };
        let mut bevel_top = cx.theme().button_hover;
        bevel_top.s *= 0.34;
        let mut bevel_bottom = cx.theme().button;
        bevel_bottom.s *= 0.24;
        bevel_bottom.l = (bevel_bottom.l + 0.025).min(1.);

        h_flex()
            .relative()
            .w_full()
            .h(ROW_HEIGHT)
            .items_center()
            .gap(px(2.))
            .rounded(px(7.))
            .when(selected, |this| {
                this.border_1()
                    .border_color(cx.theme().input.opacity(0.88))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(bevel_top.opacity(0.98), 0.),
                        linear_color_stop(bevel_bottom.opacity(0.99), 1.),
                    ))
                    .shadow(styles::illuminated_button_shadow(cx))
                    .child(compact_bevel_light(cx))
            })
            .child(
                Button::new(("conversation", id.0))
                    .ghost()
                    .selected(selected)
                    .accessibility_id(format!("conversation-{}", id.0))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .px(px(9.))
                    .rounded(px(7.))
                    .text_color(if selected {
                        cx.theme().sidebar_accent_foreground
                    } else {
                        cx.theme().sidebar_foreground.opacity(0.88)
                    })
                    .when(selected, |this| this.bg(cx.theme().transparent))
                    .child(
                        div()
                            .w_full()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .child(conversation.title.clone()),
                    )
                    .on_click(move |_, _, cx| {
                        select_view.update(cx, |sidebar, cx| sidebar.select_conversation(id, cx));
                    }),
            )
            .child(
                Button::new(("pin-conversation", id.0))
                    .ghost()
                    .small()
                    .icon(star)
                    .tooltip(pin_label)
                    .accessibility_id(format!("pin-conversation-{}", id.0))
                    .text_color(if conversation.pinned {
                        cx.theme().primary.opacity(0.88)
                    } else {
                        cx.theme().muted_foreground.opacity(0.48)
                    })
                    .on_click(move |_, _, cx| {
                        pin_view.update(cx, |sidebar, cx| sidebar.toggle_pinned(id, cx));
                    }),
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
                    .font_medium()
                    .text_color(cx.theme().muted_foreground)
                    .child(title)
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
                            .child(div().text_size(px(10.)).font_medium().child(title)),
                    )
                    .on_click(move |_, _, cx| {
                        label_view.update(cx, Self::toggle_pinned_expanded);
                    })
                    .into_any_element()
            },
        )
    }

    fn render_search_field(&self, window: &Window, cx: &App) -> AnyElement {
        let search_focused = self.search.read(cx).focus_handle(cx).is_focused(window);
        let search_border = if search_focused {
            cx.theme().primary.opacity(0.62)
        } else {
            cx.theme().sidebar_border.opacity(0.92)
        };
        let search_background = if search_focused {
            cx.theme().secondary.opacity(0.92)
        } else {
            cx.theme().secondary.opacity(0.58)
        };

        div()
            .pt(px(16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(33.))
                    .w_full()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(search_border)
                    .bg(search_background)
                    .when(search_focused, |this| {
                        this.shadow(vec![
                            gpui::BoxShadow::new(
                                px(0.),
                                px(2.),
                                cx.theme().background.opacity(0.36),
                            )
                            .blur_radius(px(5.)),
                            gpui::BoxShadow::new(px(0.), px(0.), cx.theme().primary.opacity(0.1))
                                .blur_radius(px(4.)),
                        ])
                    })
                    .child(
                        Input::new(&self.search)
                            .appearance(false)
                            .small()
                            .w_full()
                            .px(px(9.))
                            .cleanable(true)
                            .accessibility_id("search-conversations")
                            .aria_label("Search conversations")
                            .prefix(Icon::new(IconName::Search).small().text_color(
                                if search_focused {
                                    cx.theme().primary
                                } else {
                                    cx.theme().muted_foreground
                                },
                            )),
                    ),
            )
            .into_any_element()
    }

    fn render_content(&self, view: Entity<Self>, window: &Window, cx: &App) -> AnyElement {
        let new_chat = self.new_chat_button(view.clone(), cx);
        if self.collapsed {
            return v_flex()
                .w_full()
                .items_center()
                .gap(px(10.))
                .child(new_chat)
                .into_any_element();
        }

        let term = self.search_term(cx);
        let search_active = !term.is_empty();
        let pinned: Vec<_> = self
            .conversations
            .iter()
            .filter(|item| item.pinned)
            .collect();
        let recency = self.visible_recency(cx);
        let mut content = v_flex()
            .w_full()
            .gap(px(2.))
            .child(new_chat)
            .child(self.render_search_field(window, cx));

        if search_active {
            content = content.child(Self::section_label("Results", None, view.clone(), cx));
            if recency.is_empty() && pinned.iter().all(|item| !self.matches_search(item, cx)) {
                content = content.child(
                    div()
                        .px(px(9.))
                        .py(px(8.))
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child("No conversations match your search."),
                );
            } else {
                for item in self
                    .conversations
                    .iter()
                    .filter(|item| self.matches_search(item, cx))
                {
                    content = content.child(self.conversation_row(item, view.clone(), cx));
                }
            }
            return content.into_any_element();
        }

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
                    .h(px(31.))
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
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        v_flex()
            .id(id.into())
            .w_full()
            .child(
                self.view
                    .read(cx)
                    .render_content(self.view.clone(), window, cx),
            )
    }
}

impl Render for SidebarView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let _ = &self.subscriptions;
        let view = cx.entity();
        Sidebar::new("magenta-sidebar")
            .w(EXPANDED_WIDTH)
            .collapsible(SidebarCollapsible::Icon)
            .collapsed(self.collapsed)
            .header(self.render_header(&view, cx))
            .child(SidebarContent {
                view: view.clone(),
                collapsed: self.collapsed,
            })
            .footer(self.render_footer(view, cx))
    }
}
