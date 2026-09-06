#[cfg(not(target_os = "linux"))]
use gpui::{AnyElement, IntoElement, ParentElement, Styled as _, px};
#[cfg(not(target_os = "linux"))]
use gpui_component::h_flex;
#[cfg(target_os = "linux")]
mod linux {
    use gpui::{
        AnyElement, App, Context, Decorations, Entity, Hsla, InteractiveElement, IntoElement,
        MouseButton, ParentElement, Pixels, Render, RenderOnce, StatefulInteractiveElement as _,
        Styled, Subscription, Window, WindowButton, WindowButtonLayout, WindowControls, div,
        prelude::FluentBuilder as _, px,
    };
    use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};

    const LINUX_TITLE_BAR_HEIGHT: Pixels = px(32.0);

    pub fn render(controls: impl IntoElement) -> AnyElement {
        LinuxTitleBar {
            controls: controls.into_any_element(),
        }
        .into_any_element()
    }

    #[derive(IntoElement)]
    struct LinuxTitleBar {
        controls: AnyElement,
    }
    struct TitleBarState {
        should_move: bool,
        button_layout_subscription: Subscription,
    }

    impl Render for TitleBarState {
        fn render(
            &mut self,
            _window: &mut Window,
            _cx: &mut Context<'_, Self>,
        ) -> impl IntoElement {
            let _ = &self.button_layout_subscription;
            div()
        }
    }

    impl RenderOnce for LinuxTitleBar {
        fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
            let state = title_bar_state(window, cx);
            if title_bar_hidden(window) {
                return hidden_title_bar();
            }

            render_title_bar(&state, self.controls, window, cx)
        }
    }

    fn title_bar_state(window: &mut Window, cx: &mut App) -> Entity<TitleBarState> {
        window.use_state(cx, |window, _| TitleBarState {
            should_move: false,
            button_layout_subscription: window.observe_button_layout_changed(|window, _| {
                window.refresh();
            }),
        })
    }

    fn title_bar_hidden(window: &Window) -> bool {
        matches!(window.window_decorations(), Decorations::Server) || window.is_fullscreen()
    }

    fn hidden_title_bar() -> AnyElement {
        div()
            .id("title-bar")
            .h(px(0.0))
            .flex_shrink_0()
            .into_any_element()
    }

    fn render_title_bar(
        state: &Entity<TitleBarState>,
        controls: AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let button_layout = cx
            .button_layout()
            .unwrap_or_else(WindowButtonLayout::linux_default);
        let supports_window_menu = window.window_controls().window_menu;
        let left_control_id = "left-window-controls";
        let left_controls = render_controls(left_control_id, button_layout.left, window, cx);
        let right_controls =
            render_controls("right-window-controls", button_layout.right, window, cx);
        let drag_region = drag_region(state, supports_window_menu, window);

        div()
            .id("title-bar")
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(LINUX_TITLE_BAR_HEIGHT)
            .flex_shrink_0()
            .bg(cx.theme().title_bar)
            .border_b_1()
            .border_color(title_bar_border(window, cx))
            .when_some(left_controls, gpui::ParentElement::child)
            .child(controls)
            .child(drag_region)
            .when_some(right_controls, gpui::ParentElement::child)
            .into_any_element()
    }

    fn title_bar_border(window: &Window, cx: &App) -> Hsla {
        if window.is_window_active() {
            cx.theme().title_bar_border
        } else {
            cx.theme().title_bar_border.opacity(0.6)
        }
    }

    fn drag_region(
        state: &Entity<TitleBarState>,
        supports_window_menu: bool,
        window: &Window,
    ) -> AnyElement {
        let mut drag_region = div()
            .id("title-bar-drag-region")
            .flex_1()
            .min_w_0()
            .h_full()
            .on_mouse_down(
                MouseButton::Left,
                window.listener_for(state, |state, _, _, _| {
                    state.should_move = true;
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                window.listener_for(state, |state, _, _, _| {
                    state.should_move = false;
                }),
            )
            .on_mouse_down_out(window.listener_for(state, |state, _, _, _| {
                state.should_move = false;
            }))
            .on_mouse_move(window.listener_for(state, |state, _, window, _| {
                if state.should_move {
                    state.should_move = false;
                    window.start_window_move();
                }
            }))
            .on_click(|event, window, _| {
                if event.click_count() == 2
                    && window.window_controls().maximize
                    && window.is_resizable()
                {
                    window.zoom_window();
                }
            });

        if supports_window_menu {
            drag_region = drag_region.on_mouse_down(MouseButton::Right, |event, window, _| {
                window.show_window_menu(event.position);
            });
        }

        drag_region.into_any_element()
    }

    fn render_controls(
        id: &'static str,
        buttons: [Option<WindowButton>; 3],
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let supported_controls = window.window_controls();
        let is_minimizable = window.is_minimizable();
        let is_resizable = window.is_resizable();

        let buttons = buttons
            .into_iter()
            .flatten()
            .filter(|button| {
                is_supported_control(*button, supported_controls, is_minimizable, is_resizable)
            })
            .map(|button| render_control(button, window, cx))
            .collect::<Vec<_>>();

        (!buttons.is_empty()).then(|| {
            h_flex()
                .id(id)
                .items_center()
                .gap(px(2.0))
                .px(px(4.0))
                .flex_shrink_0()
                .children(buttons)
                .into_any_element()
        })
    }

    const fn is_supported_control(
        button: WindowButton,
        supported_controls: WindowControls,
        is_minimizable: bool,
        is_resizable: bool,
    ) -> bool {
        match button {
            WindowButton::Minimize => supported_controls.minimize && is_minimizable,
            WindowButton::Maximize => supported_controls.maximize && is_resizable,
            WindowButton::Close => true,
        }
    }

    fn render_control(button: WindowButton, window: &mut Window, cx: &mut App) -> AnyElement {
        let details = control_details(button, window.is_maximized());
        let colors = control_colors(details.is_close, cx);
        let foreground = control_foreground(window, cx);
        let focus_handle = window
            .use_keyed_state(button.id(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone()
            .tab_index(0)
            .tab_stop(true);

        h_flex()
            .id(button.id())
            .w(px(24.0))
            .h(px(24.0))
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .text_color(foreground)
            .hover(|style| {
                style
                    .bg(colors.hover_background)
                    .text_color(colors.hover_foreground)
            })
            .active(|style| {
                style
                    .bg(colors.active_background)
                    .text_color(colors.hover_foreground)
            })
            .focus_visible(|style| style.border_1().border_color(cx.theme().ring))
            .role(gpui::Role::Button)
            .aria_label(details.aria_label)
            .track_focus(&focus_handle)
            .tab_index(0)
            .tab_stop(true)
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                activate_control(button, window, cx);
            })
            .child(Icon::new(details.icon).small())
            .into_any_element()
    }

    struct ControlDetails {
        icon: IconName,
        aria_label: &'static str,
        is_close: bool,
    }

    const fn control_details(button: WindowButton, is_maximised: bool) -> ControlDetails {
        match button {
            WindowButton::Minimize => ControlDetails {
                icon: IconName::WindowMinimize,
                aria_label: "Minimize window",
                is_close: false,
            },
            WindowButton::Maximize if is_maximised => ControlDetails {
                icon: IconName::WindowRestore,
                aria_label: "Restore window",
                is_close: false,
            },
            WindowButton::Maximize => ControlDetails {
                icon: IconName::WindowMaximize,
                aria_label: "Maximize window",
                is_close: false,
            },
            WindowButton::Close => ControlDetails {
                icon: IconName::WindowClose,
                aria_label: "Close window",
                is_close: true,
            },
        }
    }

    struct ControlColors {
        hover_background: Hsla,
        active_background: Hsla,
        hover_foreground: Hsla,
    }

    fn control_colors(is_close: bool, cx: &App) -> ControlColors {
        if is_close {
            ControlColors {
                hover_background: cx.theme().danger,
                active_background: cx.theme().danger_active,
                hover_foreground: cx.theme().danger_foreground,
            }
        } else {
            ControlColors {
                hover_background: cx.theme().secondary_hover,
                active_background: cx.theme().secondary_active,
                hover_foreground: cx.theme().secondary_foreground,
            }
        }
    }

    fn control_foreground(window: &Window, cx: &App) -> Hsla {
        cx.theme().foreground.opacity(if window.is_window_active() {
            0.72
        } else {
            0.38
        })
    }

    fn activate_control(button: WindowButton, window: &mut Window, cx: &mut App) {
        match button {
            WindowButton::Minimize => window.minimize_window(),
            WindowButton::Maximize => window.zoom_window(),
            WindowButton::Close => window.dispatch_action(Box::new(super::CloseWindow), cx),
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::render;

#[cfg(not(target_os = "linux"))]
pub fn render(controls: impl IntoElement) -> AnyElement {
    gpui_component::TitleBar::new()
        .h(px(32.))
        .child(
            h_flex()
                .h_full()
                .flex_1()
                .min_w_0()
                .items_center()
                .child(controls),
        )
        .into_any_element()
}
#[derive(Clone, Debug, Default, Eq, PartialEq, gpui::Action)]
#[action(namespace = magenta)]
pub struct CloseWindow;
