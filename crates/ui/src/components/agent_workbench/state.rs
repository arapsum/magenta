use super::*;

impl AgentWorkbench {
    pub fn new(
        catalog: ProjectCatalog,
        repository: Arc<dyn RepositoryAccess>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.bind_keys([
            gpui_kit::KeyBinding::new("ctrl-tab", SelectNextWorkbenchTab, Some("AgentWorkbench")),
            gpui_kit::KeyBinding::new(
                "ctrl-shift-tab",
                SelectPreviousWorkbenchTab,
                Some("AgentWorkbench"),
            ),
            gpui_kit::KeyBinding::new(
                CLOSE_TAB_KEY,
                CloseActiveWorkbenchTab,
                Some("AgentWorkbench"),
            ),
            gpui_kit::KeyBinding::new(
                TOGGLE_VIEW_MODE_KEY,
                ToggleWorkbenchViewMode,
                Some("AgentWorkbench"),
            ),
            gpui_kit::KeyBinding::new("f7", NextWorkbenchHunk, Some("AgentWorkbench")),
            gpui_kit::KeyBinding::new("shift-f7", PreviousWorkbenchHunk, Some("AgentWorkbench")),
        ]);

        let tree_state = cx.new(|cx| TreeState::new(cx));
        let commit_input = cx.new(|cx| InputState::new(window, cx).placeholder("Commit message"));

        let tree_subscription = cx.subscribe(&tree_state, move |view, _, event: &TreeEvent, cx| {
            if let TreeEvent::Expanded(path) = event {
                view.expanded.insert(path.to_string());
                view.load_directory(path.to_string(), cx);
            } else if let TreeEvent::Collapsed(path) = event {
                view.expanded.remove(path.as_ref());
            }
        });
        Self {
            catalog,
            repository,
            project: None,
            tree_state,
            directories: HashMap::new(),
            expanded: HashSet::new(),
            directory_tasks: HashMap::new(),
            tabs: Vec::new(),
            active_path: None,
            tree_error: None,
            explorer_open: window.viewport_size().width >= px(696.),
            session_generation: 0,
            next_load_generation: 0,
            refresh_pending: false,
            section: WorkbenchSection::Files,
            repository_status: None,
            repository_error: None,
            repository_loading: false,
            repository_generation: 0,
            repository_task: None,
            repository_poll_task: None,
            repository_operation: None,
            commit_input,
            pending_commit: None,
            commit_error: None,
            last_commit: None,
            run_baseline: HashSet::new(),
            run_changes: HashMap::new(),
            _subscriptions: vec![tree_subscription],
        }
    }

    pub fn set_project(
        &mut self,
        project: Option<Project>,
        _window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let unchanged = self.project.as_ref().map(|project| &project.root)
            == project.as_ref().map(|project| &project.root);

        if unchanged {
            return;
        }

        self.reset_state();
        self.project = project;
        self.reset_tree(cx);

        if self.project.is_some() {
            self.load_directory(String::new(), cx);
            self.refresh_repository(cx);
        }
        cx.notify();
    }

    /// Ends the current code-view session, including its project context.
    pub fn clear_session(&mut self, cx: &mut Context<'_, Self>) {
        self.reset_state();
        self.project = None;
        self.reset_tree(cx);
        cx.notify();
    }

    pub fn show_change(
        &mut self,
        change: AgentWorkspaceChange,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if matches!(
            change.state,
            WorkspaceChangeState::Staged | WorkspaceChangeState::Committed
        ) {
            self.run_changes.insert(change.path.clone(), change.kind);
        }

        let (path, has_diff) = self.upsert_change(change, window, cx);

        if has_diff {
            self.position_active_diff(&path, window, cx);
        }
        self.activate_tab(path, cx);
    }

