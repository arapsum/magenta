use std::path::PathBuf;

use gpui::{Context, Window};
use magenta_core::{ConversationMode, Project};

use super::MainView;

impl MainView {
    pub(super) fn refresh_projects(&self, cx: &Context<'_, Self>) {
        let Some(catalog) = self.projects.clone() else {
            return;
        };
        let sidebar = self.sidebar.clone();
        cx.spawn(async move |_, cx| {
            if let Ok(projects) = catalog.list().await {
                sidebar.update(cx, |sidebar, cx| sidebar.set_projects(projects, cx));
            }
        })
        .detach();
    }

    pub(super) fn choose_project(&self, window: &Window, cx: &Context<'_, Self>) {
        let Some(catalog) = self.projects.clone() else {
            return;
        };
        let picker = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add project".into()),
        });
        let sidebar = self.sidebar.clone();
        let workbench = self.workbench.clone();
        cx.spawn_in(window, async move |_, window| {
            let Ok(Ok(paths)) = picker.await else {
                return;
            };
            let Some(paths) = paths else {
                return;
            };
            let Some(root) = paths.into_iter().next() else {
                return;
            };
            let Ok(project) = catalog.add(root).await else {
                return;
            };
            if let Ok(projects) = catalog.list().await {
                _ = sidebar.update_in(window, |sidebar, _, cx| sidebar.set_projects(projects, cx));
            }
            if let Some(workbench) = workbench {
                _ = workbench.update_in(window, |workbench, window, cx| {
                    workbench.set_project(Some(project), window, cx);
                });
            }
        })
        .detach();
    }

    pub(super) fn register_project(&self, root: PathBuf, window: &Window, cx: &Context<'_, Self>) {
        let Some(catalog) = self.projects.clone() else {
            return;
        };
        let sidebar = self.sidebar.clone();
        cx.spawn_in(window, async move |_, window| {
            if catalog.add(root).await.is_ok()
                && let Ok(projects) = catalog.list().await
            {
                _ = sidebar.update_in(window, |sidebar, _, cx| sidebar.set_projects(projects, cx));
            }
        })
        .detach();
    }

    pub(super) fn activate_project(
        &mut self,
        project: Project,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.storage_ready.is_ready() {
            return;
        }
        self.cancel_generation(cx);
        self.clear_workbench_session(cx);
        self.active_conversation = None;
        self.conversation.update(cx, super::ConversationView::clear);
        self.composer.update(cx, |composer, cx| {
            composer.activate_workspace(project.root.clone(), cx);
        });
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_active(None, cx);
            sidebar.set_active_project(Some(project.root.clone()), cx);
        });
        if let Some(workbench) = &self.workbench {
            workbench.update(cx, |workbench, cx| {
                workbench.set_project(Some(project.clone()), window, cx);
            });
        }
        if let Some(catalog) = self.projects.clone() {
            let sidebar = self.sidebar.clone();
            cx.spawn_in(window, async move |_, window| {
                let _ = catalog.touch(project).await;
                if let Ok(projects) = catalog.list().await {
                    _ = sidebar
                        .update_in(window, |sidebar, _, cx| sidebar.set_projects(projects, cx));
                }
            })
            .detach();
        }
        self.update_composer_availability(cx);
        cx.notify();
    }

    pub(super) fn forget_project(
        &self,
        root: PathBuf,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(catalog) = self.projects.clone() else {
            return;
        };
        if self.sidebar.read(cx).active_project() == Some(root.clone()) {
            self.sidebar
                .update(cx, |sidebar, cx| sidebar.set_active_project(None, cx));
            self.composer.update(cx, |composer, cx| {
                composer.set_conversation_context(ConversationMode::Chat, None, cx);
            });
        }
        let sidebar = self.sidebar.clone();
        cx.spawn_in(window, async move |_, window| {
            let _ = catalog.forget(root).await;
            if let Ok(projects) = catalog.list().await {
                _ = sidebar.update_in(window, |sidebar, _, cx| sidebar.set_projects(projects, cx));
            }
        })
        .detach();
    }
}
