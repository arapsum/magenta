//! Read-only, tabbed project tree and code preview for agent workspace changes.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    breadcrumb::{Breadcrumb, BreadcrumbItem},
    button::{Button, ButtonVariants as _, Toggle, ToggleGroup},
    h_flex,
    input::{Editor, EditorState, Position, TabSize},
    list::ListItem,
    resizable::{h_resizable, resizable_panel},
    tab::{Tab, TabBar},
    tree::{TreeEvent, TreeItem, TreeState, tree},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Task, Window,
    div, prelude::FluentBuilder as _, px, relative,
};
use magenta_application::ProjectCatalog;
use magenta_core::{
    AgentWorkspaceChange, Project, WorkspaceChangeKind, WorkspaceChangeState, WorkspaceDocument,
    WorkspaceEntry, WorkspaceEntryKind,
};

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui_kit::Action)]
#[action(namespace = magenta)]
struct SelectNextWorkbenchTab;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui_kit::Action)]
#[action(namespace = magenta)]
struct SelectPreviousWorkbenchTab;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui_kit::Action)]
#[action(namespace = magenta)]
struct CloseActiveWorkbenchTab;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui_kit::Action)]
#[action(namespace = magenta)]
struct ToggleWorkbenchViewMode;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui_kit::Action)]
#[action(namespace = magenta)]
struct NextWorkbenchHunk;

#[derive(Clone, Debug, Default, Eq, PartialEq, gpui_kit::Action)]
#[action(namespace = magenta)]
struct PreviousWorkbenchHunk;

#[cfg(target_os = "macos")]
const CLOSE_TAB_KEY: &str = "cmd-w";
#[cfg(not(target_os = "macos"))]
const CLOSE_TAB_KEY: &str = "ctrl-w";
#[cfg(target_os = "macos")]
const TOGGLE_VIEW_MODE_KEY: &str = "cmd-shift-d";
#[cfg(not(target_os = "macos"))]
const TOGGLE_VIEW_MODE_KEY: &str = "ctrl-shift-d";