    fn upsert_change(
        &mut self,
        change: AgentWorkspaceChange,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> (String, bool) {
        let AgentWorkspaceChange {
            call_id,
            path,
            kind,
            content,
            diff,
            state,
            error,
        } = change;
        let has_diff = !diff.trim().is_empty();
        let hunk_lines = parse_hunk_lines(&diff);
        let hunk_count = hunk_lines.len();

        let document = WorkspaceDocument {
            path: path.clone(),
            content,
            language: language_for_path(&path),
        };
        let existing_index = self.tab_index(&path);

        let (same_call, previous_has_diff, previous_mode, previous_hunk, previous_editor) =
            existing_index.map_or((false, false, WorkbenchViewMode::File, 0, None), |index| {
                let tab = &self.tabs[index];
                let Some(existing) = tab.diff.as_ref() else {
                    return (false, false, tab.mode, 0, None);
                };
                (
                    existing.call_id == call_id,
                    existing.has_content(),
                    tab.mode,
                    existing.selected_hunk,
                    existing.editor.clone(),
                )
            });

        let diff_editor = has_diff.then(|| {
            same_call
                .then_some(previous_editor)
                .flatten()
                .unwrap_or_else(|| Self::new_editor(window, cx, "diff"))
        });

        if let Some(index) = existing_index {
            let tab = &mut self.tabs[index];
            tab.load_generation = tab.load_generation.wrapping_add(1);
            tab.state = TabLoadState::Ready;
            apply_document(&tab.editor, &document, window, cx);

            if let Some(editor) = &diff_editor {
                apply_diff(editor, &diff, window, cx);
            }

            tab.diff = Some(WorkbenchDiff {
                editor: diff_editor,
                raw: diff,
                call_id,
                kind,
                state,
                error,
                hunk_lines,
                selected_hunk: if same_call && has_diff {
                    previous_hunk.min(hunk_count.saturating_sub(1))
                } else {
                    0
                },
                repository_area: None,
            });

            tab.mode = if has_diff && same_call && previous_has_diff {
                previous_mode
            } else if has_diff {
                WorkbenchViewMode::Diff
            } else {
                WorkbenchViewMode::File
            };
        } else {
            let editor = Self::new_editor(window, cx, "text");
            apply_document(&editor, &document, window, cx);

            if let Some(diff_editor) = &diff_editor {
                apply_diff(diff_editor, &diff, window, cx);
            }

            self.tabs.push(WorkbenchTab {
                path: path.clone(),
                editor,
                diff: Some(WorkbenchDiff {
                    editor: diff_editor,
                    raw: diff,
                    call_id,
                    kind,
                    state,
                    error,
                    hunk_lines,
                    selected_hunk: 0,
                    repository_area: None,
                }),
                mode: if has_diff {
                    WorkbenchViewMode::Diff
                } else {
                    WorkbenchViewMode::File
                },
                state: TabLoadState::Ready,
                load_generation: 0,
            });
        }
        (path, has_diff)
    }

    pub fn refresh(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if self.project.is_none() {
            return;
        }
        self.refresh_pending = false;
        self.directory_tasks.clear();
        self.directories.clear();
        self.tree_error = None;
        self.reset_tree(cx);

        self.load_directory(String::new(), cx);
        self.refresh_repository(cx);

        let paths = self
            .tabs
            .iter()
            .map(|tab| tab.path.clone())
            .collect::<Vec<_>>();

        for path in paths {
            self.reload_tab(path, window, cx);
        }
        cx.notify();
    }

    pub fn mark_workspace_invalidated(
        &mut self,
        visible: bool,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        if visible {
            self.refresh(window, cx);
        } else {
            self.refresh_pending = true;
        }
    }

    pub fn prepare_to_show(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if self.refresh_pending {
            self.refresh(window, cx);
        }
    }

    fn reset_state(&mut self) {
        self.session_generation = self.session_generation.wrapping_add(1);
        self.directory_tasks.clear();
        self.directories.clear();
        self.expanded.clear();
        self.tabs.clear();
        self.active_path = None;
        self.tree_error = None;
        self.refresh_pending = false;

        self.repository_generation = self.repository_generation.wrapping_add(1);
        self.repository_task.take();
        self.repository_poll_task.take();
        self.repository_operation.take();
        self.repository_status = None;
        self.repository_error = None;
        self.repository_loading = false;

        self.pending_commit = None;
        self.commit_error = None;
        self.last_commit = None;
        self.run_baseline.clear();
        self.run_changes.clear();
    }

    fn reset_tree(&self, cx: &mut Context<'_, Self>) {
        self.tree_state
            .update(cx, |state, cx| state.set_items(Vec::<TreeItem>::new(), cx));
    }

    pub(super) fn new_editor(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        language: &str,
    ) -> Entity<EditorState> {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .line_number(true)
                .folding(true)
                .searchable(true)
                .soft_wrap(false)
                .tab_size(TabSize {
                    tab_size: 4,
                    hard_tabs: false,
                })
        });

