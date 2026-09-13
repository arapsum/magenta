use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use gpui_kit::{TestAppContext, px, size};
use magenta_application::ProjectCatalog;
use magenta_core::{
    AgentWorkspaceChange, Project, ProjectStore, RepositoryAccess, RepositoryCommit,
    RepositoryDiff, RepositoryDiffArea, RepositoryFuture, RepositoryStatus, StorageFuture,
    Timestamp, WorkspaceBrowser, WorkspaceDocument, WorkspaceEntry, WorkspaceFuture,
};

use super::{
    AgentWorkbench, NextWorkbenchHunk, PreviousWorkbenchHunk, TOGGLE_VIEW_MODE_KEY, TabLoadState,
    WorkbenchViewMode, parse_hunk_lines, shortest_unique_suffix,
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

#[derive(Default)]
struct TestRepository;

impl RepositoryAccess for TestRepository {
    fn status(&self, _: PathBuf) -> RepositoryFuture<RepositoryStatus> {
        Box::pin(async {
            Ok(RepositoryStatus {
                branch: Some("main".to_owned()),
                detached: false,
                unborn: false,
                changes: Vec::new(),
            })
        })
    }

    fn diff(
        &self,
        _: PathBuf,
        path: String,
        area: RepositoryDiffArea,
    ) -> RepositoryFuture<RepositoryDiff> {
        Box::pin(async move {
            Ok(RepositoryDiff {
                path,
                area,
                unified_diff: String::new(),
                binary: false,
            })
        })
    }

    fn stage(&self, _: PathBuf, _: Vec<String>) -> RepositoryFuture<()> {
        Box::pin(async { Ok(()) })
    }

    fn unstage(&self, _: PathBuf, _: Vec<String>) -> RepositoryFuture<()> {
        Box::pin(async { Ok(()) })
    }

    fn commit(&self, _: PathBuf, summary: String) -> RepositoryFuture<RepositoryCommit> {
        Box::pin(async move {
            Ok(RepositoryCommit {
                oid: "0123456".to_owned(),
                summary,
            })
        })
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
        AgentWorkbench::new(
            ProjectCatalog::new(projects.clone(), projects),
            Arc::new(TestRepository),
            window,
            cx,
        )
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
