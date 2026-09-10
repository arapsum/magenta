//! Read-only, tabbed project tree and code preview for agent workspace changes.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    breadcrumb::{Breadcrumb, BreadcrumbItem},
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState, TabSize},
    list::ListItem,
    resizable::{h_resizable, resizable_panel},
    tab::{Tab, TabBar},
    tree::{TreeEvent, TreeItem, TreeState, tree},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Task, Window,
    div, prelude::FluentBuilder as _, px, relative,
};
use magenta_application::ProjectCatalog;
use magenta_core::{
    AgentWorkspaceChange, Project, WorkspaceChangeState, WorkspaceDocument, WorkspaceEntry,
    WorkspaceEntryKind,
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

#[cfg(target_os = "macos")]
const CLOSE_TAB_KEY: &str = "cmd-w";
#[cfg(not(target_os = "macos"))]
const CLOSE_TAB_KEY: &str = "ctrl-w";

#[derive(Clone, Debug)]
pub enum AgentWorkbenchEvent {
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TabLoadState {
    Loading,
    Ready,
    Change(WorkspaceChangeState, Option<String>),
    Failed(String),
}

impl TabLoadState {
    fn label(&self) -> Option<&str> {
        match self {
            Self::Loading => Some("Loading…"),
            Self::Ready => None,
            Self::Change(WorkspaceChangeState::Proposed, _) => Some("Awaiting approval"),
            Self::Change(WorkspaceChangeState::Committed, _) => Some("Committed"),
            Self::Change(WorkspaceChangeState::Rejected, _) => Some("Rejected"),
            Self::Change(WorkspaceChangeState::Failed, error) => {
                Some(error.as_deref().unwrap_or("Write failed"))
            }
            Self::Failed(_) => Some("Load failed"),
        }
    }
}

struct WorkbenchTab {
    path: String,
    editor: Entity<EditorState>,
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
        let path = change.path.clone();
        let state = TabLoadState::Change(change.state, change.error);
        let document = WorkspaceDocument {
            path: path.clone(),
            content: change.content,
            language: language_for_path(&path),
        };
        if let Some(index) = self.tab_index(&path) {
            let tab = &mut self.tabs[index];
            tab.load_generation = tab.load_generation.wrapping_add(1);
            tab.state = state;
            apply_document(&tab.editor, &document, window, cx);
        } else {
            let editor = Self::new_editor(window, cx);
            apply_document(&editor, &document, window, cx);
            self.tabs.push(WorkbenchTab {
                path: path.clone(),
                editor,
                state,
                load_generation: 0,
            });
        }
        self.activate_tab(path, cx);
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

    fn new_editor(window: &mut Window, cx: &mut Context<'_, Self>) -> Entity<EditorState> {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("text")
                .line_number(true)
                .folding(true)
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
        let editor = Self::new_editor(window, cx);
        self.tabs.push(WorkbenchTab {
            path: path.clone(),
            editor,
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
                Tab::new()
                    .label(self.tab_label(&path))
                    .aria_label(path)
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

    fn render_context_bar(&self, cx: &App) -> AnyElement {
        let Some(path) = self.active_path.as_deref() else {
            return div().h(px(28.)).flex_none().into_any_element();
        };
        let status = self
            .tab_index(path)
            .and_then(|index| self.tabs[index].state.label().map(str::to_owned));
        h_flex()
            .h(px(28.))
            .flex_none()
            .min_w_0()
            .gap(px(8.))
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
            )
            .when_some(status, |this, status| {
                this.child(
                    div()
                        .flex_none()
                        .text_size(px(10.))
                        .font_medium()
                        .text_color(cx.theme().muted_foreground)
                        .child(status),
                )
            })
            .into_any_element()
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
            TabLoadState::Ready | TabLoadState::Change(_, _) => Editor::new(&tab.editor)
                .readonly(true)
                .bordered(false)
                .h(relative(1.))
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(cx.theme().mono_font_size)
                .into_any_element(),
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

    fn render_editor_pane(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        v_flex()
            .size_full()
            .min_w_0()
            .child(self.render_tab_bar(&view, cx))
            .child(self.render_context_bar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(self.render_active_content(view, cx)),
            )
            .into_any_element()
    }

    #[cfg(test)]
    pub(crate) fn tab_count(&self) -> usize {
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
        let editor = self.render_editor_pane(view, cx);
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
            .size_full()
            .min_w_0()
            .bg(cx.theme().tokens.background.background)
            .child(
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
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(AgentWorkbenchEvent::Closed)),
                            ),
                    ),
            )
            .child(if narrow && show_explorer {
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
            })
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

    use super::{AgentWorkbench, TabLoadState, shortest_unique_suffix};

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
        AgentWorkspaceChange {
            call_id: "call-1".to_owned(),
            path: path.to_owned(),
            kind: magenta_core::WorkspaceChangeKind::Modify,
            content: content.to_owned(),
            diff: String::new(),
            state: magenta_core::WorkspaceChangeState::Committed,
            error: None,
        }
    }

    fn setup(
        cx: &mut TestAppContext,
        projects: Arc<TestProjects>,
    ) -> gpui_kit::WindowHandle<AgentWorkbench> {
        cx.update(gpui_kit::init);
        cx.open_window(size(px(900.), px(640.)), move |window, cx| {
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
