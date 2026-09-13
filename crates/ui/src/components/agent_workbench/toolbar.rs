use super::*;

impl AgentWorkbench {
    fn render_context_status(tab: &WorkbenchTab, narrow: bool, cx: &App) -> Option<AnyElement> {
        if let Some(status) = tab.state.label() {
            return Some(
                div()
                    .flex_none()
                    .text_size(px(10.))
                    .font_medium()
                    .text_color(cx.theme().muted_foreground)
                    .child(status.to_owned())
                    .into_any_element(),
            );
        }
        if let Some(diff) = &tab.diff {
            let status = diff.status_label();
            return Some(
                Button::new("workbench-change-status")
                    .ghost()
                    .xsmall()
                    .compact()
                    .icon(diff.status_icon())
                    .when(!narrow, |this| this.label(status.clone()))
                    .text_color(diff.status_color(cx))
                    .accessibility_label(status.clone())
                    .tooltip(status)
                    .into_any_element(),
            );
        }
        None
    }

    fn render_view_mode_selector(view: Entity<Self>, mode: WorkbenchViewMode) -> AnyElement {
        let toggle_view = view;
        div()
            .flex_none()
            .debug_selector(|| "workbench-view-mode".into())
            .child(
                ToggleGroup::new("workbench-view-mode")
                    .segmented()
                    .with_size(gpui_kit::component::Size::XSmall)
                    .child(
                        Toggle::new("workbench-file-mode")
                            .label("File")
                            .checked(mode == WorkbenchViewMode::File)
                            .tooltip("Show file"),
                    )
                    .child(
                        Toggle::new("workbench-diff-mode")
                            .label("Diff")
                            .checked(mode == WorkbenchViewMode::Diff)
                            .tooltip("Show diff"),
                    )
                    .on_click(move |checks, window, cx| {
                        let next_checks = (
                            checks.first().copied().unwrap_or(false),
                            checks.get(1).copied().unwrap_or(false),
                        );
                        let next_mode = match next_checks {
                            (true, false) => WorkbenchViewMode::File,
                            (false, true) => WorkbenchViewMode::Diff,
                            (false, false) => mode,
                            (true, true) => match mode {
                                WorkbenchViewMode::File => WorkbenchViewMode::Diff,
                                WorkbenchViewMode::Diff => WorkbenchViewMode::File,
                            },
                        };
                        toggle_view.update(cx, |workbench, cx| {
                            workbench.set_view_mode(next_mode, window, cx);
                        });
                    }),
            )
            .into_any_element()
    }

    fn render_hunk_controls(view: Entity<Self>, diff: &WorkbenchDiff, cx: &App) -> AnyElement {
        let total = diff.hunk_lines.len();
        let current = if total > 0 { diff.selected_hunk + 1 } else { 0 };
        let previous_disabled = total == 0 || diff.selected_hunk == 0;
        let next_disabled = total == 0 || diff.selected_hunk + 1 >= total;
        let previous_view = view.clone();
        h_flex()
            .items_center()
            .gap(px(1.))
            .child(
                Button::new("workbench-previous-hunk")
                    .ghost()
                    .xsmall()
                    .compact()
                    .icon(IconName::ChevronLeft)
                    .debug_selector(|| "workbench-previous-hunk".into())
                    .disabled(previous_disabled)
                    .accessibility_label("Previous hunk")
                    .tooltip("Previous hunk (Shift+F7)")
                    .on_click(move |_, window, cx| {
                        previous_view.update(cx, |workbench, cx| {
                            workbench.navigate_hunk(-1, window, cx);
                        });
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .min_w(px(38.))
                    .text_center()
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{current} / {total}")),
            )
            .child({
                let next_view = view;
                Button::new("workbench-next-hunk")
                    .ghost()
                    .xsmall()
                    .compact()
                    .icon(IconName::ChevronRight)
                    .debug_selector(|| "workbench-next-hunk".into())
                    .disabled(next_disabled)
                    .accessibility_label("Next hunk")
                    .tooltip("Next hunk (F7)")
                    .on_click(move |_, window, cx| {
                        next_view.update(cx, |workbench, cx| {
                            workbench.navigate_hunk(1, window, cx);
                        });
                    })
            })
            .into_any_element()
    }

    pub(super) fn render_context_bar(
        &self,
        view: Entity<Self>,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let Some(path) = self.active_path.as_deref() else {
            return div().h(px(28.)).flex_none().into_any_element();
        };
        let Some(index) = self.tab_index(path) else {
            return div().h(px(28.)).flex_none().into_any_element();
        };
        let tab = &self.tabs[index];
        let narrow = window.viewport_size().width < px(696.);
        let has_diff = tab.diff.as_ref().is_some_and(WorkbenchDiff::has_content);
        let mode = tab.mode;
        let mut bar = h_flex()
            .h(px(30.))
            .flex_none()
            .min_w_0()
            .gap(px(5.))
            .px(px(10.))
            .border_b_1()
            .border_color(cx.theme().border.opacity(0.72))
            .bg(cx.theme().popover.opacity(0.62))
            .child(
                div().flex_1().min_w_0().overflow_hidden().child(
                    Breadcrumb::new()
                        .text_size(px(11.))
                        .children(path.split('/').map(BreadcrumbItem::new)),
                ),
            );
        if let Some(status) = Self::render_context_status(tab, narrow, cx) {
            bar = bar.child(status);
        }
        if has_diff {
            bar = bar.child(Self::render_view_mode_selector(view.clone(), mode));
            if mode == WorkbenchViewMode::Diff
                && let Some(diff) = tab.diff.as_ref()
            {
                bar = bar.child(Self::render_hunk_controls(view, diff, cx));
            }
        }
        bar.into_any_element()
    }
}
