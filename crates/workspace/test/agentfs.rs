use magenta_core::{WorkspaceAccess, WorkspaceOperation, WorkspaceSessionAccess};

use super::AgentFsWorkspace;

#[test]
fn staged_changes_only_reach_the_host_after_final_apply() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("lib.rs"), "fn old() {}\n").unwrap();
        let workspace = AgentFsWorkspace::new(directory.path().join("sessions")).unwrap();

        workspace
            .start_session(root.clone(), "run-1".into())
            .await
            .unwrap();

        let preview = workspace
            .prepare(
                root.clone(),
                WorkspaceOperation::ApplyPatch {
                    path: "lib.rs".into(),
                    unified_diff: "@@ -1,1 +1,1 @@\n-fn old() {}\n+fn new() {}\n".into(),
                },
                false,
            )
            .await
            .unwrap();

        workspace
            .commit(root.clone(), preview.mutation.unwrap())
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join("lib.rs")).unwrap(),
            "fn old() {}\n"
        );

        let staged = workspace
            .prepare(
                root.clone(),
                WorkspaceOperation::ReadFile {
                    path: "lib.rs".into(),
                    start_line: None,
                    line_count: None,
                },
                false,
            )
            .await
            .unwrap();

        assert_eq!(staged.output, "fn new() {}");
        assert_eq!(
            workspace
                .apply_session(root.clone(), "run-1".into())
                .await
                .unwrap(),
            vec!["lib.rs"]
        );

        assert_eq!(
            std::fs::read_to_string(root.join("lib.rs")).unwrap(),
            "fn new() {}\n"
        );
    });
}

#[test]
fn discarded_overlay_never_changes_the_host() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let workspace = AgentFsWorkspace::new(directory.path().join("sessions")).unwrap();

        workspace
            .start_session(root.clone(), "run-2".into())
            .await
            .unwrap();

        let preview = workspace
            .prepare(
                root.clone(),
                WorkspaceOperation::CreateFile {
                    path: "new.rs".into(),
                    content: "fn new() {}\n".into(),
                },
                false,
            )
            .await
            .unwrap();

        workspace
            .commit(root.clone(), preview.mutation.unwrap())
            .await
            .unwrap();
        workspace
            .discard_session(root.clone(), "run-2".into())
            .await
            .unwrap();
        assert!(!root.join("new.rs").exists());
    });
}

#[test]
fn empty_review_session_does_not_block_the_next_run() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let workspace = AgentFsWorkspace::new(directory.path().join("sessions")).unwrap();

        workspace
            .start_session(root.clone(), "run-1".into())
            .await
            .unwrap();
        assert!(
            !workspace
                .session_has_changes(root.clone(), "run-1".into())
                .await
                .unwrap()
        );
        assert!(
            workspace
                .ensure_session_available(root.clone())
                .await
                .is_err()
        );
        workspace
            .discard_session(root.clone(), "run-1".into())
            .await
            .unwrap();
        workspace
            .ensure_session_available(root.clone())
            .await
            .unwrap();
        workspace
            .start_session(root.clone(), "run-2".into())
            .await
            .unwrap();
        assert!(
            !workspace
                .session_has_changes(root.clone(), "run-2".into())
                .await
                .unwrap()
        );
    });
}

#[test]
fn a_staged_directory_requires_review_and_applies_as_a_directory() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let workspace = AgentFsWorkspace::new(directory.path().join("sessions")).unwrap();
        workspace
            .start_session(root.clone(), "run-1".into())
            .await
            .unwrap();

        let preview = workspace
            .prepare(
                root.clone(),
                WorkspaceOperation::CreateDirectory {
                    path: "HelloExpress/src".into(),
                },
                false,
            )
            .await
            .unwrap();
        workspace
            .commit(root.clone(), preview.mutation.unwrap())
            .await
            .unwrap();
        assert!(!root.join("HelloExpress").exists());
        assert!(
            workspace
                .session_has_changes(root.clone(), "run-1".into())
                .await
                .unwrap()
        );
        assert!(
            workspace
                .ensure_session_available(root.clone())
                .await
                .is_err()
        );

        let changes = workspace
            .apply_session(root.clone(), "run-1".into())
            .await
            .unwrap();
        assert!(changes.contains(&"HelloExpress/src".to_owned()));
        assert!(root.join("HelloExpress/src").is_dir());
        workspace
            .ensure_session_available(root.clone())
            .await
            .unwrap();
        workspace.start_session(root, "run-2".into()).await.unwrap();
    });
}

#[test]
fn a_staged_project_directory_and_file_apply_together() {
    smol::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let workspace = AgentFsWorkspace::new(directory.path().join("sessions")).unwrap();
        workspace
            .start_session(root.clone(), "scaffold".into())
            .await
            .unwrap();

        for operation in [
            WorkspaceOperation::CreateDirectory {
                path: "HelloExpress/src".into(),
            },
            WorkspaceOperation::CreateFile {
                path: "HelloExpress/src/app.js".into(),
                content: "module.exports = {};\n".into(),
            },
        ] {
            let preview = workspace
                .prepare(root.clone(), operation, false)
                .await
                .unwrap();
            workspace
                .commit(root.clone(), preview.mutation.unwrap())
                .await
                .unwrap();
        }

        assert!(!root.join("HelloExpress").exists());
        workspace
            .apply_session(root.clone(), "scaffold".into())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("HelloExpress/src/app.js")).unwrap(),
            "module.exports = {};\n"
        );
    });
}
