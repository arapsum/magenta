mod destination;
mod footer;
mod header;
mod interaction;
mod navigation;
mod styles;

use gpui::{AnyElement, Context, IntoElement as _, Styled as _, px};
use gpui_component::sidebar::{Sidebar, SidebarCollapsible};

use crate::app::MainView;

use self::navigation::SidebarBody;

pub(crate) use destination::Destination;

const EXPANDED_WIDTH: gpui::Pixels = px(240.);

pub(crate) fn render(
    collapsed: bool,
    active: Destination,
    cx: &mut Context<MainView>,
) -> AnyElement {
    Sidebar::new("magenta-sidebar")
        .w(EXPANDED_WIDTH)
        .collapsible(SidebarCollapsible::Icon)
        .collapsed(collapsed)
        .header(header::render(collapsed, cx))
        .child(SidebarBody::new(active, cx.entity().clone()))
        .footer(footer::render(collapsed, cx))
        .into_any_element()
}