        editor.update(cx, |editor, cx| editor.set_readonly(true, cx));
        editor
    }

    fn load_directory(&mut self, path: String, cx: &Context<'_, Self>) {
        if self.directories.contains_key(&path) || self.directory_tasks.contains_key(&path) {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };

        let catalog = self.catalog.clone();
        let session_generation = self.session_generation;
        let task_path = path.clone();

        let task = cx.spawn(async move |view, cx| {
            let result = catalog.entries(project.root, task_path.clone()).await;

            _ = view.update(cx, |workbench, cx| {
                if workbench.session_generation != session_generation {
                    return;
                }

                workbench.directory_tasks.remove(&task_path);
                match result {
                    Ok(entries) => {
                        if workbench
                            .tree_error
                            .as_ref()
                            .is_some_and(|(path, _)| path == &task_path)
                        {
                            workbench.tree_error = None;
                        }
                        let expanded_children = entries
                            .iter()
                            .filter(|entry| {
                                entry.kind == WorkspaceEntryKind::Directory
                                    && workbench.expanded.contains(&entry.path)
                            })
                            .map(|entry| entry.path.clone())
                            .collect::<Vec<_>>();
                        workbench.directories.insert(task_path, entries);
                        workbench.rebuild_tree(cx);
                        for path in expanded_children {
                            workbench.load_directory(path, cx);
                        }
                    }
                    Err(_error) => {
                        workbench.tree_error = Some((
                            task_path,
                            "Project files could not be listed. Retry the folder scan.".to_owned(),
                        ));
                    }
                }
                cx.notify();
            });
        });
        self.directory_tasks.insert(path, task);
    }

    #[allow(clippy::needless_pass_by_ref_mut)]
    fn rebuild_tree(&self, cx: &mut Context<'_, Self>) {
        let items = self
            .directories
            .get("")
            .into_iter()
            .flatten()
            .map(|entry| self.tree_item(entry))
            .collect::<Vec<_>>();
        self.tree_state
            .update(cx, |state, cx| state.set_items(items, cx));
        self.sync_tree_selection(cx);
    }

    fn tree_item(&self, entry: &WorkspaceEntry) -> TreeItem {
        let mut item = TreeItem::new(entry.path.clone(), entry.name.clone())
            .expanded(self.expanded.contains(&entry.path));
        if entry.kind == WorkspaceEntryKind::Directory {
            if let Some(children) = self.directories.get(&entry.path) {
                item = item.children(children.iter().map(|child| self.tree_item(child)));
            } else {
                item = item.child(TreeItem::new(
                    format!("{}/.loading", entry.path),
                    "Loading…",
                ));
            }
        }
        item
    }

    pub(super) fn open_entry(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.tab_index(&path).is_some() {
            self.activate_tab(path, cx);
            return;
        }

        let editor = Self::new_editor(window, cx, "text");
        self.tabs.push(WorkbenchTab {
            path: path.clone(),
            editor,
            diff: None,
            mode: WorkbenchViewMode::File,
            state: TabLoadState::Loading,
            load_generation: 0,
        });

        self.activate_tab(path.clone(), cx);
        self.reload_tab(path, window, cx);
    }

    fn reload_tab(&mut self, path: String, window: &Window, cx: &Context<'_, Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let Some(index) = self.tab_index(&path) else {
            return;
        };
        self.next_load_generation = self.next_load_generation.wrapping_add(1);
        let load_generation = self.next_load_generation;
        self.tabs[index].load_generation = load_generation;
        self.tabs[index].state = TabLoadState::Loading;

        let session_generation = self.session_generation;
        let catalog = self.catalog.clone();

        cx.spawn_in(window, async move |view, window| {
            let result = catalog.document(project.root, path.clone()).await;

            _ = view.update_in(window, |workbench, window, cx| {
                if workbench.session_generation != session_generation {
                    return;
                }
                let Some(index) = workbench.tab_index(&path) else {
                    return;
                };
                if workbench.tabs[index].load_generation != load_generation {
                    return;
                }

                match result {
                    Ok(document) => {
                        let tab = &mut workbench.tabs[index];
                        tab.state = TabLoadState::Ready;
                        apply_document(&tab.editor, &document, window, cx);
                    }
                    Err(error) => {
                        workbench.tabs[index].state =
                            TabLoadState::Failed(classify_file_error(&error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn retry_active_tab(&mut self, window: &Window, cx: &Context<'_, Self>) {
        if let Some(path) = self.active_path.clone() {
            self.reload_tab(path, window, cx);
        }
    }

    pub(super) fn retry_tree(&mut self, cx: &mut Context<'_, Self>) {
        let path = self
            .tree_error
            .take()
            .map_or_else(String::new, |(path, _)| path);
        self.load_directory(path, cx);
        cx.notify();
    }
}
