use super::*;

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
                    .p(px(6.))
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .rounded_full()
                            .border_1()
                            .border_color(cx.theme().border.opacity(0.8))
                            .bg(cx.theme().secondary.opacity(0.72))
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
                                        let next = if checks.get(1).copied().unwrap_or(false) {
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

    fn render_changes(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        if self.repository_status.is_none() && self.repository_loading {
            return Self::render_loading_lines(cx, 5);
        }

        if self.repository_status.is_none() {
            return self.render_repository_unavailable(view, cx);
        }

        let status = self.repository_status.as_ref().unwrap();
        let staged = status
            .changes
            .iter()
            .filter(|change| change.staged.is_some())
            .cloned()
            .collect::<Vec<_>>();
        let unstaged = status
            .changes
            .iter()
            .filter(|change| change.unstaged.is_some())
            .cloned()
            .collect::<Vec<_>>();
        let branch = if status.detached {
            "Detached HEAD".to_owned()
        } else {
            status
                .branch
                .clone()
                .unwrap_or_else(|| "Repository".to_owned())
        };

        let refresh_button = {
            let refresh = view.clone();

            Button::new("refresh-repository")
                .ghost()
                .xsmall()
                .label("Refresh")
                .on_click(move |_, _, cx| {
                    refresh.update(cx, |workbench, cx| {
                        workbench.refresh_repository(cx);
                    });
                })
        };

        let mut content = v_flex()
            .size_full()
            .min_h_0()
            .overflow_y_scrollbar()
            .gap(px(7.))
            .p(px(7.))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_size(px(11.)).font_semibold().child(branch))
                    .child(refresh_button),
            );

        if let Some(error) = &self.repository_error {
            content = content.child(
                div()
                    .text_size(px(10.))
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }

        if staged.is_empty() && unstaged.is_empty() {
            return content
                .child(
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap(px(5.))
                        .child(Icon::new(IconName::CircleCheck).small())
                        .child(div().text_size(px(12.)).child("Working tree clean")),
                )
                .into_any_element();
        }

        if !staged.is_empty() {
            content = content.child(self.render_change_section(
                "STAGED",
                &staged,
                RepositoryDiffArea::Staged,
                &view,
                cx,
            ));
        }

        if !unstaged.is_empty() {
            content = content.child(self.render_change_section(
                "CHANGES",
                &unstaged,
                RepositoryDiffArea::Unstaged,
                &view,
                cx,
            ));
        }

        if !staged.is_empty() {
            content = content.child(self.render_commit_form(staged.len(), view, cx));
        }

        content.into_any_element()
    }

    fn render_repository_unavailable(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        let message = self
            .repository_error
            .clone()
            .unwrap_or_else(|| "Repository status is unavailable.".to_owned());

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .p(px(12.))
            .child(
                div()
                    .text_size(px(12.))
                    .text_center()
                    .text_color(cx.theme().muted_foreground)
                    .child(message),
            )
            .child(
                Button::new("retry-repository-status")
                    .small()
                    .outline()
                    .label("Retry")
                    .on_click(move |_, _, cx| {
                        view.update(cx, |workbench, cx| {
                            workbench.refresh_repository(cx);
                        });
                    }),
            )
            .into_any_element()
    }

    fn render_change_section(
        &self,
        title: &'static str,
        changes: &[magenta_core::RepositoryChange],
        area: RepositoryDiffArea,
        view: &Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let paths = changes
            .iter()
            .map(|change| change.path.clone())
            .collect::<Vec<_>>();
        let mutate_all = Entity::clone(view);
        let stage = area == RepositoryDiffArea::Unstaged;
        let action = if stage { "Stage all" } else { "Unstage all" };

        v_flex()
            .gap(px(3.))
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(10.))
                            .font_semibold()
                            .child(format!("{title} · {}", changes.len())),
                    )
                    .child(
                        Button::new(format!("mutate-all-{title}"))
                            .ghost()
                            .xsmall()
                            .label(action)
                            .disabled(self.repository_operation.is_some())
                            .on_click(move |_, _, cx| {
                                mutate_all.update(cx, |workbench, cx| {
                                    workbench.mutate_repository_paths(paths.clone(), stage, cx);
                                });
                            }),
                    ),
            )
            .children(
                changes
                    .iter()
                    .map(|change| self.render_change_row(change, area, Entity::clone(view), cx)),
            )
            .into_any_element()
    }

    fn render_commit_form(&self, staged_count: usize, view: Entity<Self>, cx: &App) -> AnyElement {
        let review_view = view.clone();
        let commit_view = view.clone();
        let cancel_view = view;

        let mut form = v_flex()
            .gap(px(5.))
            .pt(px(5.))
            .border_t_1()
            .border_color(cx.theme().border)
            .child(Input::new(&self.commit_input).small().appearance(false))
            .when_some(self.commit_error.clone(), |this, error| {
                this.child(
                    div()
                        .text_size(px(10.))
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .when_some(self.last_commit.clone(), |this, message| {
                this.child(
                    div()
                        .text_size(px(10.))
                        .text_color(cx.theme().success)
                        .child(message),
                )
            });

        if let Some(message) = &self.pending_commit {
            form = form
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "Commit {staged_count} staged files as “{message}”?"
                        )),
                )
                .child(
                    h_flex()
                        .justify_end()
                        .gap(px(6.))
                        .child(
                            Button::new("cancel-commit")
                                .ghost()
                                .small()
                                .label("Cancel")
                                .on_click(move |_, _, cx| {
                                    cancel_view.update(cx, |workbench, cx| {
                                        workbench.pending_commit = None;
                                        cx.notify();
                                    });
                                }),
                        )
                        .child(
                            Button::new("confirm-commit")
                                .primary()
                                .small()
                                .label("Commit")
                                .disabled(self.repository_operation.is_some())
                                .on_click(move |_, window, cx| {
                                    commit_view.update(cx, |workbench, cx| {
                                        workbench.commit_repository(window, cx);
                                    });
                                }),
                        ),
                );
        } else {
            form = form.child(
                Button::new("commit-staged")
                    .primary()
                    .small()
                    .label(format!("Review commit · {staged_count} files"))
                    .disabled(self.repository_operation.is_some())
                    .on_click(move |_, _, cx| {
                        review_view.update(cx, |workbench, cx| {
                            workbench.prepare_commit(cx);
                        });
                    }),
            );
        }

        form.into_any_element()
    }

    fn render_change_row(
        &self,
        change: &magenta_core::RepositoryChange,
        area: RepositoryDiffArea,
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let area_kind = if area == RepositoryDiffArea::Staged {
            change.staged
        } else {
            change.unstaged
        };
        let kind = match area_kind {
            Some(magenta_core::RepositoryChangeKind::Added) => "A",
            Some(magenta_core::RepositoryChangeKind::Modified) => "M",
            Some(magenta_core::RepositoryChangeKind::Deleted) => "D",
            Some(magenta_core::RepositoryChangeKind::Renamed) => "R",
            Some(magenta_core::RepositoryChangeKind::Untracked) => "?",
            Some(magenta_core::RepositoryChangeKind::Conflicted) => "!",
            None => "",
        };
        let path = change.path.clone();
        let open_path = path.clone();
        let mutate_path = path.clone();
        let open = view.clone();
        let mutate = view;
        let origin = if self.run_changes.contains_key(&path) && self.run_baseline.contains(&path) {
            "This run · pre-existing"
        } else if self.run_changes.contains_key(&path) {
            "This run"
        } else if self.run_baseline.contains(&path) {
            "Pre-existing"
        } else {
            "Unknown origin"
        };
        let action_label = if area == RepositoryDiffArea::Staged {
            "−"
        } else {
            "+"
        };

        h_flex()
            .w_full()
            .gap(px(5.))
            .p(px(5.))
            .rounded(px(5.))
            .hover(|style| style.bg(cx.theme().sidebar_accent))
            .child(
                div()
                    .w(px(12.))
                    .text_size(px(10.))
                    .font_bold()
                    .text_color(cx.theme().primary)
                    .child(kind),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .text_size(px(11.))
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(path),
                    )
                    .child(
                        div()
                            .text_size(px(9.))
                            .text_color(cx.theme().muted_foreground)
                            .child(origin),
                    ),
            )
            .child(
                Button::new(format!("view-change-{area:?}-{open_path}"))
                    .ghost()
                    .xsmall()
                    .label("View")
                    .on_click(move |_, window, cx| {
                        open.update(cx, |workbench, cx| {
                            workbench.open_repository_diff(open_path.clone(), area, window, cx);
                        });
                    }),
            )
            .child(
                Button::new(format!("mutate-change-{area:?}-{mutate_path}"))
                    .ghost()
                    .xsmall()
                    .label(action_label)
                    .disabled(self.repository_operation.is_some())
                    .on_click(move |_, _, cx| {
                        mutate.update(cx, |workbench, cx| {
                            workbench.mutate_repository_paths(
                                vec![mutate_path.clone()],
                                area == RepositoryDiffArea::Unstaged,
                                cx,
                            );
                        });
                    }),
            )
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
