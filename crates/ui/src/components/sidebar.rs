use gpui::{
    AnyElement, App, BoxShadow, Context, ElementId, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Styled as _, Window, div, linear_color_stop, linear_gradient,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Collapsible, Icon, IconName, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    sidebar::{Sidebar, SidebarCollapsible, SidebarItem},
    v_flex,
};

use crate::app::MainView;

const EXPANDED_WIDTH: gpui::Pixels = px(240.);
const NAV_ROW_HEIGHT: gpui::Pixels = px(36.);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Destination {
    NewChat,
    Models,
    ImageLibrary,
    Experts,
    Collaborate,
    TrustScore,
    Billing,
    SearchChat,
    AddFolder,
    Project,
    Recent,
    StartupStrategy,
    SocialContent,
}

impl Destination {
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::NewChat => "new-chat",
            Self::Models => "models",
            Self::ImageLibrary => "image-library",
            Self::Experts => "experts",
            Self::Collaborate => "collaborate",
            Self::TrustScore => "trust-score",
            Self::Billing => "billing",
            Self::SearchChat => "search-chat",
            Self::AddFolder => "add-folder",
            Self::Project => "project",
            Self::Recent => "recent",
            Self::StartupStrategy => "startup-strategy",
            Self::SocialContent => "social-content",
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::NewChat => "New Chat",
            Self::Models => "Models",
            Self::ImageLibrary => "Image Library",
            Self::Experts => "Experts",
            Self::Collaborate => "Collaborate",
            Self::TrustScore => "Trust Score",
            Self::Billing => "Billing",
            Self::SearchChat => "Search Chat",
            Self::AddFolder => "Add New Folder",
            Self::Project => "New project",
            Self::Recent => "Recent Conversations",
            Self::StartupStrategy => "Startup marketing strategy",
            Self::SocialContent => "Content ideas for social media",
        }
    }
}

#[derive(Clone)]
struct SidebarBody {
    collapsed: bool,
    active: Destination,
    view: Entity<MainView>,
}

impl SidebarBody {
    fn new(active: Destination, view: Entity<MainView>) -> Self {
        Self {
            collapsed: false,
            active,
            view,
        }
    }

    fn destination_button(
        &self,
        destination: Destination,
        icon: IconName,
        suffix: Option<AnyElement>,
        cx: &mut App,
    ) -> Button {
        let active = self.active == destination;
        let collapsed = self.collapsed;
        let label = destination.label();
        let view = self.view.clone();

        let content = h_flex()
            .w_full()
            .items_center()
            .justify_start()
            .gap(px(12.))
            .child(Icon::new(icon.clone()).size_4().text_color(if active {
                cx.theme().sidebar_accent_foreground
            } else {
                cx.theme().sidebar_foreground.opacity(0.88)
            }))
            .when(!collapsed, |this| {
                this.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.))
                        .line_height(px(18.))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(label),
                )
                .when_some(suffix, |this, suffix| this.child(suffix))
            });

        Button::new(destination.id())
            .ghost()
            .selected(active)
            .accessibility_id(format!("sidebar-{}", destination.id()))
            .w_full()
            .h(NAV_ROW_HEIGHT)
            .px(px(10.))
            .rounded(px(7.))
            .text_color(if active {
                cx.theme().sidebar_accent_foreground
            } else {
                cx.theme().sidebar_foreground
            })
            .when(active, |this| {
                this.bg(cx.theme().tokens.sidebar_accent.background)
                    .border_1()
                    .border_color(cx.theme().primary.opacity(0.12))
            })
            .when(collapsed, |this| {
                this.size(px(32.)).p_0().icon(icon).tooltip(label)
            })
            .when(!collapsed, |this| this.child(content))
            .on_click(move |_, _, cx| {
                view.update(cx, |view, cx| {
                    view.select_sidebar_destination(destination, cx);
                });
            })
    }

    fn history_button(
        &self,
        destination: Option<Destination>,
        label: &'static str,
        trailing_menu: bool,
        muted: bool,
        cx: &mut App,
    ) -> AnyElement {
        let active = destination == Some(self.active);
        let view = self.view.clone();
        let id = destination.map_or("website-copy", Destination::id);

        Button::new(id)
            .ghost()
            .selected(active)
            .accessibility_id(format!("sidebar-{id}"))
            .w_full()
            .h(px(36.))
            .px(px(10.))
            .rounded(px(7.))
            .text_color(if muted {
                cx.theme().muted_foreground.opacity(0.34)
            } else if active {
                cx.theme().sidebar_accent_foreground
            } else {
                cx.theme().sidebar_foreground.opacity(0.75)
            })
            .when(active, |this| {
                this.bg(cx.theme().tokens.sidebar_accent.background)
                    .border_1()
                    .border_color(cx.theme().border.opacity(0.42))
            })
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_2()
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
                    .when(trailing_menu, |this| {
                        this.child(Icon::new(IconName::EllipsisVertical).xsmall())
                    }),
            )
            .when(destination.is_none(), |this| this.tab_stop(false))
            .when_some(destination, |this, destination| {
                this.on_click(move |_, _, cx| {
                    view.update(cx, |view, cx| {
                        view.select_sidebar_destination(destination, cx);
                    });
                })
            })
            .into_any_element()
    }
}
fn new_chat_button(body: &SidebarBody, cx: &mut App) -> Button {
    let mut bevel_top = cx.theme().button_hover;
    bevel_top.s *= 0.34;
    let mut bevel_bottom = cx.theme().button;
    bevel_bottom.s *= 0.24;
    bevel_bottom.l = (bevel_bottom.l + 0.025).min(1.);

    let new_chat_view = body.view.clone();
    Button::new(Destination::NewChat.id())
        .accessibility_id("sidebar-new-chat")
        .w_full()
        .h(px(32.))
        .px(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(cx.theme().input.opacity(0.88))
        .bg(linear_gradient(
            180.,
            linear_color_stop(bevel_top.opacity(0.98), 0.),
            linear_color_stop(bevel_bottom.opacity(0.99), 1.),
        ))
        .shadow(illuminated_button_shadow(cx))
        .text_color(cx.theme().foreground)
        .when(body.collapsed, |this| {
            this.size(px(32.))
                .p_0()
                .rounded(px(8.))
                .tooltip(Destination::NewChat.label())
        })
        .child(compact_bevel_light(cx))
        .when(body.collapsed, |this| this.icon(IconName::Plus))
        .when(!body.collapsed, |this| {
            this.child(
                h_flex()
                    .relative()
                    .w_full()
                    .items_center()
                    .justify_start()
                    .gap(px(10.))
                    .child(Icon::new(IconName::Plus).size_4())
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_medium()
                            .child(Destination::NewChat.label()),
                    ),
            )
        })
        .on_click(move |_, _, cx| {
            new_chat_view.update(cx, |view, cx| {
                view.select_sidebar_destination(Destination::NewChat, cx);
            });
        })
}

