use gpui::{
    AnyElement, App, ElementId, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Collapsible, Icon, IconName, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    sidebar::SidebarItem,
    v_flex,
};

use crate::app::MainView;

use super::{Destination, styles};

const NAV_ROW_HEIGHT: gpui::Pixels = px(36.);

#[derive(Clone)]
pub(super) struct SidebarBody {
    collapsed: bool,
    active: Destination,
    view: Entity<MainView>,
}

impl SidebarBody {
    pub(super) fn new(active: Destination, view: Entity<MainView>) -> Self {
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
        .bg(gpui::linear_gradient(
            180.,
            gpui::linear_color_stop(bevel_top.opacity(0.98), 0.),
            gpui::linear_color_stop(bevel_bottom.opacity(0.99), 1.),
        ))
        .shadow(styles::illuminated_button_shadow(cx))
        .text_color(cx.theme().foreground)
        .when(body.collapsed, |this| {
            this.size(px(32.))
                .p_0()
                .rounded(px(8.))
                .tooltip(Destination::NewChat.label())
        })
        .child(styles::compact_bevel_light(cx))
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
