use super::*;

impl AgentWorkbench {
    #[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
    fn render_active_content(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        let Some(index) = self
            .active_path
            .as_deref()
            .and_then(|path| self.tab_index(path))
        else {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(42.))
                        .rounded(px(13.))
                        .bg(cx.theme().accent.opacity(0.68))
                        .text_color(cx.theme().primary)
                        .child(Icon::empty().path("icons/code.svg").size(px(20.))),
                )
                .child(
                    div()
                        .text_size(px(16.))
                        .font_semibold()
                        .child("No files open"),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child("Open a file from the explorer."),
                )
                .into_any_element();
        };
        let tab = &self.tabs[index];
        match &tab.state {
            TabLoadState::Loading => Self::render_loading_lines(cx, 9),
            TabLoadState::Failed(error) => {
                let retry_view = view.clone();
                let close_view = view;
                let close_path = tab.path.clone();
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .px(px(24.))
                    .child(
                        h_flex()
                            .items_start()
                            .gap(px(7.))
                            .id("workbench-file-error")
                            .role(gpui_kit::Role::Alert)
                            .aria_label(format!("{}: {}", error.title(), error.message()))
                            .child(Icon::new(IconName::CircleX).small())
                            .child(
                                v_flex()
                                    .gap(px(2.))
                                    .child(
                                        div().text_size(px(13.)).font_medium().child(error.title()),
                                    )
                                    .child(
                                        div()
                                            .max_w(px(480.))
                                            .text_size(px(12.))
                                            .text_color(cx.theme().muted_foreground)
                                            .child(error.message()),
                                    ),
                            ),
                    )
                    .child(
                        Button::new("retry-workbench-file")
                            .ghost()
                            .small()
                            .label("Retry")
                            .on_click(move |_, window, cx| {
                                retry_view.update(cx, |workbench, cx| {
                                    workbench.retry_active_tab(window, cx);
                                });
                            }),
                    )
                    .child(
                        Button::new("close-failed-workbench-file")
                            .ghost()
                            .small()
                            .label("Close tab")
                            .on_click(move |_, _, cx| {
                                close_view.update(cx, |workbench, cx| {
                                    workbench.close_tab(&close_path, cx);
                                });
                            }),
                    )
                    .into_any_element()
            }
            TabLoadState::Ready => {
                let editor = if tab.mode == WorkbenchViewMode::Diff {
                    tab.diff
                        .as_ref()
                        .and_then(|diff| diff.editor.as_ref())
                        .unwrap_or(&tab.editor)
                } else {
                    &tab.editor
                };
                Editor::new(editor)
                    .readonly(true)
                    .bordered(false)
                    .h(relative(1.))
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size)
                    .aria_label(if tab.mode == WorkbenchViewMode::Diff {
                        "Read-only unified diff"
                    } else {
                        "Read-only file"
                    })
                    .into_any_element()
            }
        }
    }

    pub(super) fn render_loading_lines(cx: &App, count: usize) -> AnyElement {
        let widths = [0.42, 0.68, 0.55, 0.76, 0.33, 0.61, 0.49, 0.72, 0.39];
        v_flex()
            .size_full()
            .gap(px(10.))
            .p(px(14.))
            .children((0..count).map(|index| {
                div()
                    .h(px(7.))
                    .w(relative(widths[index % widths.len()]))
                    .rounded(px(3.))
                    .bg(cx.theme().muted.opacity(0.55))
            }))
            .into_any_element()
    }

    fn render_editor_pane(&self, view: Entity<Self>, window: &Window, cx: &App) -> AnyElement {
        v_flex()
            .size_full()
            .min_w_0()
            .child(self.render_tab_bar(&view, cx))
            .child(self.render_context_bar(view.clone(), window, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(self.render_active_content(view, cx)),
            )
            .into_any_element()
    }

    fn render_header(
        project_name: String,
        show_explorer: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        h_flex()
            .flex_none()
            .h(px(38.))
            .items_center()
            .gap(px(7.))
            .px(px(9.))
            .border_b_1()
            .border_color(cx.theme().foreground.opacity(0.07))
            .bg(crate::components::visual::surface(
                crate::components::visual::SurfaceLevel::Raised,
                cx,
            ))
            .child(
                Button::new("toggle-workbench-explorer")
                    .ghost()
                    .xsmall()
                    .icon(if show_explorer {
                        IconName::PanelLeftClose
                    } else {
                        IconName::PanelLeftOpen
                    })
                    .tooltip(if show_explorer {
                        "Hide file explorer"
                    } else {
                        "Show file explorer"
                    })
                    .on_click(cx.listener(|workbench, _, _, cx| {
                        workbench.toggle_explorer(cx);
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(24.))
                    .rounded_full()
                    .bg(cx.theme().primary.opacity(0.16))
                    .text_color(cx.theme().primary)
                    .child(Icon::empty().path("icons/code.svg").xsmall()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.))
                    .font_medium()
                    .child(project_name),
            )
            .child(
                Button::new("close-workbench")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip("Close code view")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(AgentWorkbenchEvent::Closed))),
            )
            .into_any_element()
    }

    fn render_content_layout(
        tree: AnyElement,
        editor: AnyElement,
        narrow: bool,
        show_explorer: bool,
        cx: &App,
    ) -> AnyElement {
        if narrow && show_explorer {
            v_flex()
                .size_full()
                .p(px(4.))
                .bg(crate::components::visual::surface(
                    crate::components::visual::SurfaceLevel::Raised,
                    cx,
                ))
                .child(tree)
                .into_any_element()
        } else if show_explorer {
            h_resizable("agent-workbench")
                .child(
                    resizable_panel()
                        .size(px(240.))
                        .size_range(px(180.)..px(360.))
                        .child(
                            v_flex()
                                .size_full()
                                .p(px(4.))
                                .border_r_1()
                                .border_color(cx.theme().foreground.opacity(0.06))
                                .bg(crate::components::visual::surface(
                                    crate::components::visual::SurfaceLevel::Raised,
                                    cx,
                                ))
                                .child(tree),
                        ),
                )
                .child(resizable_panel().child(editor))
                .into_any_element()
        } else {
            editor
        }
    }
}