fn primary_navigation(body: &SidebarBody, cx: &mut App) -> AnyElement {
    v_flex()
        .w_full()
        .gap(px(7.))
        .child(body.destination_button(Destination::Models, IconName::Bot, None, cx))
        .child(body.destination_button(
            Destination::ImageLibrary,
            IconName::GalleryVerticalEnd,
            None,
            cx,
        ))
        .child(body.destination_button(Destination::Experts, IconName::User, None, cx))
        .child(body.destination_button(Destination::Collaborate, IconName::Calendar, None, cx))
        .child(body.destination_button(Destination::TrustScore, IconName::Star, None, cx))
        .child(body.destination_button(Destination::Billing, IconName::ChartPie, None, cx))
        .into_any_element()
}

fn secondary_navigation(body: &SidebarBody, cx: &mut App) -> AnyElement {
    let project_suffix = h_flex()
        .gap_1()
        .items_center()
        .text_color(cx.theme().muted_foreground)
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(19.))
                .rounded(px(5.))
                .bg(cx.theme().muted.opacity(0.75))
                .text_size(px(10.))
                .child("∞"),
        )
        .child(Icon::new(IconName::EllipsisVertical).xsmall())
        .into_any_element();

    v_flex()
        .w_full()
        .gap(px(5.))
        .border_t_1()
        .border_color(cx.theme().sidebar_border)
        .pt(px(14.))
        .child(body.destination_button(Destination::SearchChat, IconName::Search, None, cx))
        .child(body.destination_button(Destination::AddFolder, IconName::Folder, None, cx))
        .child(body.destination_button(
            Destination::Project,
            IconName::ChevronDown,
            Some(project_suffix),
            cx,
        ))
        .child(body.destination_button(Destination::Recent, IconName::Inbox, None, cx))
        .child(body.history_button(
            Some(Destination::StartupStrategy),
            Destination::StartupStrategy.label(),
            true,
            false,
            cx,
        ))
        .child(body.history_button(
            Some(Destination::SocialContent),
            Destination::SocialContent.label(),
            false,
            false,
            cx,
        ))
        .child(body.history_button(None, "Website landing page copy", false, true, cx))
        .into_any_element()
}

impl Collapsible for SidebarBody {
    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    fn is_collapsed(&self) -> bool {
        self.collapsed
    }
}

impl SidebarItem for SidebarBody {
    fn render(
        self,
        id: impl Into<ElementId>,
        _window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let new_chat = new_chat_button(&self, cx);
        let primary_navigation = primary_navigation(&self, cx);
        let secondary_navigation_view = (!self.collapsed).then(|| secondary_navigation(&self, cx));

        v_flex()
            .id(id.into())
            .w_full()
            .gap(px(7.))
            .child(new_chat)
            .child(primary_navigation)
            .when_some(secondary_navigation_view, |this, navigation| {
                this.child(navigation)
            })
    }
}

