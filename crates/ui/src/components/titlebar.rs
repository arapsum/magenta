#[cfg(not(target_os = "linux"))]
use gpui::{AnyElement, IntoElement, ParentElement};

#[cfg(target_os = "linux")]
mod linux {
    use gpui::{
        AnyElement, App, Context, Decorations, InteractiveElement, IntoElement, MouseButton,
        ParentElement, Pixels, Render, RenderOnce, StatefulInteractiveElement as _, Styled,
        Subscription, Window, WindowButton, WindowButtonLayout, div, prelude::FluentBuilder as _,
        px,
    };
    use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};

    const LINUX_TITLE_BAR_HEIGHT: Pixels = px(28.0);

    pub(crate) fn render(_title: impl IntoElement) -> AnyElement {
        LinuxTitleBar.into_any_element()
    }

    #[derive(IntoElement)]
    struct LinuxTitleBar;

    struct TitleBarState {
        should_move: bool,
        button_layout_subscription: Subscription,
    }

    impl Render for TitleBarState {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let _ = &self.button_layout_subscription;
            div()
        }
    }

    impl RenderOnce for LinuxTitleBar {
        fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
            let state = window.use_state(cx, |window, _| TitleBarState {
                should_move: false,
                button_layout_subscription: window.observe_button_layout_changed(|window, _| {
                    window.refresh();
                }),
            });

            if matches!(window.window_decorations(), Decorations::Server) || window.is_fullscreen()
            {
                return div().id("title-bar").h(px(0.0)).flex_shrink_0();
            }

            let button_layout = cx
                .button_layout()
                .unwrap_or_else(WindowButtonLayout::linux_default);
            let supported_controls = window.window_controls();

            let left_controls =
                render_controls("left-window-controls", button_layout.left, window, cx);
            let right_controls =
                render_controls("right-window-controls", button_layout.right, window, cx);

            let mut drag_region = div()
                .id("title-bar-drag-region")
                .flex_1()
                .min_w_0()
                .h_full()
                .on_mouse_down(
                    MouseButton::Left,
                    window.listener_for(&state, |state, _, _, _| {
                        state.should_move = true;
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    window.listener_for(&state, |state, _, _, _| {
                        state.should_move = false;
                    }),
                )
                .on_mouse_down_out(window.listener_for(&state, |state, _, _, _| {
                    state.should_move = false;
                }))
                .on_mouse_move(window.listener_for(&state, |state, _, window, _| {
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

            if supported_controls.window_menu {
                drag_region = drag_region.on_mouse_down(MouseButton::Right, |event, window, _| {
                    window.show_window_menu(event.position)
                });
            }

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
                .border_color(if window.is_window_active() {
                    cx.theme().title_bar_border
                } else {
                    cx.theme().title_bar_border.opacity(0.6)
                })
                .when_some(left_controls, |this, controls| this.child(controls))
                .child(drag_region)
                .when_some(right_controls, |this, controls| this.child(controls))
        }
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
            .filter(|button| match button {
                WindowButton::Minimize => supported_controls.minimize && is_minimizable,
                WindowButton::Maximize => supported_controls.maximize && is_resizable,
                WindowButton::Close => true,
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

    fn render_control(button: WindowButton, window: &mut Window, cx: &mut App) -> AnyElement {
        let is_close = matches!(button, WindowButton::Close);
        let is_maximised = window.is_maximized();
        let icon = match button {
            WindowButton::Minimize => IconName::WindowMinimize,
            WindowButton::Maximize if is_maximised => IconName::WindowRestore,
            WindowButton::Maximize => IconName::WindowMaximize,
            WindowButton::Close => IconName::WindowClose,
        };
        let aria_label = match button {
            WindowButton::Minimize => "Minimize window",
            WindowButton::Maximize if is_maximised => "Restore window",
            WindowButton::Maximize => "Maximize window",
            WindowButton::Close => "Close window",
        };

        let hover_background = if is_close {
            cx.theme().danger
        } else {
            cx.theme().secondary_hover
        };
        let active_background = if is_close {
            cx.theme().danger_active
        } else {
            cx.theme().secondary_active
        };
        let hover_foreground = if is_close {
            cx.theme().danger_foreground
        } else {
            cx.theme().secondary_foreground
        };
        let foreground = cx.theme().foreground.opacity(if window.is_window_active() {
            0.72
        } else {
            0.38
        });
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
            .hover(|style| style.bg(hover_background).text_color(hover_foreground))
            .active(|style| style.bg(active_background).text_color(hover_foreground))
            .focus_visible(|style| style.border_1().border_color(cx.theme().ring))
            .role(gpui::Role::Button)
            .aria_label(aria_label)
            .track_focus(&focus_handle)
            .tab_index(0)
            .tab_stop(true)
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                match button {
                    WindowButton::Minimize => window.minimize_window(),
                    WindowButton::Maximize => window.zoom_window(),
                    WindowButton::Close => window.remove_window(),
                }
            })
            .child(Icon::new(icon).small())
            .into_any_element()
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::render;

#[cfg(not(target_os = "linux"))]
pub(crate) fn render(title: impl IntoElement) -> AnyElement {
    gpui_component::TitleBar::new()
        .child(title)
        .into_any_element()
}
