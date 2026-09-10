use super::*;

pub(super) type AccountMenuBuilder =
    Box<dyn FnOnce(Entity<PopoverState>, &mut Window, &mut App) -> AnyElement>;

#[derive(IntoElement)]
pub(super) struct AccountDropdown {
    id: ElementId,
    trigger: Button,
    menu: AccountMenuBuilder,
}

#[derive(Clone, Copy, Default)]
struct AccountDropdownAnchor {
    bounds: Bounds<gpui_kit::Pixels>,
    captured: bool,
}

impl AccountDropdown {
    pub(super) fn new(
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
