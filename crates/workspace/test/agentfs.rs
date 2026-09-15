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
