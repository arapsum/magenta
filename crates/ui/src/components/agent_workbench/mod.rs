//! Read-only, tabbed project tree and code preview for agent workspace changes.

mod content;
mod navigation;
mod state;
#[cfg(test)]
#[path = "../../../test/components/agent_workbench/mod.rs"]
mod tests;
mod toolbar;
mod tree;

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
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _, px, relative,
};
use magenta_application::ProjectCatalog;
use magenta_core::{
    AgentWorkspaceChange, Project, WorkspaceChangeKind, WorkspaceChangeState, WorkspaceDocument,
    WorkspaceEntry, WorkspaceEntryKind, WorkspaceError,
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
    Failed(FileLoadFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileLoadFailure {
    Missing,
    Permission,
    Encoding,
    Oversized,
    Other,
}

impl FileLoadFailure {
    const fn title(self) -> &'static str {
        match self {
            Self::Missing => "File is no longer available",
            Self::Permission => "File access was denied",
            Self::Encoding => "File encoding is not supported",
            Self::Oversized => "File is too large to preview",
            Self::Other => "File could not be opened",
        }
    }

    const fn message(self) -> &'static str {
        match self {
            Self::Missing => "The file may have been moved or deleted. Retry or close this tab.",
            Self::Permission => "Magenta could not read this file with the current permissions.",
            Self::Encoding => "Only bounded UTF-8 text files can be previewed in the workbench.",
            Self::Oversized => "The workbench only previews files up to 1 MiB.",
            Self::Other => "Magenta could not read this file. Retry or close this tab.",
        }
    }
}

fn classify_file_error(error: &WorkspaceError) -> FileLoadFailure {
    let source = error.source.to_string().to_ascii_lowercase();
    if source.contains("not found") || source.contains("regular file") {
        FileLoadFailure::Missing
    } else if source.contains("permission") || source.contains("access denied") {
        FileLoadFailure::Permission
    } else if source.contains("utf-8") {
        FileLoadFailure::Encoding
    } else if source.contains("one mib") || source.contains("too large") {
        FileLoadFailure::Oversized
    } else {
        FileLoadFailure::Other
    }
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

#[cfg(test)]
impl AgentWorkbench {
    pub(crate) const fn tab_count(&self) -> usize {
        self.tabs.len()
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
