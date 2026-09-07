use super::*;

impl SidebarView {
    pub(super) fn render_projects(&self, view: &Entity<Self>, cx: &App) -> AnyElement {
        let add_view = view.clone();
        let mut section = v_flex().w_full().gap(px(2.)).child(
            h_flex()
                .h(px(28.))
                .items_center()
                .px(px(8.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(10.))
                        .font_semibold()
                        .text_color(cx.theme().muted_foreground.opacity(0.8))
                        .child("PROJECTS"),
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
                    section = section.child(div().pl(px(18.)).child(self.conversation_row(
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
        let expanded = self.expanded_projects.contains(&project.root);
        let active = self.active_project.as_ref() == Some(&project.root);
        let activate = project.clone();
        let forget_root = project.root.clone();
        let activate_view = view.clone();
        let action_view = view.clone();
        h_flex()
            .w_full()
            .h(ROW_HEIGHT)
            .items_center()
            .rounded(px(6.))
            .when(active, |this| this.bg(cx.theme().sidebar_accent))
            .hover(|this| this.bg(cx.theme().sidebar_accent))
            .child(
                Button::new(format!("project-{}", project.root.display()))
                    .ghost()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .px(px(8.))
                    .rounded(px(6.))
                    .child(
                        h_flex()
                            .w_full()
                            .gap(px(7.))
                            .child(
                                Icon::new(if expanded {
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
                                    .child(project.name.clone()),
                            ),
                    )
                    .on_click(move |_, _, cx| {
                        activate_view.update(cx, |sidebar, cx| {
                            sidebar.activate_project(activate.clone(), cx);
                        });
                    }),
            )
            .child(
                Button::new(format!("project-more-{}", project.root.display()))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Ellipsis)
                    .tooltip("Project actions")
                    .dropdown_menu(move |menu, window, _| {
                        menu.item(PopupMenuItem::new("Forget project").on_click(
                            window.listener_for(&action_view, {
                                let root = forget_root.clone();
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