impl Render for AgentWorkbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let view = cx.entity();
        let project_name = self
            .project
            .as_ref()
            .map_or_else(|| "Code".to_owned(), |project| project.name.clone());
        let narrow = window.viewport_size().width < px(696.);
        let show_explorer = self.explorer_open;
        let tree = self.render_explorer(view.clone(), cx);
        let editor = self.render_editor_pane(view, window, cx);
        let header = Self::render_header(project_name, show_explorer, cx);
        let content = Self::render_content_layout(tree, editor, narrow, show_explorer, cx);
        v_flex()
            .id("agent-workbench")
            .debug_selector(|| "agent-workbench".into())
            .key_context("AgentWorkbench")
            .on_action(cx.listener(|workbench, _: &SelectNextWorkbenchTab, _, cx| {
                workbench.cycle_tab(1, cx);
            }))
            .on_action(
                cx.listener(|workbench, _: &SelectPreviousWorkbenchTab, _, cx| {
                    workbench.cycle_tab(-1, cx);
                }),
            )
            .on_action(
                cx.listener(|workbench, _: &CloseActiveWorkbenchTab, _, cx| {
                    workbench.close_active_tab(cx);
                }),
            )
            .on_action(
                cx.listener(|workbench, _: &ToggleWorkbenchViewMode, window, cx| {
                    workbench.toggle_view_mode(window, cx);
                }),
            )
            .on_action(cx.listener(|workbench, _: &NextWorkbenchHunk, window, cx| {
                workbench.navigate_hunk(1, window, cx);
            }))
            .on_action(
                cx.listener(|workbench, _: &PreviousWorkbenchHunk, window, cx| {
                    workbench.navigate_hunk(-1, window, cx);
                }),
            )
            .size_full()
            .min_w_0()
            .bg(crate::components::visual::surface(
                crate::components::visual::SurfaceLevel::Recessed,
                cx,
            ))
            .child(header)
            .child(content)
    }
}
