use super::*;

impl AgentWorkbench {
    pub(super) fn render_tree(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        if let Some((_, error)) = self.tree_error.clone() {
            let retry_view = view;
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .px(px(14.))
                .text_center()
                .id("workbench-tree-error")
                .role(gpui_kit::Role::Alert)
                .aria_label(error.clone())
                .child(Icon::new(IconName::CircleX).small())
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child(error),
                )
                .child(
                    Button::new("retry-workbench-tree")
                        .ghost()
                        .small()
                        .label("Retry")
                        .on_click(move |_, _, cx| {
                            retry_view.update(cx, Self::retry_tree);
                        }),
                )
                .into_any_element();
        }
        if !self.directories.contains_key("") {
            return Self::render_loading_lines(cx, 5);
        }
        let tree_state = self.tree_state.clone();
        tree(&tree_state, move |index, entry, selected, _window, cx| {
            let item = entry.item().clone();
            let path = item.id.to_string();
            let depth = entry.depth();
            let is_folder = entry.is_folder();
            let is_expanded = entry.is_expanded();
            let view = view.clone();
            ListItem::new(index)
                .w_full()
                .px(px(8.))
                .pl(px(8.) + px(14.) * depth)
                .rounded(px(5.))
                .when(selected, |this| this.bg(cx.theme().sidebar_accent))
                .child(
                    h_flex()
                        .gap(px(6.))
                        .child(
                            Icon::new(if is_folder {
                                if is_expanded {
                                    IconName::FolderOpen
                                } else {
                                    IconName::Folder
                                }
                            } else {
                                IconName::File
                            })
                            .xsmall(),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(item.label),
                        ),
                )
                .on_click(move |_, window, cx| {
                    view.update(cx, |workbench, cx| {
                        workbench
                            .tree_state
                            .update(cx, |state, cx| state.set_selected_index(Some(index), cx));
                        if !is_folder {
                            workbench.open_entry(path.clone(), window, cx);
                        }
                    });
                })
        })
        .into_any_element()
    }

    pub(super) fn render_tab_bar(&self, view: &Entity<Self>, cx: &App) -> AnyElement {
        let active_index = self
            .active_path
            .as_deref()
            .and_then(|path| self.tab_index(path));
        if self.tabs.is_empty() {
            return div()
                .h(px(36.))
                .flex_none()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().tokens.tab_bar)
                .into_any_element();
        }
        let paths = self
            .tabs
            .iter()
            .map(|tab| tab.path.clone())
            .collect::<Vec<_>>();
        let tab_view = view.clone();
        TabBar::new("workbench-tabs")
            .w_full()
            .with_size(gpui_kit::component::Size::Small)
            .max_width(px(190.))
            .menu(true)
            .when_some(active_index, TabBar::selected_index)
            .on_click(move |index: &usize, _, cx| {
                if let Some(path) = paths.get(*index) {
                    tab_view.update(cx, |workbench, cx| {
                        workbench.activate_tab(path.clone(), cx);
                    });
                }
            })
            .children(self.tabs.iter().map(|tab| {
                let path = tab.path.clone();
                let close_path = path.clone();
                let close_view = view.clone();
                let accessible_label = self.tab_accessibility_label(&path);
                let status = tab
                    .diff
                    .as_ref()
                    .map(|diff| (diff.status_icon(), diff.status_color(cx)));
                Tab::new()
                    .label(self.tab_label(&path))
                    .aria_label(accessible_label)
                    .when_some(status, |this, (icon, color)| {
                        this.prefix(Icon::new(icon).xsmall().text_color(color))
                    })
                    .suffix(
                        Button::new(format!("close-workbench-tab-{close_path}"))
                            .ghost()
                            .xsmall()
                            .size(px(20.))
                            .p_0()
                            .icon(IconName::Close)
                            .tooltip("Close file")
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                close_view.update(cx, |workbench, cx| {
                                    workbench.close_tab(&close_path, cx);
                                });
                            }),
                    )
            }))
            .into_any_element()
    }

    fn tab_label(&self, path: &str) -> String {
        let paths = self
            .tabs
            .iter()
            .map(|tab| tab.path.as_str())
            .collect::<Vec<_>>();
        shortest_unique_suffix(path, &paths)
    }

    fn tab_accessibility_label(&self, path: &str) -> String {
        let label = self.tab_label(path);
        self.tab_index(path)
            .and_then(|index| self.tabs[index].diff.as_ref())
            .map_or_else(
                || label.clone(),
                |diff| format!("{label}, {}", diff.status_label()),
            )
    }
}
