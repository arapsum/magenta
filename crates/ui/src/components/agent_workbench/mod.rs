//! Read-only project tree and code preview for agent workspace changes.

use std::collections::HashMap;

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState, TabSize},
    list::ListItem,
    resizable::{h_resizable, resizable_panel},
    tree::{TreeEvent, TreeItem, TreeState, tree},
    v_flex,
};
use magenta_application::ProjectCatalog;
use magenta_core::{
    AgentWorkspaceChange, Project, WorkspaceChangeState, WorkspaceDocument, WorkspaceEntry,
    WorkspaceEntryKind,
};

#[derive(Clone, Debug)]
pub enum AgentWorkbenchEvent {
    Closed,
}

pub struct AgentWorkbench {
    catalog: ProjectCatalog,
    project: Option<Project>,
    tree_state: Entity<TreeState>,
    editor_state: Entity<EditorState>,
    directories: HashMap<String, Vec<WorkspaceEntry>>,
    expanded: std::collections::HashSet<String>,
    selected_path: Option<String>,
    document: Option<WorkspaceDocument>,
    status: Option<String>,
    load_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<AgentWorkbenchEvent> for AgentWorkbench {}

impl AgentWorkbench {
    pub fn new(catalog: ProjectCatalog, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let tree_state = cx.new(|cx| TreeState::new(cx));
        let editor_state = cx.new(|cx| {
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
        editor_state.update(cx, |editor, cx| editor.set_readonly(true, cx));
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
            editor_state,
            directories: HashMap::new(),
            expanded: std::collections::HashSet::new(),
            selected_path: None,
            document: None,
            status: None,
            load_task: None,
            _subscriptions: vec![tree_subscription],
        }
    }

    pub fn set_project(
        &mut self,
        project: Option<Project>,
        _window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.load_task.take();
        self.project = project;
        self.directories.clear();
        self.expanded.clear();
        self.selected_path = None;
        self.document = None;
        self.status = None;
        self.tree_state
            .update(cx, |state, cx| state.set_items(Vec::<TreeItem>::new(), cx));
        if self.project.is_some() {
            self.load_directory(String::new(), cx);
        }
        cx.notify();
    }

    pub fn show_change(
        &mut self,
        change: AgentWorkspaceChange,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.selected_path = Some(change.path.clone());
        let language = language_for_change(&change);
        self.document = Some(WorkspaceDocument {
            path: change.path,
            content: change.content,
            language,
        });
        self.status = Some(match change.state {
            WorkspaceChangeState::Proposed => "Awaiting approval".to_owned(),
            WorkspaceChangeState::Committed => "Committed".to_owned(),
            WorkspaceChangeState::Rejected => "Rejected".to_owned(),
            WorkspaceChangeState::Failed => {
                change.error.unwrap_or_else(|| "Write failed".to_owned())
            }
        });
        self.apply_document(window, cx);
        cx.notify();
    }

    pub fn refresh(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        self.load_task.take();
        self.directories.clear();
        self.expanded.clear();
        self.load_directory(String::new(), cx);
        if let Some(path) = self.selected_path.clone() {
            self.open_entry(path, window, cx);
        }
        cx.notify();
    }

    fn load_directory(&mut self, path: String, cx: &Context<'_, Self>) {
        if self.directories.contains_key(&path) || self.load_task.is_some() {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        let catalog = self.catalog.clone();
        self.load_task = Some(cx.spawn(async move |view, cx| {
            let result = catalog.entries(project.root, path.clone()).await;
            _ = view.update(cx, |workbench, cx| {
                workbench.load_task = None;
                match result {
                    Ok(entries) => {
                        workbench.directories.insert(path.clone(), entries);
                        workbench.rebuild_tree(cx);
                    }
                    Err(error) => workbench.status = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
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

    fn open_entry(&mut self, path: String, window: &Window, cx: &Context<'_, Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let catalog = self.catalog.clone();
        self.selected_path = Some(path.clone());
        self.status = Some("Loading file…".to_owned());
        cx.spawn_in(window, async move |view, window| {
            let result = catalog.document(project.root, path).await;
            _ = view.update_in(window, |workbench, window, cx| {
                match result {
                    Ok(document) => {
                        workbench.document = Some(document);
                        workbench.status = None;
                        workbench.apply_document(window, cx);
                    }
                    Err(error) => workbench.status = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    #[allow(clippy::needless_pass_by_ref_mut)]
    fn apply_document(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(document) = &self.document else {
            return;
        };
        self.editor_state.update(cx, |editor, cx| {
            editor.set_highlighter(document.language.clone(), cx);
            editor.set_value(document.content.clone(), window, cx);
        });
    }

    fn render_tree(&self, view: Entity<Self>, _cx: &App) -> AnyElement {
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
}

impl Render for AgentWorkbench {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let view = cx.entity();
        let title = self.selected_path.as_deref().map_or_else(
            || {
                self.project.as_ref().map_or_else(
                    || "Workspace files".to_owned(),
                    |project| format!("{} · Workspace files", project.name),
                )
            },
            |path| {
                self.project.as_ref().map_or_else(
                    || path.to_owned(),
                    |project| format!("{} / {path}", project.name),
                )
            },
        );
        let tree = self.render_tree(view, cx);
        let editor = Editor::new(&self.editor_state)
            .readonly(true)
            .bordered(false)
            .h(gpui::relative(1.))
            .font_family(cx.theme().mono_font_family.clone())
            .text_size(cx.theme().mono_font_size);
        v_flex()
            .size_full()
            .min_w_0()
            .bg(cx.theme().tokens.background.background)
            .child(
                h_flex()
                    .flex_none()
                    .h(px(36.))
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(Icon::empty().path("icons/code.svg").xsmall())
                    .child(div().flex_1().text_size(px(12.)).child(title))
                    .when_some(self.status.clone(), |this, status| {
                        this.child(
                            div()
                                .text_size(px(11.))
                                .text_color(cx.theme().muted_foreground)
                                .child(status),
                        )
                    })
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
            .child(
                h_resizable("agent-workbench")
                    .child(
                        resizable_panel()
                            .size(px(220.))
                            .size_range(px(160.)..px(360.))
                            .child(v_flex().size_full().p(px(4.)).child(tree)),
                    )
                    .child(resizable_panel().child(v_flex().size_full().min_w_0().child(editor))),
            )
    }
}

fn language_for_change(change: &AgentWorkspaceChange) -> String {
    let extension = std::path::Path::new(&change.path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    match extension {
        "rs" => "rust",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        "cmake" => "cmake",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" => "markdown",
        "js" => "javascript",
        "ts" => "typescript",
        "tsx" => "tsx",
        "py" => "python",
        _ => "text",
    }
    .to_owned()
}