#[derive(Clone, Debug)]
pub enum AgentWorkbenchEvent {
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TabLoadState {
    Loading,
    Ready,
    Failed(String),
}

impl TabLoadState {
    const fn label(&self) -> Option<&str> {
        match self {
            Self::Loading => Some("Loading…"),
            Self::Ready => None,
            Self::Failed(_) => Some("Load failed"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum WorkbenchViewMode {
    #[default]
    File,
    Diff,
}

struct WorkbenchDiff {
    editor: Option<Entity<EditorState>>,
    raw: String,
    call_id: String,
    kind: WorkspaceChangeKind,
    state: WorkspaceChangeState,
    error: Option<String>,
    hunk_lines: Vec<usize>,
    selected_hunk: usize,
}

impl WorkbenchDiff {
    fn has_content(&self) -> bool {
        self.editor.is_some() && !self.raw.trim().is_empty()
    }

    const fn kind_label(&self) -> &'static str {
        match self.kind {
            WorkspaceChangeKind::Create => "Created",
            WorkspaceChangeKind::Modify => "Modified",
        }
    }

    fn state_label(&self) -> &str {
        match self.state {
            WorkspaceChangeState::Proposed => "Awaiting approval",
            WorkspaceChangeState::Committed => "Committed",
            WorkspaceChangeState::Rejected => "Rejected",
            WorkspaceChangeState::Failed => self.error.as_deref().unwrap_or("Write failed"),
        }
    }

    fn status_label(&self) -> String {
        format!("{} · {}", self.kind_label(), self.state_label())
    }

    const fn status_icon(&self) -> IconName {
        match self.state {
            WorkspaceChangeState::Proposed => IconName::LoaderCircle,
            WorkspaceChangeState::Committed => IconName::CircleCheck,
            WorkspaceChangeState::Rejected | WorkspaceChangeState::Failed => IconName::CircleX,
        }
    }

    fn status_color(&self, cx: &App) -> Hsla {
        match self.state {
            WorkspaceChangeState::Proposed => cx.theme().warning,
            WorkspaceChangeState::Committed => cx.theme().success,
            WorkspaceChangeState::Rejected | WorkspaceChangeState::Failed => cx.theme().danger,
        }
    }
}

struct WorkbenchTab {
    path: String,
    editor: Entity<EditorState>,
    diff: Option<WorkbenchDiff>,
    mode: WorkbenchViewMode,
    state: TabLoadState,
    load_generation: u64,
}

pub struct AgentWorkbench {
    catalog: ProjectCatalog,
    project: Option<Project>,
    tree_state: Entity<TreeState>,
    directories: HashMap<String, Vec<WorkspaceEntry>>,
    expanded: HashSet<String>,
    directory_tasks: HashMap<String, Task<()>>,
    tabs: Vec<WorkbenchTab>,
    active_path: Option<String>,
    tree_error: Option<(String, String)>,
    explorer_open: bool,
    session_generation: u64,
    next_load_generation: u64,
    refresh_pending: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<AgentWorkbenchEvent> for AgentWorkbench {}

impl AgentWorkbench {
    pub fn new(catalog: ProjectCatalog, window: &Window, cx: &mut Context<'_, Self>) -> Self {
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

    fn position_active_diff(&self, path: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        let selected_hunk = self
            .tab_index(path)
            .and_then(|index| self.tabs[index].diff.as_ref())
            .and_then(|diff| {
                diff.editor
                    .clone()
                    .zip(diff.hunk_lines.get(diff.selected_hunk).copied())
            });
        if let Some((editor, line)) = selected_hunk {
            set_diff_cursor(&editor, line, window, cx);
        }
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
    }

    fn reset_tree(&self, cx: &mut Context<'_, Self>) {
        self.tree_state
            .update(cx, |state, cx| state.set_items(Vec::<TreeItem>::new(), cx));
    }

    fn new_editor(
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
                    Err(error) => {
                        workbench.tree_error = Some((task_path, error.to_string()));
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

    fn open_entry(&mut self, path: String, window: &mut Window, cx: &mut Context<'_, Self>) {
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
                        workbench.tabs[index].state = TabLoadState::Failed(error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn retry_active_tab(&mut self, window: &Window, cx: &Context<'_, Self>) {
        if let Some(path) = self.active_path.clone() {
            self.reload_tab(path, window, cx);
        }
    }

    fn retry_tree(&mut self, cx: &mut Context<'_, Self>) {
        let path = self
            .tree_error
            .take()
            .map_or_else(String::new, |(path, _)| path);
        self.load_directory(path, cx);
        cx.notify();
    }

    fn tab_index(&self, path: &str) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.path == path)
    }

    fn activate_tab(&mut self, path: String, cx: &mut Context<'_, Self>) {
        if self.tab_index(&path).is_none() {
            return;
        }
        self.active_path = Some(path);
        self.sync_tree_selection(cx);
        cx.notify();
    }

    fn sync_tree_selection(&self, cx: &mut Context<'_, Self>) {
        let selected: Option<SharedString> = self.active_path.clone().map(Into::into);
        self.tree_state.update(cx, |state, cx| {
            let index = selected.as_ref().and_then(|path| state.index_of(path));
            state.set_selected_index(index, cx);
        });
    }

    fn close_tab(&mut self, path: &str, cx: &mut Context<'_, Self>) {
        let Some(index) = self.tab_index(path) else {
            return;
        };
        let was_active = self.active_path.as_deref() == Some(path);
        self.tabs.remove(index);
        if was_active {
            self.active_path = self
                .tabs
                .get(index)
                .or_else(|| index.checked_sub(1).and_then(|index| self.tabs.get(index)))
                .map(|tab| tab.path.clone());
        }
        self.sync_tree_selection(cx);
        cx.notify();
    }

    fn close_active_tab(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(path) = self.active_path.clone() {
            self.close_tab(&path, cx);
        }
    }

    fn cycle_tab(&mut self, offset: isize, cx: &mut Context<'_, Self>) {
        if self.tabs.len() < 2 {
            return;
        }
        let current = self
            .active_path
            .as_deref()
            .and_then(|path| self.tab_index(path))
            .unwrap_or_default();
        let len = isize::try_from(self.tabs.len()).expect("tab count fits in isize");
        let index = (isize::try_from(current).expect("tab index fits in isize") + offset)
            .rem_euclid(len) as usize;
        self.activate_tab(self.tabs[index].path.clone(), cx);
    }

    fn toggle_explorer(&mut self, cx: &mut Context<'_, Self>) {
        self.explorer_open = !self.explorer_open;
        cx.notify();
    }

    fn set_view_mode(
        &mut self,
        mode: WorkbenchViewMode,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(path) = self.active_path.as_deref() else {
            return;
        };
        let Some(index) = self.tab_index(path) else {
            return;
        };
        let tab = &mut self.tabs[index];
        if mode == WorkbenchViewMode::Diff
            && !tab.diff.as_ref().is_some_and(WorkbenchDiff::has_content)
        {
            return;
        }
        if tab.mode != mode {
            tab.mode = mode;
            let editor = if mode == WorkbenchViewMode::Diff {
                tab.diff
                    .as_ref()
                    .and_then(|diff| diff.editor.clone())
                    .unwrap_or_else(|| tab.editor.clone())
            } else {
                tab.editor.clone()
            };
            editor.update(cx, |editor, cx| editor.focus(window, cx));
            cx.notify();
        }
    }

    fn toggle_view_mode(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(path) = self.active_path.as_deref() else {
            return;
        };
        let Some(index) = self.tab_index(path) else {
            return;
        };
        let mode = match self.tabs[index].mode {
            WorkbenchViewMode::File => WorkbenchViewMode::Diff,
            WorkbenchViewMode::Diff => WorkbenchViewMode::File,
        };
        self.set_view_mode(mode, window, cx);
    }

    fn navigate_hunk(&mut self, direction: isize, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(path) = self.active_path.as_deref() else {
            return;
        };
        let Some(index) = self.tab_index(path) else {
            return;
        };
        let tab = &mut self.tabs[index];
        let Some(diff) = tab.diff.as_mut() else {
            return;
        };
        if tab.mode != WorkbenchViewMode::Diff || diff.hunk_lines.is_empty() {
            return;
        }
        let next = if direction.is_positive() {
            diff.selected_hunk
                .checked_add(1)
                .filter(|next| *next < diff.hunk_lines.len())
        } else {
            diff.selected_hunk.checked_sub(1)
        };
        let Some(next) = next else {
            return;
        };
        let line = diff.hunk_lines[next];
        diff.selected_hunk = next;
        let Some(editor) = diff.editor.clone() else {
            return;
        };
        editor.update(cx, |editor, cx| {
            editor.set_cursor_position(
                Position {
                    line: u32::try_from(line).unwrap_or(u32::MAX),
                    character: 0,
                },
                window,
                cx,
            );
        });
        cx.notify();
    }

    fn render_tree(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        if let Some((_, error)) = self.tree_error.clone() {
            let retry_view = view;
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .px(px(14.))
                .text_center()
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

    fn render_tab_bar(&self, view: &Entity<Self>, cx: &App) -> AnyElement {
        let active_index = self
            .active_path
            .as_deref()
            .and_then(|path| self.tab_index(path));
        if self.tabs.is_empty() {
            return div()
                .h(px(32.))
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

    fn render_context_bar(&self, view: Entity<Self>, window: &Window, cx: &App) -> AnyElement {
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
            .h(px(28.))
            .flex_none()
            .min_w_0()
            .gap(px(5.))
            .px(px(10.))
            .border_b_1()
            .border_color(cx.theme().border.opacity(0.72))
            .bg(cx.theme().tokens.background.background)
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
                .gap(px(8.))
                .child(
                    Icon::empty()
                        .path("icons/code.svg")
                        .size(px(22.))
                        .text_color(cx.theme().muted_foreground.opacity(0.72)),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .font_medium()
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
                let retry_view = view;
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .px(px(24.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_medium()
                            .child("File could not be opened"),
                    )
                    .child(
                        div()
                            .max_w(px(480.))
                            .text_center()
                            .text_size(px(12.))
                            .text_color(cx.theme().muted_foreground)
                            .child(error.clone()),
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

    fn render_loading_lines(cx: &App, count: usize) -> AnyElement {
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
            .h(px(36.))
            .items_center()
            .gap(px(7.))
            .px(px(8.))
            .border_b_1()
            .border_color(cx.theme().border)
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
            .child(Icon::empty().path("icons/code.svg").xsmall())
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
    ) -> AnyElement {
        if narrow && show_explorer {
            v_flex()
                .size_full()
                .p(px(4.))
                .child(tree)
                .into_any_element()
        } else if show_explorer {
            h_resizable("agent-workbench")
                .child(
                    resizable_panel()
                        .size(px(220.))
                        .size_range(px(160.)..px(360.))
                        .child(v_flex().size_full().p(px(4.)).child(tree)),
                )
                .child(resizable_panel().child(editor))
                .into_any_element()
        } else {
            editor
        }
    }

    #[cfg(test)]
    pub(crate) const fn tab_count(&self) -> usize {
        self.tabs.len()
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
        let tree = self.render_tree(view.clone(), cx);
        let editor = self.render_editor_pane(view, window, cx);
        let header = Self::render_header(project_name, show_explorer, cx);
        let content = Self::render_content_layout(tree, editor, narrow, show_explorer);
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
            .bg(cx.theme().tokens.background.background)
            .child(header)
            .child(content)
    }
}

fn apply_document(
    editor: &Entity<EditorState>,
    document: &WorkspaceDocument,
    window: &mut Window,
    cx: &mut Context<'_, AgentWorkbench>,
) {
    editor.update(cx, |editor, cx| {
        editor.set_highlighter(document.language.clone(), cx);
        editor.set_value(document.content.clone(), window, cx);
    });
}

fn apply_diff(
    editor: &Entity<EditorState>,
    diff: &str,
    window: &mut Window,
    cx: &mut Context<'_, AgentWorkbench>,
) {
    editor.update(cx, |editor, cx| {
        editor.set_highlighter("diff", cx);
        editor.set_value(diff.to_owned(), window, cx);
    });
}

fn set_diff_cursor(
    editor: &Entity<EditorState>,
    line: usize,
    window: &mut Window,
    cx: &mut Context<'_, AgentWorkbench>,
) {
    editor.update(cx, |editor, cx| {
        editor.set_cursor_position(
            Position {
                line: u32::try_from(line).unwrap_or(u32::MAX),
                character: 0,
            },
            window,
            cx,
        );
    });
}

fn parse_hunk_lines(diff: &str) -> Vec<usize> {
    diff.lines()
        .enumerate()
        .filter_map(|(line, content)| content.starts_with("@@").then_some(line))
        .collect()
}

fn shortest_unique_suffix(path: &str, paths: &[&str]) -> String {
    let components = path.split('/').collect::<Vec<_>>();
    for length in 1..=components.len() {
        let candidate = components[components.len() - length..].join("/");
        let unique = paths.iter().filter(|other| **other != path).all(|other| {
            let other_components = other.split('/').collect::<Vec<_>>();
            other_components.len() < length
                || other_components[other_components.len() - length..].join("/") != candidate
        });
        if unique {
            return candidate;
        }
    }
    path.to_owned()
}

fn language_for_path(path: &str) -> String {
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    match (name, extension) {
        ("CMakeLists.txt", _) | (_, "cmake") => "cmake",
        (_, "rs") => "rust",
        (_, "c" | "h") => "c",
        (_, "cc" | "cpp" | "cxx" | "hpp") => "cpp",
        (_, "sh" | "bash") => "bash",
        (_, "json") => "json",
        (_, "toml") => "toml",
        (_, "yaml" | "yml") => "yaml",
        (_, "md" | "markdown") => "markdown",
        (_, "js" | "mjs" | "cjs") => "javascript",
        (_, "ts") => "typescript",
        (_, "tsx") => "tsx",
        (_, "py") => "python",
        _ => "text",
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use gpui_kit::{TestAppContext, px, size};
    use magenta_application::ProjectCatalog;
    use magenta_core::{
        AgentWorkspaceChange, Project, ProjectStore, StorageFuture, Timestamp, WorkspaceBrowser,
        WorkspaceDocument, WorkspaceEntry, WorkspaceFuture,
    };

    use super::{
        AgentWorkbench, NextWorkbenchHunk, PreviousWorkbenchHunk, TOGGLE_VIEW_MODE_KEY,
        TabLoadState, WorkbenchViewMode, parse_hunk_lines, shortest_unique_suffix,
    };

    #[derive(Default)]
    struct TestProjects {
        documents: Mutex<Vec<WorkspaceFuture<WorkspaceDocument>>>,
    }

    impl ProjectStore for TestProjects {
        fn projects(&self) -> StorageFuture<Vec<Project>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn upsert_project(&self, _: Project) -> StorageFuture<()> {
            Box::pin(async { Ok(()) })
        }

        fn remove_project(&self, _: PathBuf) -> StorageFuture<()> {
            Box::pin(async { Ok(()) })
        }
    }

    impl WorkspaceBrowser for TestProjects {
        fn canonicalize_root(&self, root: PathBuf) -> WorkspaceFuture<PathBuf> {
            Box::pin(async move { Ok(root) })
        }

        fn list_directory(&self, _: PathBuf, _: String) -> WorkspaceFuture<Vec<WorkspaceEntry>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn read_document(&self, _: PathBuf, _: String) -> WorkspaceFuture<WorkspaceDocument> {
            self.documents.lock().unwrap().remove(0)
        }
    }

    fn project() -> Project {
        Project {
            name: "Magenta".to_owned(),
            root: PathBuf::from("/workspace/magenta"),
            added_at: Timestamp(1),
            last_opened_at: Timestamp(1),
        }
    }

    fn change(path: &str, content: &str) -> AgentWorkspaceChange {
        change_with(
            path,
            content,
            "",
            "call-1",
            magenta_core::WorkspaceChangeState::Committed,
        )
    }

    fn change_with(
        path: &str,
        content: &str,
        diff: &str,
        call_id: &str,
        state: magenta_core::WorkspaceChangeState,
    ) -> AgentWorkspaceChange {
        AgentWorkspaceChange {
            call_id: call_id.to_owned(),
            path: path.to_owned(),
            kind: magenta_core::WorkspaceChangeKind::Modify,
            content: content.to_owned(),
            diff: diff.to_owned(),
            state,
            error: None,
        }
    }

    fn setup(
        cx: &mut TestAppContext,
        projects: Arc<TestProjects>,
    ) -> gpui_kit::WindowHandle<AgentWorkbench> {
        setup_with_size(cx, projects, size(px(900.), px(640.)))
    }

    fn setup_with_size(
        cx: &mut TestAppContext,
        projects: Arc<TestProjects>,
        window_size: gpui_kit::Size<gpui_kit::Pixels>,
    ) -> gpui_kit::WindowHandle<AgentWorkbench> {
        cx.update(gpui_kit::init);
        cx.open_window(window_size, move |window, cx| {
            AgentWorkbench::new(ProjectCatalog::new(projects.clone(), projects), window, cx)
        })
    }

    #[test]
    fn tab_labels_use_the_shortest_distinguishing_suffix() {
        let paths = ["src/index.ts", "tests/index.ts", "README.md"];
        assert_eq!(
            shortest_unique_suffix("src/index.ts", &paths),
            "src/index.ts"
        );
        assert_eq!(
            shortest_unique_suffix("tests/index.ts", &paths),
            "tests/index.ts"
        );
        assert_eq!(shortest_unique_suffix("README.md", &paths), "README.md");
    }

    #[test]
    fn hunk_detection_handles_crlf_empty_and_malformed_diffs() {
        assert_eq!(
            parse_hunk_lines(
                "diff --git a/a.rs b/a.rs\r\n@@ -1 +1 @@\r\n-old\r\n+new\r\n@@ -4 +4 @@\r\n"
            ),
            vec![1, 4]
        );
        assert!(parse_hunk_lines("").is_empty());
        assert!(parse_hunk_lines("not a unified diff\n+still content").is_empty());
        assert_eq!(parse_hunk_lines("@@ malformed\ncontent"), vec![0]);
    }

    #[gpui_kit::test]
    fn workspace_changes_upsert_tabs_and_closing_selects_the_neighbor(cx: &mut TestAppContext) {
        let window = setup(cx, Arc::new(TestProjects::default()));
        window
            .update(cx, |workbench, window, cx| {
                workbench.show_change(change("src/a.rs", "first"), window, cx);
                workbench.show_change(change("src/b.rs", "second"), window, cx);
                workbench.show_change(change("src/a.rs", "updated"), window, cx);
                assert_eq!(workbench.tabs.len(), 2);
                assert_eq!(workbench.active_path.as_deref(), Some("src/a.rs"));
                assert_eq!(
                    workbench.tabs[0].editor.read(cx).value().as_ref(),
                    "updated"
                );

                workbench.close_active_tab(cx);
                assert_eq!(workbench.active_path.as_deref(), Some("src/b.rs"));
                workbench.close_active_tab(cx);
                assert!(workbench.tabs.is_empty());
                assert!(workbench.active_path.is_none());
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn diff_state_defaults_navigates_and_preserves_same_call_mode(cx: &mut TestAppContext) {
        let window = setup(cx, Arc::new(TestProjects::default()));
        let diff = "header\n@@ -1 +1 @@\n-old\n+new\n@@ -4 +4 @@\n-before\n+after\n";
        window
            .update(cx, |workbench, window, cx| {
                workbench.show_change(
                    change_with(
                        "src/a.rs",
                        "new",
                        diff,
                        "call-1",
                        magenta_core::WorkspaceChangeState::Proposed,
                    ),
                    window,
                    cx,
                );
                let tab = &workbench.tabs[0];
                assert_eq!(tab.mode, WorkbenchViewMode::Diff);
                let change = tab.diff.as_ref().unwrap();
                assert_eq!(change.raw, diff);
                assert_eq!(change.hunk_lines, vec![1, 4]);
                assert_eq!(change.selected_hunk, 0);
                assert_eq!(
                    change
                        .editor
                        .as_ref()
                        .unwrap()
                        .read(cx)
                        .cursor_position()
                        .line,
                    1
                );

                workbench.navigate_hunk(1, window, cx);
                assert_eq!(workbench.tabs[0].diff.as_ref().unwrap().selected_hunk, 1);
                assert_eq!(
                    workbench.tabs[0]
                        .diff
                        .as_ref()
                        .unwrap()
                        .editor
                        .as_ref()
                        .unwrap()
                        .read(cx)
                        .cursor_position()
                        .line,
                    4
                );
                workbench.navigate_hunk(1, window, cx);
                assert_eq!(workbench.tabs[0].diff.as_ref().unwrap().selected_hunk, 1);
                workbench.navigate_hunk(-1, window, cx);
                workbench.navigate_hunk(-1, window, cx);
                assert_eq!(workbench.tabs[0].diff.as_ref().unwrap().selected_hunk, 0);

                workbench.set_view_mode(WorkbenchViewMode::File, window, cx);
                workbench.show_change(
                    change_with(
                        "src/a.rs",
                        "updated",
                        "header\n@@ -1 +1 @@\n-old\n+updated\n",
                        "call-1",
                        magenta_core::WorkspaceChangeState::Committed,
                    ),
                    window,
                    cx,
                );
                assert_eq!(workbench.tabs[0].mode, WorkbenchViewMode::File);
                assert_eq!(
                    workbench.tabs[0].editor.read(cx).value().as_ref(),
                    "updated"
                );
                assert_eq!(workbench.tabs[0].diff.as_ref().unwrap().selected_hunk, 0);
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn keyboard_actions_toggle_modes_and_navigate_hunks(cx: &mut TestAppContext) {
        let window = setup(cx, Arc::new(TestProjects::default()));
        window
            .update(cx, |workbench, window, cx| {
                workbench.show_change(
                    change_with(
                        "src/a.rs",
                        "new",
                        "header\n@@ -1 +1 @@\n-old\n+new\n@@ -4 +4 @@\n-before\n+after\n",
                        "call-1",
                        magenta_core::WorkspaceChangeState::Committed,
                    ),
                    window,
                    cx,
                );
            })
            .unwrap();

        let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let mode_bounds = visual
            .debug_bounds("workbench-view-mode")
            .expect("the diff tab should render its mode selector");
        visual.simulate_click(
            gpui_kit::point(mode_bounds.origin.x + px(4.), mode_bounds.center().y),
            gpui_kit::Modifiers::default(),
        );
        window
            .update(cx, |workbench, _, _cx| {
                assert_eq!(workbench.tabs[0].mode, WorkbenchViewMode::File);
            })
            .unwrap();
        let mode_bounds = visual
            .debug_bounds("workbench-view-mode")
            .expect("the mode selector should remain visible in file mode");
        visual.simulate_click(
            gpui_kit::point(
                mode_bounds.origin.x + mode_bounds.size.width - px(4.),
                mode_bounds.center().y,
            ),
            gpui_kit::Modifiers::default(),
        );
        let next_bounds = visual
            .debug_bounds("workbench-next-hunk")
            .expect("the next hunk button should render");
        visual.simulate_click(next_bounds.center(), gpui_kit::Modifiers::default());
        window
            .update(cx, |workbench, _, _cx| {
                assert_eq!(workbench.tabs[0].diff.as_ref().unwrap().selected_hunk, 1);
            })
            .unwrap();
        visual.simulate_click(next_bounds.center(), gpui_kit::Modifiers::default());
        let previous_bounds = visual
            .debug_bounds("workbench-previous-hunk")
            .expect("the previous hunk button should render");
        visual.simulate_click(previous_bounds.center(), gpui_kit::Modifiers::default());
        visual.simulate_click(previous_bounds.center(), gpui_kit::Modifiers::default());
        window
            .update(cx, |workbench, _, _cx| {
                assert_eq!(workbench.tabs[0].diff.as_ref().unwrap().selected_hunk, 0);
            })
            .unwrap();
        visual.simulate_keystrokes(TOGGLE_VIEW_MODE_KEY);
        visual.simulate_keystrokes(TOGGLE_VIEW_MODE_KEY);
        visual.dispatch_action(NextWorkbenchHunk);
        visual.dispatch_action(NextWorkbenchHunk);
        visual.dispatch_action(PreviousWorkbenchHunk);
        window
            .update(cx, |workbench, _, _cx| {
                assert_eq!(workbench.tabs[0].mode, WorkbenchViewMode::Diff);
                assert_eq!(workbench.tabs[0].diff.as_ref().unwrap().selected_hunk, 0);
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn new_calls_reset_diff_mode_and_empty_diffs_are_file_only(cx: &mut TestAppContext) {
        let window = setup(cx, Arc::new(TestProjects::default()));
        window
            .update(cx, |workbench, window, cx| {
                workbench.show_change(
                    change_with(
                        "src/a.rs",
                        "one",
                        "@@ -1 +1 @@\n-one\n+one\n",
                        "call-1",
                        magenta_core::WorkspaceChangeState::Committed,
                    ),
                    window,
                    cx,
                );
                workbench.set_view_mode(WorkbenchViewMode::File, window, cx);
                workbench.show_change(
                    change_with(
                        "src/a.rs",
                        "two",
                        "@@ -1 +1 @@\n-one\n+two\n",
                        "call-2",
                        magenta_core::WorkspaceChangeState::Committed,
                    ),
                    window,
                    cx,
                );
                assert_eq!(workbench.tabs[0].mode, WorkbenchViewMode::Diff);
                workbench.show_change(
                    change_with(
                        "src/b.rs",
                        "other",
                        "@@ -1 +1 @@\n-old\n+other\n",
                        "call-b",
                        magenta_core::WorkspaceChangeState::Committed,
                    ),
                    window,
                    cx,
                );
                workbench.activate_tab("src/a.rs".to_owned(), cx);
                assert_eq!(workbench.tabs[0].mode, WorkbenchViewMode::Diff);
                workbench.set_view_mode(WorkbenchViewMode::File, window, cx);
                workbench.activate_tab("src/b.rs".to_owned(), cx);
                assert_eq!(workbench.tabs[1].mode, WorkbenchViewMode::Diff);
                workbench.activate_tab("src/a.rs".to_owned(), cx);
                workbench.show_change(change("src/a.rs", "three"), window, cx);
                assert_eq!(workbench.tabs[0].mode, WorkbenchViewMode::File);
                assert!(!workbench.tabs[0].diff.as_ref().unwrap().has_content());
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn refresh_reloads_file_content_without_discarding_diff_state(cx: &mut TestAppContext) {
        let projects = Arc::new(TestProjects::default());
        projects.documents.lock().unwrap().push(Box::pin(async {
            Ok(WorkspaceDocument {
                path: "src/a.rs".to_owned(),
                content: "from disk".to_owned(),
                language: "rust".to_owned(),
            })
        }));
        projects.documents.lock().unwrap().push(Box::pin(async {
            Ok(WorkspaceDocument {
                path: "src/a.rs".to_owned(),
                content: "refreshed from disk".to_owned(),
                language: "rust".to_owned(),
            })
        }));
        let window = setup(cx, projects);
        window
            .update(cx, |workbench, window, cx| {
                workbench.set_project(Some(project()), window, cx);
                workbench.open_entry("src/a.rs".to_owned(), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |workbench, window, cx| {
                workbench.show_change(
                    change_with(
                        "src/a.rs",
                        "agent content",
                        "@@ -1 +1 @@\n-from disk\n+agent content\n",
                        "call-1",
                        magenta_core::WorkspaceChangeState::Committed,
                    ),
                    window,
                    cx,
                );
                workbench.refresh(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |workbench, _, cx| {
                assert_eq!(
                    workbench.tabs[0].editor.read(cx).value().as_ref(),
                    "refreshed from disk"
                );
                assert_eq!(workbench.tabs[0].mode, WorkbenchViewMode::Diff);
                assert_eq!(
                    workbench.tabs[0].diff.as_ref().unwrap().raw,
                    "@@ -1 +1 @@\n-from disk\n+agent content\n"
                );
                workbench.clear_session(cx);
                assert!(workbench.tabs.is_empty());
                assert!(workbench.active_path.is_none());
                assert!(workbench.project.is_none());
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn narrow_diff_toolbar_keeps_review_controls_in_the_workbench(cx: &mut TestAppContext) {
        let window = setup_with_size(
            cx,
            Arc::new(TestProjects::default()),
            size(px(520.), px(640.)),
        );
        window
            .update(cx, |workbench, window, cx| {
                workbench.show_change(
                    change_with(
                        "src/a.rs",
                        "new",
                        "@@ -1 +1 @@\n-old\n+new\n@@ -4 +4 @@\n-before\n+after\n",
                        "call-1",
                        magenta_core::WorkspaceChangeState::Committed,
                    ),
                    window,
                    cx,
                );
            })
            .unwrap();
        let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let workbench_bounds = visual
            .debug_bounds("agent-workbench")
            .expect("the narrow workbench should render");
        for selector in [
            "workbench-view-mode",
            "workbench-previous-hunk",
            "workbench-next-hunk",
        ] {
            let bounds = visual
                .debug_bounds(selector)
                .expect("diff toolbar control should render on a narrow window");
            assert!(
                bounds.origin.x + bounds.size.width
                    <= workbench_bounds.origin.x + workbench_bounds.size.width
            );
        }
    }

    #[gpui_kit::test]
    fn stale_load_cannot_overwrite_a_reopened_tab(cx: &mut TestAppContext) {
        let projects = Arc::new(TestProjects::default());
        let (old_sender, old_receiver) = futures_channel::oneshot::channel();
        projects
            .documents
            .lock()
            .unwrap()
            .push(Box::pin(async move { old_receiver.await.unwrap() }));
        projects.documents.lock().unwrap().push(Box::pin(async {
            Ok(WorkspaceDocument {
                path: "src/a.rs".to_owned(),
                content: "new".to_owned(),
                language: "rust".to_owned(),
            })
        }));
        let window = setup(cx, projects);
        window
            .update(cx, |workbench, window, cx| {
                workbench.set_project(Some(project()), window, cx);
                workbench.open_entry("src/a.rs".to_owned(), window, cx);
                workbench.close_active_tab(cx);
                workbench.open_entry("src/a.rs".to_owned(), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |workbench, _, cx| {
                assert_eq!(workbench.tabs[0].editor.read(cx).value().as_ref(), "new");
                assert_eq!(workbench.tabs[0].state, TabLoadState::Ready);
            })
            .unwrap();
        old_sender
            .send(Ok(WorkspaceDocument {
                path: "src/a.rs".to_owned(),
                content: "old".to_owned(),
                language: "rust".to_owned(),
            }))
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |workbench, _, cx| {
                assert_eq!(workbench.tabs[0].editor.read(cx).value().as_ref(), "new");
            })
            .unwrap();
    }
}