pub(crate) fn render(
    collapsed: bool,
    active: Destination,
    cx: &mut Context<MainView>,
) -> AnyElement {
    Sidebar::new("magenta-sidebar")
        .w(EXPANDED_WIDTH)
        .collapsible(SidebarCollapsible::Icon)
        .collapsed(collapsed)
        .header(header(collapsed, cx))
        .child(SidebarBody::new(active, cx.entity().clone()))
        .footer(footer(collapsed, cx))
        .into_any_element()
}

fn select_handler(
    destination: Destination,
    cx: &Context<MainView>,
) -> impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static {
    cx.listener(move |view, _, _, cx| {
        view.select_sidebar_destination(destination, cx);
    })
}

fn header(collapsed: bool, cx: &mut Context<MainView>) -> AnyElement {
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
        .h(px(57.))
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(cx.theme().sidebar_border)
        .when(!collapsed, |this| {
            this.child(
                div()
                    .text_size(px(20.))
                    .font_semibold()
                    .text_color(cx.theme().foreground)
                    .child("magenta"),
            )
        })
        .when(collapsed, |this| this.justify_center())
        .child(
            Button::new("sidebar-toggle")
                .ghost()
                .small()
                .icon(toggle_icon)
                .tooltip(toggle_label)
                .on_click(cx.listener(|view, _, _, cx| {
                    view.toggle_sidebar(cx);
                })),
        )
        .into_any_element()
}

fn footer(collapsed: bool, cx: &mut Context<MainView>) -> AnyElement {
    if collapsed {
        compact_footer(cx)
    } else {
        upgrade_card(cx)
    }
}

fn compact_footer(cx: &mut Context<MainView>) -> AnyElement {
    h_flex()
        .w_full()
        .justify_center()
        .child(
            Button::new("upgrade-compact")
                .primary()
                .small()
                .icon(IconName::Asterisk)
                .tooltip("Upgrade to Magenta Pro")
                .on_click(select_handler(Destination::Billing, cx)),
        )
        .into_any_element()
}

fn upgrade_card(cx: &mut Context<MainView>) -> AnyElement {
    let mut card_top = cx.theme().button_hover;
    card_top.s *= 0.38;
    let mut card_bottom = cx.theme().button;
    card_bottom.s *= 0.24;
    card_bottom.l = (card_bottom.l + 0.025).min(1.);

    v_flex()
        .id("upgrade-card")
        .w_full()
        .h(px(128.))
        .justify_between()
        .p(px(10.))
        .rounded(px(12.))
        .bg(linear_gradient(
            180.,
            linear_color_stop(card_top.opacity(0.98), 0.),
            linear_color_stop(card_bottom.opacity(0.99), 1.),
        ))
        .border_1()
        .border_color(cx.theme().primary.opacity(0.38))
        .shadow(illuminated_card_shadow(cx))
        .child(card_bevel_light(cx))
        .child(upgrade_card_header(cx))
        .child(upgrade_card_copy(cx))
        .child(upgrade_card_actions(cx))
        .into_any_element()
}

fn upgrade_card_header(cx: &mut Context<MainView>) -> AnyElement {
    h_flex()
        .relative()
        .items_center()
        .justify_between()
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(24.))
                        .rounded_full()
                        .bg(cx.theme().tokens.primary.background)
                        .text_color(cx.theme().primary_foreground)
                        .child(Icon::new(IconName::Star).xsmall()),
                )
                .child(
                    div()
                        .font_medium()
                        .text_size(px(13.))
                        .text_color(cx.theme().foreground)
                        .child("Upgrade Pro!"),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(20.))
                .rounded_full()
                .border_1()
                .border_color(cx.theme().border)
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::WindowClose).xsmall()),
        )
        .into_any_element()
}

fn upgrade_card_copy(cx: &mut Context<MainView>) -> AnyElement {
    div()
        .relative()
        .max_w(px(170.))
        .text_size(px(10.))
        .line_height(px(14.))
        .text_color(cx.theme().muted_foreground)
        .child("Upgrade to Pro and elevate your experience today")
        .into_any_element()
}

