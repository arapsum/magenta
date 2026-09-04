use gpui::{App, ClickEvent, Context, Window};

use crate::app::MainView;

use super::Destination;

pub(super) fn select_handler(
    destination: Destination,
    cx: &Context<MainView>,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |view, _, _, cx| {
        view.select_sidebar_destination(destination, cx);
    })
}
