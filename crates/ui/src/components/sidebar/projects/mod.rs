use super::*;

impl SidebarView {
    pub(super) fn render_projects(&self, view: &Entity<Self>, cx: &App) -> AnyElement {
        let add_view = view.clone();
        let mut section = v_flex().w_full().gap(px(2.)).pt(px(7.)).child(
            h_flex()
                .h(SECTION_LABEL_HEIGHT)
                .items_center()
                .px(px(8.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(10.))
                        .font_medium()
                        .text_color(cx.theme().muted_foreground.opacity(0.78))
                        .child("Projects"),
                )
                .child(
                    Button::new("add-project")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Plus)
                        .tooltip("Add project")
                        .accessibility_id("add-project")
                        .on_click(move |_, _, cx| {
                            add_view.update(cx, |_, cx| cx.emit(SidebarEvent::AddProject));
                        }),
                ),
        );

        if self.projects.is_empty() {
            return section
                .child(
                    div()
                        .px(px(8.))
                        .pb(px(4.))
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child("Add a workspace for agent conversations."),
                )
                .into_any_element();
        }

        for project in &self.projects {
            section = section.child(self.project_row(project, view, cx));
            if self.expanded_projects.contains(&project.root) {
                for conversation in self.project_conversations(&project.root) {
                    section = section.child(div().pl(px(24.)).child(self.conversation_row(
                        conversation,
                        view.clone(),
                        cx,
                    )));
                }
            }
        }
        section.into_any_element()
    }

    fn project_row(
        &self,
        project: &magenta_core::Project,
        view: &Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let has_conversations = !self.project_conversations(&project.root).is_empty();
        let expanded = self.expanded_projects.contains(&project.root);
        let active = self.active_project.as_ref() == Some(&project.root);
        let group_name: SharedString = format!("project-row-{}", project.root.display()).into();
        let active_background = cx.theme().sidebar_accent;
        let hover_background = if active {
            cx.theme().sidebar_accent
        } else {
            cx.theme().sidebar_accent.opacity(0.72)
        };
        let disclosure = Self::project_disclosure(project, expanded, has_conversations, view);
        let project_button =
            Self::project_button(project, expanded, has_conversations, active, view, cx);
        let actions = Self::project_actions(project, active, group_name.clone(), view);

        h_flex()
            .relative()
            .group(group_name)
            .w_full()
            .h(ROW_HEIGHT)
            .items_center()
            .rounded(ROW_RADIUS)
            .when(active, |this| this.bg(active_background))
            .hover(move |this| this.bg(hover_background))
            .when(active, |this| {
                this.child(
                    div()
                        .absolute()
                        .left(px(0.))
                        .top(px(8.))
                        .bottom(px(8.))
                        .w(px(2.))
                        .rounded_full()
                        .bg(cx.theme().foreground.opacity(0.7)),
                )
            })
            .child(disclosure)
            .child(project_button)
            .child(actions)
            .into_any_element()
    }

    fn project_disclosure(
        project: &magenta_core::Project,
        expanded: bool,
        has_conversations: bool,
        view: &Entity<Self>,
    ) -> AnyElement {
        if !has_conversations {
            return div().flex_none().size(px(24.)).into_any_element();
        }

        let root = project.root.clone();
        let view = view.clone();
        Button::new(format!("project-disclosure-{}", project.root.display()))
            .ghost()
            .xsmall()
            .size(px(22.))
            .p_0()
            .rounded(px(6.))
            .icon(if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .tooltip(if expanded {
                "Collapse project"
            } else {
                "Expand project"
            })
            .on_click(move |_, _, cx| {
                view.update(cx, |sidebar, cx| {
                    sidebar.toggle_project_expanded(root.clone(), cx);
                });
            })
            .into_any_element()
    }

    fn project_button(
        project: &magenta_core::Project,
        expanded: bool,
        has_conversations: bool,
        active: bool,
        view: &Entity<Self>,
        cx: &App,
    ) -> Button {
        let project_for_click = project.clone();
        let view = view.clone();
        Button::new(format!("project-{}", project.root.display()))
            .ghost()
            .flex_1()
            .min_w_0()
            .h_full()
            .px(px(3.))
            .rounded(ROW_RADIUS)
            .text_color(if active {
                cx.theme().sidebar_accent_foreground
            } else {
                cx.theme().sidebar_foreground
            })
            .child(
                h_flex()
                    .w_full()
                    .gap(px(7.))
                    .child(
                        Icon::new(if expanded && has_conversations {
                            IconName::FolderOpen
                        } else {
                            IconName::Folder
                        })
                        .xsmall(),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(13.))
                            .when(active, gpui_kit::component::StyledExt::font_medium)
                            .child(project.name.clone()),
                    ),
            )
            .on_click(move |_, _, cx| {
                view.update(cx, |sidebar, cx| {
                    sidebar.activate_project(project_for_click.clone(), cx);
                });
            })
    }

    fn project_actions(
        project: &magenta_core::Project,
        active: bool,
        group_name: SharedString,
        view: &Entity<Self>,
    ) -> AnyElement {
        let root = project.root.clone();
        let view = view.clone();
        div()
            .flex_none()
            .size(px(28.))
            .when(!active, |this| {
                this.invisible()
                    .group_hover(group_name, gpui_kit::Styled::visible)
            })
            .child(
                Button::new(format!("project-more-{}", project.root.display()))
                    .ghost()
                    .xsmall()
                    .size(px(28.))
                    .p_0()
                    .icon(IconName::Ellipsis)
                    .tooltip("Project actions")
                    .dropdown_menu(move |menu, window, _| {
                        menu.item(PopupMenuItem::new("Forget project").on_click(
                            window.listener_for(&view, {
                                let root = root.clone();
                                move |_, _, _, cx| {
                                    cx.emit(SidebarEvent::ForgetProject(root.clone()));
                                }
                            }),
                        ))
                    }),
            )
            .into_any_element()
    }
}