fn upgrade_card_actions(cx: &mut Context<MainView>) -> AnyElement {
    h_flex()
        .relative()
        .items_center()
        .gap(px(8.))
        .child(
            Button::new("upgrade")
                .primary()
                .small()
                .h(px(28.))
                .px(px(14.))
                .rounded(px(7.))
                .bg(linear_gradient(
                    100.,
                    linear_color_stop(cx.theme().primary, 0.),
                    linear_color_stop(cx.theme().blue, 1.),
                ))
                .shadow(vec![
                    BoxShadow::new(px(0.), px(3.), cx.theme().primary.opacity(0.22))
                        .blur_radius(px(9.)),
                    BoxShadow::new(px(0.), px(1.), cx.theme().foreground.opacity(0.2)).inset(),
                ])
                .icon(IconName::Star)
                .label("Upgrade")
                .on_click(select_handler(Destination::Billing, cx)),
        )
        .child(
            Button::new("learn-more")
                .ghost()
                .h(px(28.))
                .px(px(8.))
                .label("Learn More")
                .on_click(select_handler(Destination::Billing, cx)),
        )
        .into_any_element()
}

fn illuminated_button_shadow(cx: &App) -> Vec<BoxShadow> {
    vec![
        BoxShadow::new(px(0.), px(0.), cx.theme().primary.opacity(0.12)).blur_radius(px(12.)),
        BoxShadow::new(px(0.), px(4.), cx.theme().background.opacity(0.66)).blur_radius(px(9.)),
        BoxShadow::new(px(0.), px(1.), cx.theme().foreground.opacity(0.1)).inset(),
        BoxShadow::new(px(0.), px(-1.), cx.theme().background.opacity(0.72)).inset(),
    ]
}

fn illuminated_card_shadow(cx: &App) -> Vec<BoxShadow> {
    vec![
        BoxShadow::new(px(0.), px(0.), cx.theme().primary.opacity(0.12)).blur_radius(px(17.)),
        BoxShadow::new(px(0.), px(7.), cx.theme().background.opacity(0.7)).blur_radius(px(14.)),
        BoxShadow::new(px(0.), px(1.), cx.theme().foreground.opacity(0.09)).inset(),
        BoxShadow::new(px(0.), px(-2.), cx.theme().background.opacity(0.7)).inset(),
    ]
}

fn compact_bevel_light(cx: &App) -> AnyElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .overflow_hidden()
        .rounded(px(7.))
        .child(
            div()
                .absolute()
                .top(px(-2.))
                .right(px(14.))
                .w(px(120.))
                .h(px(1.))
                .shadow(vec![
                    BoxShadow::new(px(0.), px(7.), cx.theme().primary.opacity(0.34))
                        .blur_radius(px(17.))
                        .spread_radius(px(2.)),
                ]),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left(px(12.))
                .w(px(133.))
                .h(px(1.))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(cx.theme().primary.opacity(0.04), 0.),
                    linear_color_stop(cx.theme().primary.opacity(0.66), 1.),
                )),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left(px(145.))
                .right(px(12.))
                .h(px(1.))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(cx.theme().primary.opacity(0.66), 0.),
                    linear_color_stop(cx.theme().primary.opacity(0.04), 1.),
                )),
        )
        .child(
            div()
                .absolute()
                .top(px(1.))
                .left(px(1.))
                .right(px(1.))
                .h(px(10.))
                .rounded(px(6.))
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(cx.theme().foreground.opacity(0.055), 0.),
                    linear_color_stop(cx.theme().foreground.opacity(0.), 1.),
                )),
        )
        .into_any_element()
}

fn card_bevel_light(cx: &App) -> AnyElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .overflow_hidden()
        .rounded(px(11.))
        .child(
            div()
                .absolute()
                .top(px(-2.))
                .right(px(14.))
                .w(px(130.))
                .h(px(1.))
                .shadow(vec![
                    BoxShadow::new(px(0.), px(13.), cx.theme().primary.opacity(0.38))
                        .blur_radius(px(28.))
                        .spread_radius(px(5.)),
                    BoxShadow::new(px(0.), px(9.), cx.theme().foreground.opacity(0.075))
                        .blur_radius(px(20.))
                        .spread_radius(px(2.)),
                ]),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left(px(10.))
                .w(px(135.))
                .h(px(1.))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(cx.theme().primary.opacity(0.08), 0.),
                    linear_color_stop(cx.theme().primary.opacity(0.76), 1.),
                )),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left(px(145.))
                .right(px(10.))
                .h(px(1.))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(cx.theme().primary.opacity(0.76), 0.),
                    linear_color_stop(cx.theme().primary.opacity(0.08), 1.),
                )),
        )
        .child(
            div()
                .absolute()
                .top(px(1.))
                .left(px(1.))
                .right(px(1.))
                .h(px(28.))
                .rounded(px(10.))
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(cx.theme().foreground.opacity(0.045), 0.),
                    linear_color_stop(cx.theme().foreground.opacity(0.), 1.),
                )),
        )
        .child(
            div()
                .absolute()
                .bottom_0()
                .left(px(1.))
                .right(px(1.))
                .h(px(18.))
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(cx.theme().background.opacity(0.), 0.),
                    linear_color_stop(cx.theme().background.opacity(0.28), 1.),
                )),
        )
        .into_any_element()
}
