use super::*;

mod render;

impl AgentWorkbench {
    pub fn show_section(
        &mut self,
        section: WorkbenchSection,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.section = section;
        self.explorer_open = true;

        if section == WorkbenchSection::Changes {
            self.refresh_repository(cx);
            self.start_repository_poll(window, cx);
        } else {
            self.repository_poll_task.take();
        }

        cx.notify();
    }

    fn start_repository_poll(&mut self, window: &Window, cx: &Context<'_, Self>) {
        if self.repository_poll_task.is_some() {
            return;
        }

        self.repository_poll_task = Some(cx.spawn_in(window, async move |view, window| {
            loop {
                window
                    .background_executor()
                    .timer(Duration::from_secs(2))
                    .await;

                let keep_polling = view
                    .update_in(window, |workbench, _, cx| {
                        if workbench.section != WorkbenchSection::Changes
                            || workbench.project.is_none()
                        {
                            workbench.repository_poll_task = None;
                            return false;
                        }

                        workbench.refresh_repository(cx);
                        true
                    })
                    .unwrap_or(false);

                if !keep_polling {
                    break;
                }
            }
        }));
    }

    pub fn begin_agent_run(&mut self, cx: &mut Context<'_, Self>) {
        self.run_changes.clear();
        self.run_baseline = self
            .repository_status
            .as_ref()
            .map(|status| {
                status
                    .changes
                    .iter()
                    .map(|change| change.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.refresh_repository(cx);
    }

    pub(super) fn refresh_repository(&mut self, cx: &Context<'_, Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        self.repository_generation = self.repository_generation.wrapping_add(1);
        let generation = self.repository_generation;
        self.repository_loading = true;
        self.repository_error = None;
        let repository = Arc::clone(&self.repository);
        self.repository_task = Some(cx.spawn(async move |view, cx| {
            let result = repository.status(project.root).await;
            _ = view.update(cx, |workbench, cx| {
                if workbench.repository_generation != generation {
                    return;
                }
                workbench.repository_task = None;
                workbench.repository_loading = false;
                match result {
                    Ok(status) => workbench.repository_status = Some(status),
                    Err(error) => {
                        workbench.repository_error = Some(repository_error_message(&error));
                    }
                }
                cx.notify();
            });
        }));
    }

    fn mutate_repository_paths(
        &mut self,
        paths: Vec<String>,
        stage: bool,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let repository = Arc::clone(&self.repository);
        self.repository_error = None;
        self.pending_commit = None;

        self.repository_operation = Some(cx.spawn(async move |view, cx| {
            let result = if stage {
                repository.stage(project.root, paths).await
            } else {
                repository.unstage(project.root, paths).await
            };
            _ = view.update(cx, |workbench, cx| {
                workbench.repository_operation = None;
                if let Err(error) = result {
                    workbench.repository_error = Some(repository_error_message(&error));
                }
                workbench.refresh_repository(cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn prepare_commit(&mut self, cx: &mut Context<'_, Self>) {
        let message = self.commit_input.read(cx).value().trim().to_owned();

        if message.is_empty() {
            self.commit_error = Some("Enter a commit message first.".to_owned());
            cx.notify();
            return;
        }

        self.commit_error = None;
        self.pending_commit = Some(message);
        cx.notify();
    }

    fn commit_repository(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let Some(message) = self.pending_commit.take() else {
            return;
        };

        let repository = Arc::clone(&self.repository);
        self.commit_error = None;
        self.last_commit = None;

        self.repository_operation = Some(cx.spawn_in(window, async move |view, window| {
            let result = repository.commit(project.root, message).await;
            _ = view.update_in(window, |workbench, window, cx| {
                workbench.repository_operation = None;
                match result {
                    Ok(commit) => {
                        workbench.last_commit =
                            Some(format!("Committed {} · {}", commit.oid, commit.summary));
                        workbench
                            .commit_input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                    }
                    Err(error) => {
                        workbench.commit_error = Some(repository_error_message(&error));
                    }
                }

                workbench.refresh_repository(cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn open_repository_diff(
        &self,
        path: String,
        area: RepositoryDiffArea,
        window: &Window,
        cx: &Context<'_, Self>,
    ) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let repository = Arc::clone(&self.repository);
        let session_generation = self.session_generation;
        cx.spawn_in(window, async move |view, window| {
            let result = repository.diff(project.root, path.clone(), area).await;
            _ = view.update_in(window, |workbench, window, cx| {
                if workbench.session_generation != session_generation {
                    return;
                }
                match result {
                    Ok(repository_diff) => {
                        let diff_text = if repository_diff.binary {
                            "Binary or oversized file; a textual diff is unavailable.\n".to_owned()
                        } else if repository_diff.unified_diff.is_empty() {
                            "No textual changes in this area.\n".to_owned()
                        } else {
                            repository_diff.unified_diff
                        };
                        let hunk_lines = parse_hunk_lines(&diff_text);
                        let diff_editor = Self::new_editor(window, cx, "diff");
                        apply_diff(&diff_editor, &diff_text, window, cx);
                        let file_editor = Self::new_editor(window, cx, "text");
                        if let Some(index) = workbench.tab_index(&path) {
                            let tab = &mut workbench.tabs[index];
                            tab.diff = Some(WorkbenchDiff {
                                editor: Some(diff_editor),
                                raw: diff_text,
                                call_id: String::new(),
                                kind: WorkspaceChangeKind::Modify,
                                state: WorkspaceChangeState::Committed,
                                error: None,
                                hunk_lines,
                                selected_hunk: 0,
                                repository_area: Some(area),
                            });
                            tab.mode = WorkbenchViewMode::Diff;
                            tab.state = TabLoadState::Ready;
                        } else {
                            workbench.tabs.push(WorkbenchTab {
                                path: path.clone(),
                                editor: file_editor,
                                diff: Some(WorkbenchDiff {
                                    editor: Some(diff_editor),
                                    raw: diff_text,
                                    call_id: String::new(),
                                    kind: WorkspaceChangeKind::Modify,
                                    state: WorkspaceChangeState::Committed,
                                    error: None,
                                    hunk_lines,
                                    selected_hunk: 0,
                                    repository_area: Some(area),
                                }),
                                mode: WorkbenchViewMode::Diff,
                                state: TabLoadState::Ready,
                                load_generation: 0,
                            });
                        }
                        workbench.activate_tab(path, cx);
                    }
                    Err(error) => {
                        workbench.repository_error = Some(repository_error_message(&error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_explorer(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        let section = self.section;
        let switch_view = view.clone();
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .w_full()
                    .p(px(7.))
                    .border_b_1()
                    .border_color(cx.theme().foreground.opacity(0.06))
                    .child(
                        div()
                            .debug_selector(|| "workbench-section-selector".into())
                            .rounded_full()
                            .border_1()
                            .border_color(cx.theme().foreground.opacity(0.07))
                            .bg(crate::components::visual::surface(
                                crate::components::visual::SurfaceLevel::Recessed,
                                cx,
                            ))
                            .p(px(2.))
                            .child(
                                ToggleGroup::new("workbench-section")
                                    .segmented()
                                    .with_size(gpui_kit::component::Size::Small)
                                    .child(
                                        Toggle::new("workbench-files")
                                            .label("Files")
                                            .w(px(64.))
                                            .checked(section == WorkbenchSection::Files),
                                    )
                                    .child(
                                        Toggle::new("workbench-changes")
                                            .label("Changes")
                                            .w(px(76.))
                                            .checked(section == WorkbenchSection::Changes),
                                    )
                                    .on_click(move |checks, window, cx| {
                                        let selects_changes =
                                            crate::components::select_second_segment(
                                                checks,
                                                section == WorkbenchSection::Changes,
                                            );
                                        let next = if selects_changes {
                                            WorkbenchSection::Changes
                                        } else {
                                            WorkbenchSection::Files
                                        };

                                        switch_view.update(cx, |workbench, cx| {
                                            workbench.show_section(next, window, cx);
                                        });
                                    }),
                            ),
                    ),
            )
            .child(if section == WorkbenchSection::Files {
                self.render_tree(view, cx)
            } else {
                self.render_changes(view, cx)
            })
            .into_any_element()
    }
}

fn repository_error_message(error: &magenta_core::RepositoryError) -> String {
    match error.kind {
        magenta_core::RepositoryErrorKind::GitUnavailable => {
            "Git is not installed or could not be started.".to_owned()
        }
        magenta_core::RepositoryErrorKind::NotRepository => {
            "This workspace is not a Git repository.".to_owned()
        }
        magenta_core::RepositoryErrorKind::WorkspaceIsNotRepositoryRoot => {
            "Open the repository root as the project to use Changes.".to_owned()
        }
        _ => error.source.to_string(),
    }
}
